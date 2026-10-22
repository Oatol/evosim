//! 世界环境：网格 + 生物集合 + 环境规则 + 两阶段主循环。
//! 与 JS 版 `src/core/World.js` 对应。
//!
//! 借用策略：**纯计算**在 `organism.rs`，**写世界的动作**在本文件。
//! 原因是 Rust 不允许"拿着 `&mut 自己` 再去借 `&mut 世界`"，
//! 所以 act / reproduce / die / harm 都写成了 `&mut self` 的方法，按索引访问个体。

use serde::{Deserialize, Serialize};

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::anatomy::{Anatomy, BodyCell, Flags};
use crate::cell_type::{
    self, ct, Behavior, EnterFx, StandFx, DIR_VECTORS, TERRAIN_MASK, TICKS_TERRAIN,
    TICKING_TERRAIN_TYPES,
};
use crate::config::AttrField;
use crate::config::Config;
use crate::fields::Fields;
use crate::grid::Grid;
use crate::light::Light;

/// 遮挡能见度：生产者正上方最多数几行。
///
/// 8 行 ≈ 两个 4 格身体的高度。再深就够不着了——真正厚的墙早就把内部饿死了，
/// 数更远只是白花时间。
const SHADE_DEPTH: i32 = 8;
use crate::mutation::mutate_organism;
use crate::organism::Organism;
use crate::rng::Rng;
use crate::species::SpeciesRegistry;

const NEIGHBORS_4: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];

/// 一套 swap-remove 索引（食物 / 腐殖质都用这个套路）
#[derive(Default, Clone, Serialize, Deserialize)]
pub struct TerrainSet {
    pub cells: Vec<usize>,
    pub pos: Vec<i32>,
}

/// 能量账目快照 —— 见 [`World::energy_audit`]。
#[derive(Clone, Copy, Debug, Default)]
pub struct EnergyAudit {
    /// 所有存活个体体内的能量总和
    pub organisms: f64,
    /// 地上所有食物格里剩下的能量总和
    pub food: f64,
    /// 热池：离开生物圈的能量（腐败、代谢热、压碎、塞不下的溢出）
    pub heat: f64,
    /// 丢弃：超出容量的光合产出 + 捕食的消化损耗
    pub wasted: f64,
    /// 累计注入（光合 + 腐殖质 + 手工放置）
    pub injected: f64,
    /// **卵里存着的能量**（批次 27）：既不在个体里、也不在食物里，
    /// 但它确实还在生物圈内——不单独记一笔，审计会凭空少一块。
    pub eggs: f64,
}

impl EnergyAudit {
    /// 现在世界上一共有多少能量（含已经离开生物圈的那部分）。
    pub fn stored(&self) -> f64 {
        self.organisms + self.food + self.eggs + self.heat + self.wasted
    }

    /// 守恒残差：`stored - injected`。**应该恒等于 0**。
    pub fn drift(&self) -> f64 {
        self.stored() - self.injected
    }
}

/// 一枚还没孵化的卵（批次 27）。
///
/// 存的是**完整的后代个体**，不是蓝图：这样 RNG 消耗发生在产卵那一刻
/// （和普通繁殖完全一致），孵化只是"把它放进世界"。
#[derive(Clone, Serialize, Deserialize)]
pub struct Egg {
    /// 到这一 tick 孵化
    pub hatch_at: u64,
    /// 母体 id（只用于诊断 / 谱系；母体可能已经死了）
    pub parent: u32,
    /// 还没出生的那个个体（`col` / `row` 已经指向卵所在格）
    pub child: Organism,
}

pub struct World {
    pub config: Config,
    pub rng: Rng,
    pub grid: Grid,
    pub tick: u64,

    pub organisms: Vec<Organism>,
    pub id_index: HashMap<u32, usize>,
    pub next_organism_id: u32,

    /// 食物索引：food_cells 存扁平索引，food_pos 存反向位置（O(1) 增删）
    pub food_cells: Vec<usize>,
    pub food_pos: Vec<i32>,

    /// 带 tick 行为的地形索引（目前只有腐殖质）
    pub terrain: HashMap<u8, TerrainSet>,

    pub species: SpeciesRegistry,

    /// 每格环境场（温度 / 辐射）。
    ///
    /// **不参与 `step()`**：主程序按墙钟 60 Hz 调 `update_fields()`，
    /// 与 tick 解耦——这样 1× 和 50× 下岩浆烤热周围的速度一致，
    /// 也不会因为倍速把 CPU 吃光。也因此它不影响黄金快照。
    pub fields: Fields,

    /// 光场（0–255）。生产者细胞靠它吃饭，见 `light.rs`。
    ///
    /// 和 `fields` 一样在 `update_fields()` 里刷，但性质不同：光场是**地形的纯函数**
    /// （生物不挡光），所以刷新只是把地形重新投射一遍，不涉及时间演化。
    /// 光合按 tick 采样 `light.level[i]`，所以刷新的频率只要不低于地形变化即可。
    pub light: Light,

    /// 光合产出因超出容量而被丢弃的累计能量（f64）。
    ///
    /// 现在只是统计量；批次 10 做守恒审计时，它要和「热池」一起进能量账。
    pub energy_wasted: f64,

    /// 从生物圈流失掉的能量累计（腐败、溢出丢弃）。
    ///
    /// 批次 8 只记账不参与模拟；批次 10 做守恒审计时，
    /// `生物可用总能量 + heat_pool` 才应该等于「注入总量」。
    pub heat_pool: f64,

    // ── 繁殖受阻统计（诊断用：不参与模拟、不进存档、不影响任何判定）──
    //
    // 存在的理由：`reproduce_organism` **先扣能量再检查空间**，所以"想繁殖
    // 但没地方"的个体是白付一次全款。这个惩罚对大体型更重（要整块连通空地），
    // 是"体型涨不上去"的头号嫌疑。分项计数才能分清到底卡在哪一步，
    // 不然只能靠猜。
    /// 进入 `reproduce_organism` 的次数（= 能量攒够了的次数）
    pub repro_tries: u64,
    /// 成功产出后代的次数
    pub repro_ok: u64,
    /// 个体总数达到上限
    pub repro_blocked_cap: u64,
    /// `is_clear` 失败：要整块空地，没找到
    pub repro_blocked_space: u64,
    /// `is_straight_path` 失败：位置空着但中间被挡
    pub repro_blocked_path: u64,
    /// 变异后的蓝图放不下（母体能塞进去，但后代身体长歪了）
    pub repro_blocked_mutated: u64,
    /// 体型小于 `min_breed_cells`，没资格繁殖
    pub repro_blocked_too_small: u64,
    /// 出芽模式下芽位已满（母体已经挂着 `bud_slots` 个未成熟的芽）
    pub repro_blocked_buds: u64,

    /// **卵**（批次 27）：格子下标 → 还没孵化的后代。
    ///
    /// 卵是**占一格**的中间态：它挡住移动和繁殖，可以被嘴吃掉。
    /// 这是"捕食压力第一次出现在繁殖环节"——吃一个卵等于消灭一个未来的个体，
    /// 而且不用打赢母体。
    ///
    /// 存完整的 `Organism` 而不是只存蓝图：这样 `Organism::new` 的 RNG 消耗
    /// 发生在**产卵那一刻**（和普通繁殖一致），孵化时不碰 RNG。
    pub eggs: HashMap<usize, Egg>,

    /// 本 tick 这个攻击者**已经咬过**的目标 id（「整口咬合」用）。
    ///
    /// 每个个体行动前清空，所以它天然是"per 攻击者、per tick"的。
    /// 不进存档：它只活在一次 `act_organism` 之内。
    killer_hits: HashSet<u32>,

    /// **累计注入生物圈的能量**（批次 10 守恒审计的左半边）。
    ///
    /// 这个世界的能量只有三个入口：
    /// 1. 光合（`Behavior::Producer`）—— 唯一的生物入口；
    /// 2. 腐殖质冒食物（`TickFx::Fertilize`）；
    /// 3. 玩家手工放置的食物（画笔）。
    ///
    /// 三者都走 `set_cell_at(FOOD)`，所以计数点放在那里，一处就够。
    /// 唯一的例外是**搬运**：`set_food_at` 把别处的能量（尸体）写进一格食物，
    /// 那不是新注入，要在里面把账退掉——见该函数的注释。
    ///
    /// 有了它，[`World::energy_audit`] 才能给出「存量 − 注入」这个恒为 0 的残差。
    pub energy_injected: f64,

    /// **移动者死亡登记**（批次 38）：`(年龄和, 个数)`。用来算移动者的平均寿命。
    ///
    /// 为什么要单独记：**寿命是一阶量，种群是二阶量**。批次 37 证明 3 种子的种群
    /// 能差 ±30%，而 n 个个体的平均寿命噪声小得多——用它才量得出「策略好坏」。
    pub mover_deaths: (u64, u64),
    /// 非移动者的死亡登记（同上）。
    pub still_deaths: (u64, u64),

    /// **地力**（批次 35）：每格 0~255。光合细胞吃它，空格按 `soil_recover` 回。
    ///
    /// 只在 `soil_deplete > 0` 时有意义；关掉时这张表一直是 255，
    /// 不参与任何计算（连恢复循环都不跑）。
    pub soil: Vec<u8>,

    /// 光场需要重算吗。
    ///
    /// 光场是「地形 + 光源 + 天空光」的纯函数，这三样都变得远比 60 Hz 慢，
    /// 所以没必要每帧算。多源 BFS 在 debug 下要 3–4 ms，每帧跑一次会把
    /// `field_update_is_cheap` 直接顶穿。
    ///
    /// 置位的地方：挡光地形变了（`set_cell_at`）、批量填充、光源增删、改尺寸。
    /// 天空光的变化不靠这个标记——`update_fields` 里直接比对 `light.sky`。
    light_dirty: bool,

    /// 阶段 B 的随机结算顺序（复用，避免每 tick 分配）
    order: Vec<usize>,
}

impl World {
    pub fn new(config: Config, rng: Rng) -> Self {
        let grid = Grid::new(config.world.cols, config.world.rows, config.world.wrap_edges);
        let len = grid.len;
        let mut w = World {
            grid,
            tick: 0,
            organisms: Vec::new(),
            id_index: HashMap::new(),
            next_organism_id: 1,
            food_cells: Vec::new(),
            food_pos: Vec::new(),
            terrain: HashMap::new(),
            species: SpeciesRegistry::default(),
            fields: Fields::new(len),
            soil: vec![255; len],
            mover_deaths: (0, 0),
            still_deaths: (0, 0),
            light: Light::new(config.world.cols, config.world.rows),
            energy_wasted: 0.0,
            heat_pool: 0.0,
            repro_tries: 0,
            repro_ok: 0,
            repro_blocked_cap: 0,
            repro_blocked_space: 0,
            repro_blocked_path: 0,
            repro_blocked_mutated: 0,
            repro_blocked_too_small: 0,
            repro_blocked_buds: 0,
            eggs: HashMap::new(),
            killer_hits: HashSet::new(),
            energy_injected: 0.0,
            light_dirty: true,
            order: Vec::new(),
            config,
            rng,
        };
        w.init_food_index();
        w.init_terrain_index();
        // 云影的噪声种子跟世界种子走：不同种子的世界"云长在不一样的地方"，
        // 但同一个种子的世界永远长一样（地形画笔触发 rebuild 时云不能跳）。
        w.light.mask_seed = w.rng.seed();
        // 建世界时地形是空的，但 `light` 的初值全 0（全黑）。这里先算一次，
        // 否则要等到第一次 `update_fields()` 才亮起来——中间那几帧光合会白算。
        w.refresh_light();
        w
    }

    // ────────────────────────────── 食物索引 ──────────────────────────────

    pub fn init_food_index(&mut self) {
        let n = self.grid.len;
        self.food_pos = vec![-1; n];
        self.food_cells.clear();
        for i in 0..n {
            if self.grid.types[i] == ct::FOOD {
                self.food_pos[i] = self.food_cells.len() as i32;
                self.food_cells.push(i);
            }
        }
    }

    fn add_food(&mut self, index: usize) {
        if self.food_pos[index] >= 0 {
            return;
        }
        self.food_pos[index] = self.food_cells.len() as i32;
        self.food_cells.push(index);
    }

    fn remove_food(&mut self, index: usize) {
        let p = self.food_pos[index];
        if p < 0 {
            return;
        }
        let p = p as usize;
        let last = self.food_cells.pop().unwrap();
        if p < self.food_cells.len() {
            self.food_cells[p] = last;
            self.food_pos[last] = p as i32;
        }
        self.food_pos[index] = -1;
    }

    // ────────────────────────────── 地形索引 ──────────────────────────────

    pub fn init_terrain_index(&mut self) {
        let n = self.grid.len;
        self.terrain.clear();
        for &t in TICKING_TERRAIN_TYPES.iter() {
            self.terrain.insert(
                t,
                TerrainSet {
                    cells: Vec::new(),
                    pos: vec![-1; n],
                },
            );
        }
        for i in 0..n {
            let f = self.grid.floor[i];
            if TICKS_TERRAIN[f as usize] {
                self.add_terrain_cell(i, f);
            }
        }
    }

    fn add_terrain_cell(&mut self, index: usize, ty: u8) {
        if let Some(set) = self.terrain.get_mut(&ty) {
            if set.pos[index] >= 0 {
                return;
            }
            set.pos[index] = set.cells.len() as i32;
            set.cells.push(index);
        }
    }

    fn remove_terrain_cell(&mut self, index: usize, ty: u8) {
        if let Some(set) = self.terrain.get_mut(&ty) {
            let p = set.pos[index];
            if p < 0 {
                return;
            }
            let p = p as usize;
            let last = set.cells.pop().unwrap();
            if p < set.cells.len() {
                set.cells[p] = last;
                set.pos[last] = p as i32;
            }
            set.pos[index] = -1;
        }
    }

    fn retag_terrain(&mut self, index: usize, old_ty: u8, new_ty: u8) {
        if TICKS_TERRAIN[old_ty as usize] {
            self.remove_terrain_cell(index, old_ty);
        }
        if TICKS_TERRAIN[new_ty as usize] {
            self.add_terrain_cell(index, new_ty);
        }
    }

    // ────────────────────────────── 格子读写 ──────────────────────────────

    /// 唯一的写入口：同时维护地板层、食物索引、地形索引。
    pub fn set_cell_at(&mut self, index: usize, ty: u8, owner: i32) {
        if index >= self.grid.len {
            return;
        }
        if self.grid.types[index] == ty && self.grid.owners[index] == owner {
            return;
        }

        let was_food = self.grid.types[index] == ct::FOOD;
        self.grid.types[index] = ty;
        self.grid.owners[index] = owner;

        if TERRAIN_MASK[ty as usize] {
            if self.grid.floor[index] != ty {
                let old = self.grid.floor[index];
                self.grid.floor[index] = ty;
                self.retag_terrain(index, old, ty);
                // 只有「挡光 / 不挡光」的转换才影响光场（岩浆换成沼泽不影响）
                if crate::light::blocks_light(old) != crate::light::blocks_light(ty) {
                    self.light_dirty = true;
                }
            }
        } else if ty == ct::EMPTY && owner < 0 && self.grid.floor[index] != ct::EMPTY {
            let old = self.grid.floor[index];
            self.grid.floor[index] = ct::EMPTY;
            self.retag_terrain(index, old, ct::EMPTY);
            if crate::light::blocks_light(old) {
                self.light_dirty = true;
            }
        }

        if ty == ct::FOOD {
            if !was_food {
                self.add_food(index);
                // 新食物默认装满一份（`food_energy_scale`）。
                // 尸体/腐殖质这种"要指定能量"的路径走 `set_food_at` 覆盖它。
                self.grid.food[index] = self.config.food_energy_gain();
                // 这是能量**进入**世界的一条路（画笔 / 腐殖质），记账。
                // `set_food_at` 会在后面把这笔退掉——那是搬运，不是注入。
                self.energy_injected += self.grid.food[index];
            }
        } else if was_food {
            self.remove_food(index);
            // 食物格不再是食物了 —— 剩下的能量不能留在数组里
            // （否则以后再变成食物会"复活"）。
            //
            // **但也不能凭空消失**：不管是身体压碎（`owner >= 0`）、画笔抹掉、
            // 还是世界生成时清场，这些能量都离开了生物圈，记进热池。
            //
            // 唯一"合法拿走"这条能量的路径是嘴——它在调 `reveal_floor` 之前
            // **会先把 `grid.food[index]` 清零**，所以走到这里时读到的是 0。
            // 那条顺序不能反：反了的话同一口饭会被同时算进个体和热池。
            self.heat_pool += self.grid.food[index];
            self.grid.food[index] = 0.0;
        }
    }

    pub fn set_cell(&mut self, col: i32, row: i32, ty: u8, owner: i32) -> Option<usize> {
        let index = self.grid.index(col, row)?;
        self.set_cell_at(index, ty, owner);
        Some(index)
    }

    /// 把一格变成食物，并指定它携带多少能量。
    ///
    /// 尸体和额外投放走这条路；普通画笔/腐殖质走 `set_cell_at` 的默认值。
    ///
    /// **这不是注入，是搬运**：能量是从个体身上（尸体）搬过来的，
    /// 所以必须把 `set_cell_at` 刚记的那笔"新长出一份食物"退掉。
    /// 不退的话每死一只个体就往 `energy_injected` 里多灌一份默认食物，
    /// 守恒审计会显示"凭空多出能量"。
    pub fn set_food_at(&mut self, index: usize, energy: f64) {
        if index >= self.grid.len {
            return;
        }
        // 原本就是食物的话，那一格的旧能量不能凭空消失，得跟新的加在一起
        let was_food = self.grid.types[index] == ct::FOOD;
        let carried = if was_food { self.grid.food[index] } else { 0.0 };
        self.set_cell_at(index, ct::FOOD, -1);
        if !was_food {
            self.energy_injected -= self.config.food_energy_gain();
        }
        if self.grid.types[index] == ct::FOOD {
            self.grid.food[index] = carried + energy.max(0.0);
        }
    }

    /// 把地板重新铺回顶层（生物离开 / 食物被吃掉时用）
    pub fn reveal_floor(&mut self, index: usize) {
        if index >= self.grid.len {
            return;
        }
        let ty = self.grid.floor[index];
        self.set_cell_at(index, ty, -1);
    }

    pub fn floor_at(&self, col: i32, row: i32) -> u8 {
        match self.grid.index(col, row) {
            Some(i) => self.grid.floor[i],
            None => ct::EMPTY,
        }
    }

    pub fn type_at(&self, col: i32, row: i32) -> u8 {
        self.grid.type_at(col, row)
    }

    pub fn fill_grid(&mut self, ty: u8, keep_terrain: bool) {
        self.grid.fill(ty, keep_terrain);
        self.init_food_index();
        self.init_terrain_index();
        // 整张地形都换了，光场必重算
        self.light_dirty = true;
    }

    // ────────────────────────────── 生物集合 ──────────────────────────────

    pub fn add_organism(&mut self, mut org: Organism) -> u32 {
        let id = self.next_organism_id;
        self.next_organism_id += 1;
        org.id = id;
        if org.species_fingerprint.is_none() {
            org.species_fingerprint = Some(Arc::from(org.anatomy.fingerprint().as_str()));
        }
        let idx = self.organisms.len();
        self.organisms.push(org);
        self.id_index.insert(id, idx);
        self.write_grid(idx);

        let fp = self.organisms[idx].species_fingerprint.clone().unwrap();
        let parent_fp = self.organisms[idx].parent_fingerprint.clone();
        let anatomy_desc_fp = fp.clone();
        let _ = anatomy_desc_fp;
        let tick = self.tick;
        // 需要 anatomy 的描述与细胞数：先算好再登记
        let desc_anatomy = self.organisms[idx].anatomy.clone();
        self.species
            .observe(&fp, parent_fp.as_ref(), &desc_anatomy, tick);
        id
    }

    /// 直接收编一个已构造好的个体，**不登记物种**（读档用）
    pub fn adopt_organism(&mut self, mut org: Organism) -> u32 {
        let id = self.next_organism_id;
        self.next_organism_id += 1;
        org.id = id;
        if org.species_fingerprint.is_none() {
            org.species_fingerprint = Some(Arc::from(org.anatomy.fingerprint().as_str()));
        }
        let idx = self.organisms.len();
        self.organisms.push(org);
        self.id_index.insert(id, idx);
        self.write_grid(idx);
        id
    }

    pub fn organism_index(&self, id: u32) -> Option<usize> {
        self.id_index.get(&id).copied()
    }

    pub fn organism(&self, id: u32) -> Option<&Organism> {
        self.organism_index(id).map(|i| &self.organisms[i])
    }

    pub fn can_add_organism(&self) -> bool {
        let max = self.config.life.max_organisms;
        max < 0 || (self.organisms.len() as i64) < max
    }

    pub fn clear_organisms(&mut self) {
        for i in 0..self.organisms.len() {
            self.kill_organism(i);
        }
        self.organisms.clear();
        self.id_index.clear();
    }

    /// 压缩掉已死亡的个体。
    ///
    /// **必须稳定**（保持原有相对顺序）：阶段 B 的结算顺序是"对数组做一次洗牌"，
    /// 数组顺序一变，洗牌出来的访问序列就跟着变，黄金快照立刻对不上。
    fn compact_organisms(&mut self) {
        self.organisms.retain(|o| o.alive);
        self.id_index.clear();
        for (i, org) in self.organisms.iter().enumerate() {
            self.id_index.insert(org.id, i);
        }
    }

    // ────────────────────────────── 网格写入 ──────────────────────────────

    pub fn write_grid(&mut self, i: usize) {
        let n = self.organisms[i].anatomy.cells.len();
        for k in 0..n {
            let cell = self.organisms[i].anatomy.cells[k];
            let col = self.organisms[i].real_col(&cell);
            let row = self.organisms[i].real_row(&cell);
            if let Some(index) = self.grid.index(col, row) {
                let id = self.organisms[i].id as i32;
                self.set_cell_at(index, cell.type_id, id);
            }
        }
    }

    fn clear_grid(&mut self, i: usize) {
        let n = self.organisms[i].anatomy.cells.len();
        for k in 0..n {
            let cell = self.organisms[i].anatomy.cells[k];
            let col = self.organisms[i].real_col(&cell);
            let row = self.organisms[i].real_row(&cell);
            if let Some(index) = self.grid.index(col, row) {
                self.reveal_floor(index);
            }
        }
    }

    // ────────────────────────────── 环境规则 ──────────────────────────────

    /// 食物腐败：**能量一点点流失**，不是整格突然消失。
    ///
    /// 每格每 tick 掉 `max(e × rel, floor)`：
    ///
    /// * `rel`（相对项）让**大块尸体慢慢烂**——这是"守尸 / 抢食"能成立的前提；
    /// * `floor`（绝对项）让**小块食物干脆消失**，避免 0.001 点能量赖着不走、占着格子。
    ///
    /// 流失的能量进 `heat_pool`。旧实现是"每 tick 按比例随机删格子"，
    /// 断崖式：同一块肉要么在、要么突然没了，看不出腐败的过程。
    pub fn apply_food_decay(&mut self) {
        let rel = self.config.world.food_decay_rate / 100.0;
        let floor = self.config.world.food_decay_floor;
        if rel <= 0.0 && floor <= 0.0 {
            return;
        }
        // 遍历时可能删格（swap-remove 会打乱数组），所以用 while 手动控制下标：
        // 删掉的格子会让当前位换成原末尾元素，必须**原地重看**，不能 i += 1。
        let mut i = 0;
        while i < self.food_cells.len() {
            let index = self.food_cells[i];
            let e = self.grid.food[index];
            let decay = (e * rel).max(floor).min(e);
            let left = e - decay;
            self.heat_pool += decay;
            // **先写回剩下的量再决定要不要删格**：`reveal_floor` 会把
            // "消失的食物能量"记进热池，如果这里还留着 `e`，那笔 `decay`
            // 就会被记第二次——账面上凭空多出能量（批次 10 的对账测试抓到过）。
            self.grid.food[index] = left;
            if left <= 1e-6 {
                self.reveal_floor(index); // 会从 food_cells 里 swap-remove
            } else {
                i += 1;
            }
        }
    }

    pub fn apply_terrain_tick(&mut self) {
        for &ty in TICKING_TERRAIN_TYPES.iter() {
            let len = self.terrain.get(&ty).map(|s| s.cells.len()).unwrap_or(0);
            for k in 0..len {
                let index = match self.terrain.get(&ty) {
                    Some(set) => match set.cells.get(k) {
                        Some(&v) => v,
                        None => break,
                    },
                    None => break,
                };
                self.run_terrain_tick(ty, index);
            }
        }
    }

    fn run_terrain_tick(&mut self, ty: u8, index: usize) {
        let fx = match cell_type::def(ty).and_then(|d| d.tick) {
            Some(f) => f,
            None => return,
        };
        match fx {
            cell_type::TickFx::Fertilize => {
                let prob = self.config.terrain.humus_food_prob;
                if prob <= 0.0 {
                    return;
                }
                if !self.rng.chance(prob) {
                    return;
                }
                let (col, row) = self.grid.coords(index);
                // 5 个候选：0 = 腐殖质**自己那一格**，1..=4 = 四邻域。
                // 用户要求"也在它自己上生成"——腐殖质自己也是块地，长在自己身上才对。
                let k = self.rng.int(5);
                let first = if k == 0 {
                    Some(index)
                } else {
                    let n = NEIGHBORS_4[(k - 1) as usize];
                    self.grid.index(col + n.0, row + n.1)
                };
                // 选中的格子被占（有生物 / 已有食物）就退回自己那格再试一次，
                // 否则腐殖质被围住时永远不产食。
                //
                // 注意：地形格自己的顶层 `types` 存的就是它自己（HUMUS），
                // 所以"能放食物"的判据是「空格 **或** 裸地形」，不能只判 EMPTY。
                for cand in [first, Some(index)] {
                    if let Some(t) = cand {
                        let ty_here = self.grid.types[t];
                        if ty_here == ct::EMPTY || ty_here == ty {
                            self.set_cell_at(t, ct::FOOD, -1);
                            break;
                        }
                    }
                }
            }
        }
    }

    pub fn apply_food_drop(&mut self) {
        let prob = self.config.world.food_drop_prob;
        if prob <= 0.0 {
            return;
        }
        let area = self.grid.len as f64;
        let count = ((area * prob) / 50000.0).floor().max(1.0) as usize;
        for _ in 0..count {
            if !self.rng.chance(prob) {
                continue;
            }
            let col = self.rng.int(self.grid.cols) as i32;
            let row = self.rng.int(self.grid.rows) as i32;
            if let Some(index) = self.grid.index(col, row) {
                if self.grid.types[index] == ct::EMPTY {
                    self.set_cell_at(index, ct::FOOD, -1);
                }
            }
        }
    }

    // ────────────────────────────── 主循环 ──────────────────────────────

    pub fn step(&mut self) {
        self.tick += 1;

        // ── 昼夜（批次 22）──
        //
        // 必须在感知之前：`think()` 里的视觉输入读的是当前光场，
        // 光和行动要落在同一时刻上，否则"看见亮的却走进黑"会差半拍。
        self.update_daylight();

        // ── 阶段 A：全体感知与决策（只读世界）──
        //
        // 这一阶段**只读世界**、只写个体自己的 plan/缓冲，所以可以安全并行。
        // 关键：`think()` 里不消耗 RNG（已确认），因此并行不会改变随机序列，
        // 黄金快照依然逐位一致。
        self.think_all();

        // ── 阶段 B：随机顺序结算（写世界）──
        self.order.clear();
        self.order.extend(0..self.organisms.len());
        let mut order = std::mem::take(&mut self.order);
        self.rng.shuffle(&mut order);

        for &i in order.iter() {
            if i < self.organisms.len() && self.organisms[i].alive {
                self.act_organism(i);
            }
        }
        self.order = order;

        self.compact_organisms();

        // ── 孵化（批次 27）──
        //
        // 放在个体结算之后：这一 tick 里刚产下的卵不该立刻孵化
        // （`hatch_at` 至少是 `tick + egg_hatch_ticks`，天然保证了这点）。
        self.hatch_eggs();

        // ── 环境更新 ──
        self.apply_terrain_tick();
        self.apply_food_decay();
        self.apply_soil_recover();
        self.apply_food_drop();

        // ── 物种统计 ──
        //
        // 1.x 是在统计采样时调 `species.update()` 的；2.0 漏了这一步，
        // 结果所有物种的 population 恒为 0 —— 物种面板是空的、存活/峰值/灭绝全是 0。
        // 这里按 statsInterval 重算一次（不消耗 RNG、不写网格，所以不影响黄金快照）。
        if self.config.stats.interval > 0 && self.tick % self.config.stats.interval == 0 {
            let World {
                organisms,
                species,
                tick,
                config,
                ..
            } = self;
            let fps: Vec<std::sync::Arc<str>> = organisms
                .iter()
                .filter_map(|o| o.species_fingerprint.clone())
                .collect();
            species.update(fps.iter(), *tick);
            if species.list.len() > crate::species::MAX_SPECIES {
                species.prune(crate::species::MAX_SPECIES, config.stats.min_discard);
            }
        }
    }

    /// 推进昼夜相位，把日照系数灌进光场（批次 22）。
    ///
    /// 曲线是**余弦**而不是方波：方波会让"日落"那一 tick 光合收入从满额直接掉到
    /// `night_floor`，储能来不及补；余弦让个体有整个下午去填电池，
    /// 也让"什么时候繁殖"变成一个真问题（黄昏出生的芽必须熬过整夜）。
    ///
    /// `phase = 0` 是正午（`daylight = 1`），`phase = 0.5` 是午夜（`= night_floor`）。
    /// 起始相位取正午，因为世界生成时（`tick = 0`）是亮的——和"始祖生在光下"一致。
    pub fn update_daylight(&mut self) {
        let len = self.config.world.day_length;
        if len == 0 {
            // 关掉昼夜：确保回到满照（这条路径每 tick 都会走，但 `set_daylight`
            // 在值没变时直接返回，所以零开销）
            self.light.set_daylight(1.0);
            return;
        }
        let phase = (self.tick % len as u64) as f64 / len as f64;
        let floor = self.config.world.night_floor.clamp(0.0, 1.0);
        let d = floor + (1.0 - floor) * (0.5 + 0.5 * (std::f64::consts::TAU * phase).cos());
        self.light.set_daylight(d as f32);
    }

    /// 孵化到期的卵（批次 27）。
    ///
    /// 三条边界：
    /// - **格子必须还是卵**：可能已经被嘴吃掉、或者被地形覆盖了。
    /// - **位置必须还放得下**：卵挡路时别人进不来，但被吃掉的卵会让出位置，
    ///   那时可能已经有别人站在上面了。
    /// - **顺序按格子下标排序**：`HashMap` 的迭代顺序不稳定，
    ///   不排序的话同一次孵化在不同运行里会换顺序 → 不可复现。
    fn hatch_eggs(&mut self) {
        if self.eggs.is_empty() {
            return;
        }
        let now = self.tick;
        let mut ready: Vec<usize> = self
            .eggs
            .iter()
            .filter(|(_, e)| e.hatch_at <= now)
            .map(|(&i, _)| i)
            .collect();
        ready.sort_unstable();
        for idx in ready {
            let Some(egg) = self.eggs.remove(&idx) else {
                continue;
            };
            if self.grid.types.get(idx).copied() != Some(ct::EGG) {
                continue; // 卵已经被吃 / 被覆盖
            }
            // **用 reveal_floor 而不是 set_cell_at(EMPTY)**：卵可能产在一格
            // 地形上（腐殖质），直接设 EMPTY 会把那层地形一起抹掉。
            self.reveal_floor(idx);
            if !self.can_add_organism() {
                self.repro_blocked_cap += 1;
                continue;
            }
            let col = (idx % self.grid.cols) as i32;
            let row = (idx / self.grid.cols) as i32;
            let rot = egg.child.rotation;
            if !egg.child.can_place_at(
                &self.grid,
                &self.config,
                col,
                row,
                rot,
                &egg.child.anatomy,
            ) {
                // 位置被占了：这一枚就没了（卵里的能量进热池）
                self.heat_pool += egg.child.energy;
                continue;
            }
            self.add_organism(egg.child);
        }
    }

    /// 阶段 A：全体感知 + 决策。
    ///
    /// 拆字段借用后用 rayon 并行——这一阶段占满载开销的七成左右，
    /// 且完全不碰 RNG，所以并行既安全又不破坏可复现性。
    pub fn think_all(&mut self) {
        use rayon::prelude::*;
        let World {
            grid,
            config,
            organisms,
            soil,
            ..
        } = self;
        let soil_on = config.life.soil_deplete > 0;
        organisms.par_iter_mut().filter(|o| o.alive).for_each(|o| {
            // 批次 36：把"脚下地力"喂给大脑。关掉地力时恒为 1.0（逐位等于旧行为）。
            let s = if soil_on {
                grid.index(o.col, o.row)
                    .map(|i| soil[i] as f64 / 255.0)
                    .unwrap_or(1.0)
            } else {
                1.0
            };
            o.think(grid, config, s)
        });
    }

    /// 单个体版（测试与串行对照用）
    pub fn think_organism(&mut self, i: usize) {
        let World {
            grid,
            config,
            organisms,
            ..
        } = self;
        let s = if config.life.soil_deplete > 0 {
            grid.index(organisms[i].col, organisms[i].row)
                .map(|k| self.soil[k] as f64 / 255.0)
                .unwrap_or(1.0)
        } else {
            1.0
        };
        organisms[i].think(grid, config, s);
    }

    /// 阶段 B：结算
    ///
    /// 外层只负责**记录检视用的两个观测量**，真正的结算在 `act_organism_inner`。
    /// 这样做是因为 inner 里有 5 处提前 `return`（老死 / 饿死 / 被打死），
    /// 在每个 return 前手写一遍收支会漏；包一层就只写一次。
    pub fn act_organism(&mut self, i: usize) {
        if i >= self.organisms.len() {
            return;
        }
        let before = self.organisms[i].energy;
        self.organisms[i].last_move_ok = false;
        // 「整口咬合」的记账是 per 攻击者 / per tick 的，每次行动前清空。
        self.killer_hits.clear();
        self.act_organism_inner(i);
        if let Some(org) = self.organisms.get_mut(i) {
            org.energy_gained_last = org.energy - before;
        }
    }

    fn act_organism_inner(&mut self, i: usize) {
        // 年龄
        self.organisms[i].age += 1;
        // 繁殖失败冷却（批次 26）：每 tick 递减
        if self.organisms[i].repro_cooldown > 0 {
            self.organisms[i].repro_cooldown -= 1;
        }
        let lifespan = self.organisms[i].lifespan(&self.config);
        if self.organisms[i].age as f64 > lifespan {
            self.kill_organism(i);
            return;
        }

        self.organisms[i].in_radiation = false;

        // 代谢
        //
        // **这是「活着的价格」**。`metabolism_scale = 0` 时这里整段跳过——
        // 细胞不吃饭也不会饿死，于是"养一个生产者细胞"是免费的，
        // 光合就成了严格占优策略（批次 9.5 的实测：异养占比恒为 0）。
        //
        // 付出去的能量记进 `heat_pool`：代谢热也是"离开生物圈"，
        // 不能凭空消失，否则守恒审计永远对不上账。
        // 注意记的是**实际付出**（`min(upkeep, energy)`）而不是名义开销：
        // 身上只剩 0.01 的个体付不出 0.05，多记的 0.04 是凭空的。
        let upkeep = self.organisms[i].upkeep_cost(&self.config);
        if upkeep > 0.0 {
            let limit = self.starve_limit(i);
            let mut starved = false;
            let paid: f64;
            {
                let org = &mut self.organisms[i];
                paid = upkeep.min(org.energy.max(0.0));
                org.energy -= upkeep;
                if org.energy <= 0.0 {
                    org.energy = 0.0;
                    org.starve_ticks += 1;
                    if org.starve_ticks >= limit {
                        starved = true;
                    }
                } else {
                    org.starve_ticks = 0;
                }
            }
            self.heat_pool += paid;
            if starved {
                self.kill_organism(i);
                return;
            }
        }

        if self.organisms[i].anatomy.has(Flags::MOVER) {
            self.act_move(i);
        }
        if !self.organisms[i].alive {
            return;
        }

        // 驻留效果（移动之后）
        self.apply_terrain_stand(i);
        if !self.organisms[i].alive {
            return;
        }

        // 细胞功能
        let n = self.organisms[i].anatomy.cells.len();
        for k in 0..n {
            let cell = self.organisms[i].anatomy.cells[k];
            self.run_cell_function(i, cell);
            if !self.organisms[i].alive {
                return;
            }
        }

        // 成长（批次 12）：芽把攒下的能量拿去长身体，长满才算成年。
        //
        // 放在细胞行为**之后**：光合挣的钱先到账，才谈得上拿去长。
        // 放在繁殖判定**之前**：成年判定要在这一 tick 就生效。
        if self.config.life.growth_enabled && self.organisms[i].growth_plan.is_some() {
            self.grow_organism(i);
            if !self.organisms[i].alive {
                return;
            }
        }

        // 环境温度伤害（批次 11）
        //
        // 放在细胞行为之后、繁殖之前：光合挣的钱先到账，热区把它烧掉一部分，
        // 剩下的才够不够繁殖由后面的判定决定。顺序反过来会让"刚挣到就被烧光"
        // 的个体连繁殖判定都走不到，等于把热区变成纯粹的坟场。
        self.apply_heat_damage(i);
        if !self.organisms[i].alive {
            return;
        }

        // 繁殖
        //
        // 出芽模式下门槛是**固定的** `bud_start_cost`，不是 `food_needed = 细胞数`。
        // 这正是"大个体产后代更划算"的体现：10 格母体的收入是 1 格的 10 倍，
        // 但每产一个芽的启动成本一样。
        let at = if self.budding() && self.bud_slots(i) > 0 {
            self.config.life.bud_start_cost
        } else {
            self.organisms[i].reproduce_at(&self.config)
        };
        // **未成年不能繁殖**：芽攒再多能量也得先长成成体。
        // 这一条是整个成长机制的意义所在——它让「1 格」不再等于「完整个体」。
        if self.organisms[i].energy >= at
            && self.organisms[i].growth_plan.is_none()
            && self.organisms[i].repro_cooldown == 0
        {
            self.reproduce_organism(i);
        }
    }

    /// 饥饿致死的宽限期（tick）。批次 34：没有生产者的个体扛饿更久。
    ///
    /// 这是**生理差异**，不是凭空发钱：捕食是"饱一顿饿一顿"，光合是"每天都有底薪"。
    /// 批次 31 把纯异养推到 6.9% 后饱和，说明瓶颈是"找不到猎物的那些 tick 会饿死"，
    /// 而不是"咬得不狠"——所以要动的就是这个宽限期。
    pub fn starve_limit(&self, i: usize) -> i32 {
        let base = self.config.life.starvation_ticks;
        if self.organisms[i].anatomy.has(Flags::PRODUCER) {
            return base;
        }
        let m = self.config.life.predator_starve_mult;
        if m > 1.0 {
            ((base as f64) * m).round() as i32
        } else {
            base
        }
    }

    /// 地力恢复（批次 35）。关掉时直接返回，连循环都不跑。
    pub fn apply_soil_recover(&mut self) {
        if self.config.life.soil_deplete == 0 || self.config.life.soil_recover == 0 {
            return;
        }
        let r = self.config.life.soil_recover;
        for v in self.soil.iter_mut() {
            *v = v.saturating_add(r);
        }
    }

    fn act_move(&mut self, i: usize) {
        if self.organisms[i].turn_cooldown > 0 {
            self.organisms[i].turn_cooldown -= 1;
        }

        if self.organisms[i].plan_turn {
            if self.attempt_rotate(i) {
                self.organisms[i].turn_cooldown = self.config.life.turn_cooldown;
                return;
            }
            let d = self.organisms[i].plan_direction;
            self.organisms[i].direction = d & 3;
            return;
        }

        let d = self.organisms[i].plan_direction;
        self.organisms[i].direction = d & 3;
        if self.attempt_move(i) {
            self.organisms[i].last_move_ok = true;
            return;
        }

        if self.attempt_rotate(i) {
            self.organisms[i].turn_cooldown = self.config.life.turn_cooldown;
        } else {
            let d = self.organisms[i].plan_direction;
            self.organisms[i].direction = d & 3;
        }
    }

    fn attempt_move(&mut self, i: usize) -> bool {
        let (nc, nr) = self.organisms[i].step_target();
        let rot = self.organisms[i].rotation;
        if !self.organisms[i].is_clear(&self.grid, &self.config, nc, nr, rot) {
            return false;
        }

        // 目的地的地形要在 write_grid 之前读（write_grid 会盖住顶层）
        let dest_type = match self.grid.index(nc, nr) {
            Some(index) => self.grid.floor[index],
            None => ct::EMPTY,
        };

        self.clear_grid(i);
        self.organisms[i].col = nc;
        self.organisms[i].row = nr;
        self.write_grid(i);
        {
            let grid = &self.grid;
            self.organisms[i].normalize_position(grid);
        }

        if dest_type != ct::EMPTY {
            self.apply_terrain_enter(i, dest_type);
        }
        true
    }

    fn attempt_rotate(&mut self, i: usize) -> bool {
        let new_rot = self.rng.int(4) as u8;
        let (c, r) = (self.organisms[i].col, self.organisms[i].row);
        if !self.organisms[i].is_clear(&self.grid, &self.config, c, r, new_rot) {
            return false;
        }
        self.clear_grid(i);
        self.organisms[i].rotation = new_rot;
        self.organisms[i].direction = self.rng.int(4) as u8;
        self.write_grid(i);
        true
    }

    // ────────────────────────────── 地形效果 ──────────────────────────────

    fn apply_terrain_enter(&mut self, i: usize, ty: u8) {
        let fx = match cell_type::def(ty).and_then(|d| d.enter) {
            Some(f) => f,
            None => return,
        };
        match fx {
            EnterFx::Burn => {
                let dmg = self.config.terrain.lava_enter_damage;
                if dmg > 0.0 {
                    let id = self.organisms[i].id;
                    self.harm_organism(id, dmg, None);
                }
            }
            EnterFx::Bog => {
                let cost = self.config.terrain.swamp_move_cost;
                if cost > 0.0 {
                    // 踩沼泽耗掉的能量也是"离开生物圈"，要进热池——
                    // 否则能量账每走一步沼泽就漏一笔。
                    let paid: f64;
                    {
                        let org = &mut self.organisms[i];
                        paid = cost.min(org.energy.max(0.0));
                        org.energy -= cost;
                        if org.energy < 0.0 {
                            org.energy = 0.0;
                        }
                    }
                    self.heat_pool += paid;
                }
            }
        }
    }

    fn apply_terrain_stand(&mut self, i: usize) {
        let index = match self.grid.index(self.organisms[i].col, self.organisms[i].row) {
            Some(x) => x,
            None => return,
        };
        let ty = self.grid.floor[index];
        if ty == ct::EMPTY {
            return;
        }
        let fx = match cell_type::def(ty).and_then(|d| d.stand) {
            Some(f) => f,
            None => return,
        };
        match fx {
            StandFx::Burn => {
                let dmg = self.config.terrain.lava_stand_damage;
                if dmg > 0.0 {
                    let id = self.organisms[i].id;
                    self.harm_organism(id, dmg, None);
                }
            }
            StandFx::Irradiate => {
                let dmg = self.config.terrain.rad_damage;
                if dmg > 0.0 {
                    let id = self.organisms[i].id;
                    self.harm_organism(id, dmg, None);
                }
                if self.organisms[i].alive {
                    self.organisms[i].in_radiation = true;
                }
            }
        }
    }

    /// 温度视图的显示上限（℃）。
///
/// 固定值，**不随环境温度移动**：如果范围跟着 `ambient + span` 一起动，
/// 调环境温度时底色会跟着一起漂，反而看不出变化。
/// 120℃ 让「常温」落在色表的深紫段、「岩浆」顶到最亮，
/// 中间的变化肉眼可分。
const TEMP_VIEW_MAX_C: f64 = 120.0;

/// 遮挡：生产者正上方同列 `SHADE_DEPTH` 行内，有几个身体格（**谁的不重要**）。
    ///
    /// 返回天空光的透过率 `(1-k)^n`，`n = 0` 时是 1.0。
    ///
    /// **自己的细胞也算。** 第一版只数"别人"（怕惩罚大体型），实测这是个漏洞：
    /// 一个身体可以在同一列里叠好几个生产者，底下那些"被自己压着"却照样满额产光，
    /// 于是演化直接去叠高——平均体型从 3.0 涨到 4.3，致密格反而没降。
    /// 光不知道格子归谁，数全部才是物理上对的，而且它顺带给了**布局**一个价格：
    /// 生产者要长在最上面，埋在下面的只值 `(1-k)^n`。这是"让体型有意义"想要的东西。
    ///
    /// 为什么只算天空光：灯是点光源，方向朝四面八方，用"上方"去近似它是错的。
    /// 这一条的代价是灯照的世界没有遮挡——可接受，因为灯通常就是玩家手动摆的稀缺资源。
    fn shade_factor(&self, col: i32, row: i32) -> f64 {
        let k = self.config.life.shade_strength;
        if !(k > 0.0) {
            return 1.0;
        }
        let mut n = 0i32;
        for d in 1..=SHADE_DEPTH {
            // **垂直方向不能环绕**：`grid.index` 在 `wrap = true` 时会把 `row - d`
            // 折到世界底部，于是最上面几行的生产者会被底部的生物"挡住"。
            // 这和光场 BFS 里那条坑是同一个（高度方向代表"地下"，上下不相连）。
            let rr = row - d;
            if rr < 0 {
                break;
            }
            let Some(j) = self.grid.index(col, rr) else {
                continue;
            };
            let owner = self.grid.owners[j];
            if owner < 0 {
                continue;
            }
            n += 1;
        }
        if n == 0 {
            1.0
        } else {
            (1.0 - k.clamp(0.0, 1.0)).powi(n)
        }
    }

    // ────────────────────────────── 细胞功能 ──────────────────────────────

    // ────────────────────────────── 能量账 ──────────────────────────────

    /// 拍一张能量账目的快照。
    ///
    /// 守恒律只对**生命层**严格成立：世界的能量只有三个入口
    /// （光合、腐殖质、手工放置），其余全是内部搬运。所以
    ///
    /// ```text
    /// 注入总量 == 生物体内 + 地上食物 + 热池 + 丢弃
    /// ```
    ///
    /// 恒成立（浮点误差以内）。`drift()` 就是这条等式的残差——
    /// 它不为 0 就说明有某条路径在**凭空造能量或吞能量**，
    /// 这正是这个函数存在的意义：以后每加一条改动能量的路径，
    /// 跑一下审计就知道账有没有漏。
    ///
    /// 注意 `heat_pool` 和 `energy_wasted` 都算**存量**而不是"损失"：
    /// 能量离开生物圈不等于消失，账面上它还在这两个池子里。
    pub fn energy_audit(&self) -> EnergyAudit {
        let organisms: f64 = self
            .organisms
            .iter()
            .filter(|o| o.alive)
            .map(|o| o.energy)
            .sum();
        let food: f64 = self.food_cells.iter().map(|&k| self.grid.food[k]).sum();
        let eggs: f64 = self.eggs.values().map(|e| e.child.energy).sum();
        EnergyAudit {
            organisms,
            food,
            heat: self.heat_pool,
            wasted: self.energy_wasted,
            injected: self.energy_injected,
            eggs,
        }
    }

    /// 这个个体能存多少能量（见 [`Organism::energy_capacity`]）。
    fn energy_capacity(&self, i: usize) -> f64 {
        self.organisms[i].energy_capacity(&self.config)
    }

    /// 给个体加能量，**装不下的部分记进 `energy_wasted`**。
    ///
    /// **所有"加能量"的路径都必须走这里**：光合、吃食物、咬活体。
    ///
    /// 为什么不能直接 `energy += x`：容量检查只在光合那边做，而它判断的是
    /// `room = cap - energy`。一旦别的路径把 energy 顶到 cap 以上，
    /// 下一次光合就会算出 `room = 0` 然后 `energy = cap` —— 把个体**往下 clamp**，
    /// 那一截既没进 `wasted` 也没进 `heat`，等于凭空蒸发。
    ///
    /// 这个 bug 在"繁殖失败不扣钱"之后才暴露：旧行为下个体能量经常被繁殖
    /// 清到 0，几乎撞不到容量上限；新行为下能量长期贴着上限，
    /// 每一次越界 clamp 都在偷偷吃掉能量。
    /// 返回**实际到账**多少（装不下被丢弃的那部分不算）——
    /// 收入来源统计要的是真到手的，不是名义上的。
    fn add_energy(&mut self, i: usize, amount: f64) -> f64 {
        if !(amount > 0.0) {
            return 0.0;
        }
        let cap = self.energy_capacity(i);
        // 历史遗留的超容量（旧存档 / 早先漏了 clamp 的路径）：先把它记掉，
        // 否则下面 `energy = cap` 会静默吞掉这一截。
        if self.organisms[i].energy > cap {
            self.energy_wasted += self.organisms[i].energy - cap;
            self.organisms[i].energy = cap;
        }
        let room = (cap - self.organisms[i].energy).max(0.0);
        if amount <= room {
            self.organisms[i].energy += amount;
            amount
        } else {
            self.organisms[i].energy = cap;
            self.energy_wasted += amount - room;
            room
        }
    }

    /// 芽长一格。
    ///
    /// 从 `growth_plan` 里挑一个「还没长、且与现有身体四邻相邻、目标格可放」
    /// 的细胞，花 `growth_cost` 能量把它长出来。
    ///
    /// **为什么必须"相邻"**：蓝图里可能同时有左右两个分支，如果允许凭空长，
    /// 芽会分裂成两块互不相连的身体——那是另一种生物了。
    /// 逐个相邻长出来，形状才会像母体。
    ///
    /// 长满之后把 `growth_plan` 置空，个体才算「成年」、才能繁殖。
    fn grow_organism(&mut self, i: usize) {
        let Some(plan) = self.organisms[i].growth_plan.clone() else {
            return;
        };
        // 年龄只在"还是芽"的时候涨。它是「最短成长期」的计时器：
        // 1 格目标的芽出生时就已经"长满"了，没有它就会当 tick 成年、立刻繁殖，
        // 于是「小」变成免费策略，体型在**任何**环境里都涨不上去（见批次 13 实测）。
        self.organisms[i].grow_age = self.organisms[i].grow_age.saturating_add(1);
        // 出芽：先向母体取一份能量（内部转移，不进热池）。
        if let Some(pid) = self.organisms[i].parent_id {
            self.feed_from_parent(i, pid);
        }
        let min_age = self.config.life.growth_min_ticks;
        let old_enough = self.organisms[i].grow_age >= min_age;
        let cost = self.config.life.growth_cost.max(0.0);
        // 能量不够就长不动 —— 但**不能直接 return**：要走下面的停滞计数，
        // 否则"饿着的芽"永远不会被判定为"该提前成熟"。
        let can_pay = self.organisms[i].energy >= cost;
        let mut grew = false;
        if can_pay {
            for cell in plan.cells.iter() {
            // 已经长出来了？
            if self.organisms[i].anatomy.cell_at(cell.dx, cell.dy).is_some() {
                continue;
            }
            // 必须和现有身体相邻
            let adjacent = self.organisms[i]
                .anatomy
                .cells
                .iter()
                .any(|c| (c.dx - cell.dx).abs() + (c.dy - cell.dy).abs() == 1);
            if !adjacent {
                continue;
            }
            let (c, r) = (
                self.organisms[i].real_col(cell),
                self.organisms[i].real_row(cell),
            );
            let Some(idx) = self.grid.index(c, r) else {
                continue;
            };
            if !self.cell_is_free_for_birth(idx) {
                continue;
            }
            self.organisms[i]
                .anatomy
                .add_cell(cell.type_id, cell.dx, cell.dy, cell.dir);
            self.write_grid(i);
            grew = true;
            break;
        }
        if !grew {
            // ── 兜底：目标格全被邻居占了 ──
            //
            // 这是实测踩出来的：`growth_plan` 是**母体的形状**，而芽生在母体旁边，
            // 那片区域的格子很可能已经被别的个体占住。严格按 plan 长的话，
            // 芽会永远卡在 1 格、永远不能繁殖 —— 实测 1002/1003 两个种子直接灭绝。
            //
            // 兜底策略：长到**任意相邻空位**，细胞类型从 plan 里随机取。
            // 形状会因此偏离母体，但体型（= 目标细胞数）仍然受限，
            // 而且"能长大"比"长得像"重要得多。
            let ty = plan.cells[self.rng.int(plan.cells.len())].type_id;
            let cells = self.organisms[i].anatomy.cells.clone();
            'outer: for c in cells {
                for n in NEIGHBORS_4 {
                    let (ndx, ndy) = (c.dx + n.0, c.dy + n.1);
                    if self.organisms[i].anatomy.cell_at(ndx, ndy).is_some() {
                        continue;
                    }
                    let probe = BodyCell::new(ty, ndx, ndy, 0);
                    let (cc, rr) = (
                        self.organisms[i].real_col(&probe),
                        self.organisms[i].real_row(&probe),
                    );
                    let Some(idx) = self.grid.index(cc, rr) else {
                        continue;
                    };
                    if !self.cell_is_free_for_birth(idx) {
                        continue;
                    }
                    self.organisms[i].anatomy.add_cell(ty, ndx, ndy, 0);
                    self.write_grid(i);
                    grew = true;
                    break 'outer;
                }
            }
        }
        }
        if grew && cost > 0.0 {
            let paid: f64;
            {
                let org = &mut self.organisms[i];
                paid = cost.min(org.energy.max(0.0));
                org.energy -= cost;
                if org.energy < 0.0 {
                    org.energy = 0.0;
                }
            }
            // 生长耗能也是"离开生物圈"，进热池——否则账上会多出一块。
            self.heat_pool += paid;
        }
        // ── 停滞计数与「提前成熟」──
        //
        // 长不动 = 能量不够（食物稀缺）**或**目标格被占（拥挤）。
        // 两者在生态上是同一件事：**这个环境撑不起这么大的身体**。
        //
        // 连续卡够久就放弃长满，以当前体型成年 —— 这就是表型可塑性：
        // 同一个基因型，食物充足时长成大体型（K），稀缺时提前成熟成小体型（r）。
        if grew {
            self.organisms[i].grow_stall = 0;
        } else {
            let limit = self.config.life.growth_stall_mature;
            if limit > 0 {
                self.organisms[i].grow_stall = self.organisms[i].grow_stall.saturating_add(1);
                // **提前成熟也要过年龄门槛**：否则「最短成长期」会被
                // 「长不动就提前成熟」这条路径绕过去——那正好是最需要它的场景
                // （贫斑块里芽长不动，本来会立刻变小体型）。
                if self.organisms[i].grow_stall >= limit && old_enough {
                    self.organisms[i].growth_plan = None;
                    self.organisms[i].grow_stall = 0;
                    // 成年即脱离母体：之后不再取能量，也不再占母体的芽位。
                    self.organisms[i].parent_id = None;
                    return;
                }
            }
        }

        // 长满了**而且**够年龄 → 成年
        if self.organisms[i].anatomy.cells.len() >= plan.cells.len() && old_enough {
            self.organisms[i].growth_plan = None;
            self.organisms[i].grow_stall = 0;
            self.organisms[i].parent_id = None;
        }
    }

    /// 环境温度造成的伤害：**表面积/体积比**决定谁扛得住。
    ///
    /// ```text
    /// 局部温度 = ambient_temp + (场值/255) × temp_span      // 摄氏度
    /// excess   = (局部温度 - comfort_temp) / temp_span      // 0..1
    /// 伤害     = temp_damage × excess × 体型^temp_size_exponent
    /// ```
    ///
    /// 取个体占据格子里**最热**的一格：身体只要有一角伸进热区就得承受。
    /// 这是故意的——它让"贴着岩浆取暖"变成一件有代价的事，
    /// 而不是可以随便把一根手指伸进去白嫖。
    ///
    /// 血量 `max_health` ∝ 体积 ∝ n，伤害 ∝ n^0.75，所以**单位体重的抗性
    /// ∝ n^0.25**：大个体能在热区待更久。`temp_size_exponent = 1` 会让
    /// 伤害和血量同步增长，体型优势消失；`= 0` 则相反（大个体纯靠血厚）。
    fn apply_heat_damage(&mut self, i: usize) {
        let base = self.config.life.temp_damage;
        if !(base > 0.0) {
            return;
        }
        let ambient = self.config.world.ambient_temp;
        let span = self.config.world.temp_span.max(1.0);
        let comfort = self.config.life.comfort_temp;

        let mut hottest = 0u8;
        for cell in &self.organisms[i].anatomy.cells {
            let c = self.organisms[i].real_col(cell);
            let r = self.organisms[i].real_row(cell);
            if let Some(idx) = self.grid.index(c, r) {
                hottest = hottest.max(self.fields.temp[idx]);
            }
        }
        // 场值是"高于环境的温差"，换算成绝对温度才能和舒适温度比。
        let local = ambient + (hottest as f64 / 255.0) * span;
        if local <= comfort {
            return;
        }
        let excess = ((local - comfort) / span).clamp(0.0, 1.0);
        let n = self.organisms[i].anatomy.cells.len().max(1) as f64;
        // **线性**，不是平方。平方在"中等温度"上会把伤害压成零：
        // 实测环境 40℃（比舒适温度高 5℃）时 excess = 0.05、平方后 0.0025，
        // 伤害 0.001/tick，和 22℃ 的结果**一模一样**，看起来像 bug。
        // 线性下 excess 本身就随距离上升，"越靠近岩浆越致命"照样成立。
        let dmg = base * excess * n.powf(self.config.life.temp_size_exponent);
        if !(dmg > 0.0) {
            return;
        }
        let id = self.organisms[i].id;
        self.harm_organism(id, dmg, None);
    }

    fn run_cell_function(&mut self, i: usize, cell: BodyCell) {
        let behavior = match cell_type::def(cell.type_id).and_then(|d| d.behavior) {
            Some(b) => b,
            None => return,
        };
        match behavior {
            Behavior::Mouth => {
                let col = self.organisms[i].real_col(&cell);
                let row = self.organisms[i].real_row(&cell);
                // 摄取倍率只乘**这一格剩下的能量**——食物本身带存量之后，
                // 「一份食物值多少」不再由全局常数决定，而是看它被吃时还剩多少。
                let mult = self.config.attr(cell.type_id, AttrField::BiteMult);
                let my_id = self.organisms[i].id;
                // 「一张嘴一口」：这一格嘴在这一 tick 只能对上一个活体目标。
                //
                // 批次 20 挖出的根因：下面这个四邻域循环原本对**每个**邻居各抽一次
                // `bite_rate`，于是单嘴上限是 `4 × bite_rate`（0.30 时 1.2/tick，
                // 满照光合只有 0.12）——嘴因此成了 10 倍收益的机器，76% 的个体都靠
                // 它活着。嘴是一根管子，同一瞬间只能对上一个目标。
                //
                // 实现上分成两段：先走一遍四邻域**吃光所有够得着的食物**，顺手把
                // 活体邻居记进 `victims`；再决定咬哪一个。食物不受配额限制——它是
                // 躺在格子里的存量，一口能舔到几格由几何决定，不占"咬住不放"的配额。
                let mut victims = [0usize; 4];
                let mut nv = 0usize;
                for k in 0..4 {
                    let n = NEIGHBORS_4[k];
                    let Some(index) = self.grid.index(col + n.0, row + n.1) else {
                        continue;
                    };
                    if self.grid.types[index] == ct::FOOD {
                        let e = self.grid.food[index];
                        // **先清零再铺地板**：`set_cell_at` 会把"消失的食物能量"
                        // 记进热池，不先取走的话这一口会被算两次。
                        self.grid.food[index] = 0.0;
                        self.reveal_floor(index);
                        let gain = e * mult;
                        let got = self.add_energy(i, gain);
                        self.organisms[i].gain_food += got;
                        // 摄取倍率不是 1 的时候，吃这一口**改变**了世界里的能量：
                        // > 1 等于凭空放大（记成注入），< 1 等于消化损耗（记成丢弃）。
                        // 不记的话账上会凭空多/少一块，审计立刻红。
                        if gain > e {
                            self.energy_injected += gain - e;
                        } else {
                            self.energy_wasted += e - gain;
                        }
                        continue;
                    }
                    // ── 吃卵（批次 27）──
                    //
                    // 卵是"还没出生的个体"，但它躺在格子里、不还手——
                    // 对嘴来说这就是一块会孵化的肉。这是捕食压力第一次
                    // 出现在**繁殖环节**：吃一枚卵等于消灭一个未来的个体，
                    // 而且不用打赢母体。
                    if self.grid.types[index] == ct::EGG {
                        let e = self
                            .eggs
                            .remove(&index)
                            .map(|egg| egg.child.energy)
                            .unwrap_or(0.0);
                        // 先拿走再铺地板，理由同食物：否则会被记两次
                        self.reveal_floor(index);
                        let gain = e * mult;
                        let got = self.add_energy(i, gain);
                        self.organisms[i].gain_food += got;
                        if gain > e {
                            self.energy_injected += gain - e;
                        } else {
                            self.energy_wasted += e - gain;
                        }
                        continue;
                    }
                    // ── 咬活体（批次 9.5）──
                    //
                    // 批次 9 的诊断：杀手能杀人，但尸体按格撒在死者身上，凶手只贴着
                    // 其中一格，大部分能量它够不着——捕食的实际回报远低于当生产者，
                    // 于是杀手和移动器一起灭绝。这里把回报直接接到嘴上：抽多少立刻到账。
                    //
                    // 同一格既可能是食物又可能是活体？不可能——`grid.types` 是单层，
                    // 所以上面 `continue` 掉食物分支之后，剩下有主的一定是活体。
                    let owner = self.grid.owners[index];
                    if owner < 0 || owner == my_id as i32 {
                        continue;
                    }
                    victims[nv] = index;
                    nv += 1;
                }
                if nv == 0 {
                    return;
                }
                // 咬谁：有多个目标时随机挑一个，而不是"按四邻域顺序的第一个"。
                //
                // 顺序挑会引入**方向偏置**——`NEIGHBORS_4` 是上右下左，那嘴就永远
                // 优先咬上方的邻居，生态里会长出一种假的方向性。也刻意不挑"最肥的"：
                // 那等于给捕食者完美信息。单目标时不抽随机数，让 rng 流保持干净
                // （旧路径 `mouth_one_bite = false` 更是一颗随机数都不碰）。
                let one = self.config.life.mouth_one_bite;
                let pick = if one && nv > 1 {
                    self.rng.int(nv)
                } else {
                    0
                };
                let last = if one { pick + 1 } else { nv };
                for j in pick..last {
                    let index = victims[j];
                    let owner = self.grid.owners[index];
                    // 上一口可能把目标咬死了（尸体格换主），跳过。
                    if owner < 0 || owner == my_id as i32 {
                        continue;
                    }
                    // 体型差（批次 23）：大咬小加成、小咬大打折。
                    // 亲缘折扣（批次 24）：同种相残打折。
                    // 两个都关掉时恒为 1.0，逐位等于旧行为。
                    let adv = self.size_advantage(i, owner as u32);
                    let kin = self.kin_penalty(i, owner as u32);
                    let spec = self.predator_specialization(i);
                    let gain = self.bite_from(owner as u32, index, mult * adv * kin * spec);
                    // 同样要走容量检查：咬到嘴里的能量一旦超过容量，
                    // 直接 `+=` 会让下一次光合把个体往下 clamp，那一截就蒸发了。
                    let got = self.add_energy(i, gain);
                    self.organisms[i].gain_bite += got;
                }
            }
            Behavior::Producer => {
                if self.organisms[i].anatomy.has(Flags::MOVER)
                    && !self.config.life.movers_can_produce
                {
                    return;
                }
                // ── 光合：光强 × 全局光合速率 × 这个生产者自己的倍率 ──
                //
                // 能量**直接进体内**，不经过食物格。这是批次 7 的核心改动：
                // 以前是 `rng.chance(food_prod_prob)` 凭空在相邻空格造一份食物，
                // 造食物不消耗任何东西——那是台永动机。
                //
                // 现在光成了唯一的能量来源：产出正比于**这一格的光强**，
                // 所以「站在哪」第一次有了价值。暗处的生产者产出为 0，
                // 它是纯负担（占一格、消耗一份 upkeep），这本身就是选择压。
                let col = self.organisms[i].real_col(&cell);
                let row = self.organisms[i].real_row(&cell);
                // 光分两种来源，**遮挡只作用于天空光**：
                // 身体挡得住从头顶下来的光，挡不住从侧面照进来的灯。
                // 合成一个场就没法区分了，所以 `Light` 里存的是两层。
                let sky = self.light.sky_at(&self.grid, col, row) as f64;
                let lamp = self.light.lamp_at(&self.grid, col, row) as f64;
                let shade = self.shade_factor(col, row);
                let lit = (sky * shade + lamp) / crate::light::SUN_MAX as f64;
                if lit <= 0.0 {
                    return;
                }
                // 注意：**不要再乘 `sun_intensity`**：`lit` 已经是最终光强，
                // 天空光和点光源都在里面了（`sun_intensity` 只决定天空光注入多亮）。
                // 乘第二次的后果是「关掉天空光 → 点光源也一起熄」，实测直接把生态打死。
                let mut gain = lit
                    * self.config.life.photosynthesis_rate
                    * self.config.attr(cell.type_id, AttrField::ProdMult);
                if gain <= 0.0 {
                    return;
                }
                // ── 地力（批次 35）──
                //
                // 光合从**脚下这一格**抽地力：同一格连着吃会越来越瘦，换一块生地
                // 就吃满额。这是"移动"第一次有收益的地方——批次 33 量出来移动细胞
                // 只占 0.1%，根因就是"站着不动免费"。
                //
                // 乘在 `gain` 上而不是改 `photosynthesis_rate`：这样点光源、遮挡、
                // 光照分带那些逻辑全都不动，只是在最后乘一个 0.25~1 的系数。
                if self.config.life.soil_deplete > 0 {
                    if let Some(si) = self.grid.index(col, row) {
                        let f = self.soil[si] as f64 / 255.0;
                        let floor = self.config.life.soil_floor.clamp(0.0, 1.0);
                        gain *= floor + (1.0 - floor) * f;
                        self.soil[si] = self.soil[si].saturating_sub(self.config.life.soil_deplete);
                    }
                }
                // 容量上限：光合直充体内之后必须有，否则能量无限膨胀。
                // **注入记的是 `gain` 全额**，装不下的那部分由 `add_energy`
                // 记进 `energy_wasted`——两者相加才等于"这一格造出来的能量"。
                // 审计的等式是 `注入 = 生物 + 食物 + 热池 + 丢弃`，
                // 所以丢弃必须留在右半边。
                self.energy_injected += gain;
                let got = self.add_energy(i, gain);
                self.organisms[i].gain_photo += got;
            }
            Behavior::Killer => {
                let damage = self.config.attr(cell.type_id, AttrField::Damage);
                if damage <= 0.0 {
                    return;
                }
                // 「整口咬合」：这一口的大小 = 单格伤害 × 这个个体的杀手细胞数。
                //
                // 关键在于它**不随接触面积放大**：一只 5 杀手细胞的捕食者贴着
                // 猎物的 5 个格子，仍然只咬一口 5 点，而不是 5 口各 5 点。
                // 这样击杀时间 = 目标血量 / 咬合力，体型才买得到生存时间。
                let capped = self.config.combat.killer_bite_capped;
                let bite = if capped {
                    let n = self.organisms[i]
                        .anatomy
                        .cells
                        .iter()
                        .filter(|c| {
                            cell_type::def(c.type_id)
                                .map(|d| d.behavior == Some(Behavior::Killer))
                                .unwrap_or(false)
                        })
                        .count();
                    damage * n as f64
                } else {
                    damage
                };
                let col = self.organisms[i].real_col(&cell);
                let row = self.organisms[i].real_row(&cell);
                let my_id = self.organisms[i].id;
                for k in 0..4 {
                    let n = NEIGHBORS_4[k];
                    if let Some(index) = self.grid.index(col + n.0, row + n.1) {
                        let owner = self.grid.owners[index];
                        if owner < 0 || owner == my_id as i32 {
                            continue;
                        }
                        if self.grid.types[index] == ct::ARMOR {
                            // 装甲从"完全免疫"改成"按比例减免"。
                            //
                            // 出厂 1.0 = 100% 减免，必须走 `continue` 的**快路径**：
                            // `harm_organism` 即使 amount = 0 也会触发刺的反伤和死亡判定，
                            // 把伤害乘 0 送进去会引入旧行为里没有的副作用（黄金快照会漂）。
                            let resist = self.config.attr(ct::ARMOR, AttrField::ArmorResist);
                            if resist >= 1.0 {
                                continue;
                            }
                            // resist <= 0 或 NaN 都退回"不减免"，按普通格处理
                            if resist > 0.0 {
                                // 装甲只减免"这一口"，同样不按接触面积叠加。
                                if capped && !self.killer_hits.insert(owner as u32) {
                                    continue;
                                }
                                let eff = bite * (1.0 - resist);
                                self.harm_organism(owner as u32, eff, Some(my_id));
                                continue;
                            }
                        }
                        // 「整口咬合」：同一个目标这一 tick 只挨这一口。
                        // 记在 `killer_hits` 里的是**目标 id**，所以同一只捕食者的
                        // 第 2、3、4 个接触格都会在这里被挡掉。
                        if capped && !self.killer_hits.insert(owner as u32) {
                            continue;
                        }
                        self.harm_organism(owner as u32, bite, Some(my_id));
                    }
                }
            }
            Behavior::Regen => {
                if self.organisms[i].damage <= 0.0 {
                    return;
                }
                // 全局「再生速率」× 这个再生细胞自己的倍率（出厂 1.0 = 不变）
                let rate = self.config.life.regen_rate
                    * self.config.attr(cell.type_id, AttrField::RegenMult);
                if !(rate > 0.0) {
                    return;
                }
                let org = &mut self.organisms[i];
                org.damage -= rate;
                if org.damage < 0.0 {
                    org.damage = 0.0;
                }
            }
            Behavior::Eye => {}
        }
    }

    /// 体型差带来的咬合倍率（批次 23）。
    ///
    /// `clamp((n_捕食者 / n_猎物)^k, 0.25, 4.0)`。`k = 0`（默认）时恒为 1.0。
    ///
    /// 上下限不是随手拍的：
    /// - **上限 4.0** 防止"大者通吃"。20 格咬 1 格在 k=0.5 时是 4.47，夹到 4.0——
    ///   再高的话捕食收益会压过光合（满照 0.12，4 倍咬合 = 0.24/tick），
    ///   生态会瞬间倒向"长得越大越好"，而那正是这批改动想避免的。
    /// - **下限 0.25** 保留"蚂蚁咬大象也能啃下一口"，但只有四分之一。
    ///   归零会让小个体彻底失去捕食这条路，而小个体正是生态里的多数。
    fn size_advantage(&self, pred_i: usize, victim_id: u32) -> f64 {
        const MIN: f64 = 0.25;
        const MAX: f64 = 4.0;
        let k = self.config.life.size_advantage;
        if !(k > 0.0) {
            return 1.0;
        }
        let Some(vi) = self.organism_index(victim_id) else {
            return 1.0;
        };
        let np = self.organisms[pred_i].cell_count().max(1) as f64;
        let nv = self.organisms[vi].cell_count().max(1) as f64;
        (np / nv).powf(k).clamp(MIN, MAX)
    }

    /// 捕食专精奖励（批次 31）：身体里**没有生产者细胞**时咬得更狠。
    ///
    /// `predator_specialization = 1.0`（默认）时恒为 1.0，逐位等于旧行为。
    ///
    /// 为什么需要它：混养体（生产者 + 嘴）永远赢——它有底薪又吃红利。
    /// 想让它不赢，只有两条路：**禁止混养**（批次 30 诊断证明是灾难），
    /// 或者**给放弃底薪的个体一个补偿**。这一项是后者。
    fn predator_specialization(&self, i: usize) -> f64 {
        let k = self.config.life.predator_specialization;
        if !(k > 1.0) {
            return 1.0;
        }
        if self.organisms[i].anatomy.has(Flags::PRODUCER) {
            return 1.0;
        }
        k
    }

    /// 亲缘折扣（批次 24）：同种相残的收益倍率。
    ///
    /// 同种返回 `1 - kin_bite_penalty`，异种返回 1.0。`kin_bite_penalty = 0`（默认）
    /// 时恒为 1.0。
    ///
    /// 判据是 `species_fingerprint`（身体蓝图的逐格哈希）。**它很脆**：
    /// 变异一次指纹就变，母子可能被判成不同种。这一批先按它做，是为了量出
    /// "同种相残到底占多少"——如果惩罚开满也没有效果，说明真正的问题不是亲缘，
    /// 而是捕食本身太弱。
    fn kin_penalty(&self, pred_i: usize, victim_id: u32) -> f64 {
        let p = self.config.life.kin_bite_penalty.clamp(0.0, 1.0);
        if p <= 0.0 {
            return 1.0;
        }
        let Some(vi) = self.organism_index(victim_id) else {
            return 1.0;
        };
        // 批次 32：可选按**构成聚类**判同种。默认还是指纹（行为不变）——
        // 聚类口径比指纹粗得多，切过去会让"同种相残打折"从少数情况变成多数情况，
        // 那是行为改动，得单独扫过再决定。
        if self.config.life.kin_by_species {
            let d = self.organisms[pred_i]
                .genome()
                .composition_distance(self.organisms[vi].genome());
            let k = self.config.life.species_distance as f32;
            return if d <= k { 1.0 - p } else { 1.0 };
        }
        let a = self.organisms[pred_i].species_fingerprint.as_deref();
        let b = self.organisms[vi].species_fingerprint.as_deref();
        match (a, b) {
            (Some(x), Some(y)) if x == y => 1.0 - p,
            _ => 1.0,
        }
    }

    /// **聚类物种数**（批次 32）：按身体构成贪心聚类，返回个体数 ≥ `min_pop` 的簇数。
    ///
    /// 为什么不能用 `species.alive_count`：那个是**精确指纹**判据——变异一格就换一个
    /// 指纹，所以它数的是"结构变体的累计"，会单调上涨且不封顶（实测 48000 tick 到 67.5
    /// 还在涨）。物种数要回答的是"生态里有多少种**形态**在共存"，那必须是连续的判据。
    ///
    /// 阈值是 `species_distance`（Jaccard 构成距离，默认 0.15）。`kin_by_species`
    /// 打开时亲缘判定用同一个阈值——两处口径不一致的话，"同种相残打折"和"物种数"
    /// 会互相打架。
    ///
    /// 贪心、顺序相关、代表取第一次出现的那个：这样结果**可复现**（`organisms` 是
    /// 有序 `Vec`）。簇直径 ≤ 2 × 阈值，不会像"链式聚类"那样一路串成一坨。
    pub fn species_clusters(&self, min_pop: usize) -> usize {
        let k = self.config.life.species_distance as f32;
        let mut reps: Vec<[u16; 17]> = Vec::new();
        let mut counts: Vec<usize> = Vec::new();
        for org in &self.organisms {
            let comp = org.genome().composition();
            let mut hit = None;
            for (i, rep) in reps.iter().enumerate() {
                let (mut inter, mut union) = (0u32, 0u32);
                for j in 0..17 {
                    let (x, y) = (rep[j] as u32, comp[j] as u32);
                    inter += x.min(y);
                    union += x.max(y);
                }
                if union > 0 && 1.0 - inter as f32 / union as f32 <= k {
                    hit = Some(i);
                    break;
                }
            }
            match hit {
                Some(i) => counts[i] += 1,
                None => {
                    reps.push(comp);
                    counts.push(1);
                }
            }
        }
        counts.iter().filter(|&&c| c >= min_pop).count()
    }

    /// 从 `victim_id` 身上抽走一点能量，返回**实际到账**的量（已扣消化损耗）。
    ///
    /// `index` 是被咬的那一格——只有它可能是装甲格，所以减免按格查而不是按个体查。
    /// 这一点和杀手那边一致：一个个体身上有装甲格不等于全身免疫。
    ///
    /// 三条边界：
    /// - **被对方身上剩的能量卡住**：抽不出超过它有的。咬一口空壳 = 0，
    ///   不会变成负数，也不会凭空造出能量。
    /// - **装甲减免按 `ArmorResist` 走**，出厂 1.0 等于这一格咬不动。
    ///   注意这是"快路径"式的全免（直接返回 0），和杀手那边的 `resist >= 1.0` 对齐。
    /// - **消化损耗进 `energy_wasted`**：`1 - bite_efficiency` 那部分离开了生物圈。
    ///   批次 10 的守恒审计要用它，所以现在就得记上，不能让它悄悄消失。
    fn bite_from(&mut self, victim_id: u32, index: usize, mult: f64) -> f64 {
        let rate = self.config.life.bite_rate;
        if !(rate > 0.0) {
            return 0.0;
        }
        let mut cut = 1.0;
        if self.grid.types[index] == ct::ARMOR {
            let resist = self.config.attr(ct::ARMOR, AttrField::ArmorResist);
            if resist >= 1.0 {
                return 0.0;
            }
            if resist > 0.0 {
                cut = 1.0 - resist;
            }
        }
        let want = rate * mult * cut;
        if !(want > 0.0) {
            return 0.0;
        }
        let Some(vi) = self.organism_index(victim_id) else {
            return 0.0;
        };
        if !self.organisms[vi].alive {
            return 0.0;
        }
        let have = self.organisms[vi].energy;
        if !(have > 0.0) {
            return 0.0;
        }
        let taken = want.min(have);
        self.organisms[vi].energy = have - taken;
        let eff = self.config.life.bite_efficiency.clamp(0.0, 1.0);
        let gain = taken * eff;
        self.energy_wasted += taken - gain;
        gain
    }

    // ────────────────────────────── 伤害与死亡 ──────────────────────────────

    pub fn harm_organism(&mut self, target_id: u32, amount: f64, attacker_id: Option<u32>) {
        let ti = match self.organism_index(target_id) {
            Some(i) => i,
            None => return,
        };
        if !self.organisms[ti].alive {
            return;
        }
        self.organisms[ti].damage += amount;

        if let Some(ai) = attacker_id {
            if ai != target_id {
                let attacker_alive = self
                    .organism_index(ai)
                    .map(|k| self.organisms[k].alive)
                    .unwrap_or(false);
                if attacker_alive {
                    let back = self.organisms[ti].spike_damage(&self.config);
                    if back > 0.0 {
                        // 不带 attacker → 不再连锁反伤（递归深度恒为 1）
                        self.harm_organism(ai, back, None);
                    }
                }
            }
        }

        let insta = self.config.combat.insta_kill;
        let hp = self.organisms[ti].max_health(&self.config);
        if insta || self.organisms[ti].damage >= hp {
            self.kill_organism(ti);
        }
    }

    /// 画笔「擦生物」用：把这一格上的那只生物整个抹掉，**不留残骸**。
    ///
    /// 和 `kill_organism` 的区别只有一条：不撒食物。这是玩家在擦画布，不是捕食。
    /// 但死者的能量不能凭空消失，所以进热池（离开生物圈，账仍然对得上）。
    ///
    /// 返回是否真的擦掉了一只。
    pub fn erase_creature_at(&mut self, index: usize) -> bool {
        if index >= self.grid.len {
            return false;
        }
        let owner = self.grid.owners[index];
        if owner < 0 {
            return false;
        }
        let Some(i) = self.organism_index(owner as u32) else {
            return false;
        };
        self.heat_pool += self.organisms[i].energy.max(0.0);
        self.organisms[i].alive = false;
        self.note_death(i);
        self.organisms[i].energy = 0.0;
        let coords: Vec<(i32, i32)> = self.organisms[i]
            .anatomy
            .cells
            .iter()
            .map(|c| {
                (
                    self.organisms[i].real_col(c),
                    self.organisms[i].real_row(c),
                )
            })
            .collect();
        for (c, r) in coords {
            if let Some(idx) = self.grid.index(c, r) {
                self.set_cell_at(idx, ct::EMPTY, -1);
            }
        }
        true
    }

    /// 寿命登记（批次 38）：按「有没有运动细胞」分组累加。
    ///
    /// 死亡有两个入口（`kill_organism` 和画笔的 `erase_creature_at`），
    /// 抽成一个函数，避免以后再加第三个入口时漏掉。
    fn note_death(&mut self, i: usize) {
        let (dead_age, had_mover) = {
            let o = &self.organisms[i];
            (o.age as u64, o.anatomy.has(Flags::MOVER))
        };
        if had_mover {
            self.mover_deaths.0 += dead_age;
            self.mover_deaths.1 += 1;
        } else {
            self.still_deaths.0 += dead_age;
            self.still_deaths.1 += 1;
        }
    }

    pub fn kill_organism(&mut self, i: usize) {
        if i >= self.organisms.len() || !self.organisms[i].alive {
            return;
        }
        self.organisms[i].alive = false;
        self.note_death(i);

        // ── 残骸携带死者**体内攒下的能量** ──
        //
        // 这是「狩猎才有意义」的地基：肥的死了留肥尸体，捕食者吃到的就是它
        // 生前攒的那一份。旧实现按细胞类型给固定的 `food_value`，
        // 体内 energy 直接蒸发——攒得越多死得越亏，但亏掉的那部分凭空消失。
        //
        // 每格容量是 `food_value`（细胞属性，类型参数里可调），装不下的部分
        // 撒到周围空格，再装不下就记进热池（不算凭空消失）。
        let total = self.organisms[i].energy.max(0.0);
        let n = self.organisms[i].anatomy.cells.len();
        let per = if n > 0 { total / n as f64 } else { 0.0 };
        let mut leftover = 0.0;
        for k in 0..n {
            let cell = self.organisms[i].anatomy.cells[k];
            let col = self.organisms[i].real_col(&cell);
            let row = self.organisms[i].real_row(&cell);
            let cap = self.config.attr(cell.type_id, AttrField::FoodValue).max(0.0);
            let here = per.min(cap);
            leftover += per - here;
            if let Some(index) = self.grid.index(col, row) {
                if here > 1e-6 {
                    self.set_food_at(index, here);
                } else {
                    // 一点能量都没有的格子就别变成食物了（否则立刻被腐败清掉）
                    self.set_cell_at(index, ct::EMPTY, -1);
                }
            }
        }
        if leftover > 1e-6 {
            let (col, row) = (self.organisms[i].col, self.organisms[i].row);
            self.drop_extra_food(col, row, leftover);
        }

    }

    /// 在 (col,row) 周围的空格里撒 `energy` 这么多能量，按每格一份摊开。
    ///
    /// 尸体装不下的部分走这里。如果周围全满，剩下的进热池——
    /// 能量不会凭空消失，只是离开了生物圈。
    fn drop_extra_food(&mut self, col: i32, row: i32, energy: f64) {
        if energy <= 1e-6 {
            return;
        }
        let cap = self.config.food_energy_gain().max(1e-6);
        let count = (energy / cap).ceil().max(1.0) as usize;
        let each = energy / count as f64;
        let max_r = self.grid.cols.max(self.grid.rows) as i32;
        let mut placed = 0usize;
        let mut r = 1i32;
        while r <= max_r && placed < count {
            let mut dy = -r;
            while dy <= r && placed < count {
                let mut dx = -r;
                while dx <= r && placed < count {
                    if dx.abs().max(dy.abs()) != r {
                        dx += 1;
                        continue;
                    }
                    if let Some(index) = self.grid.index(col + dx, row + dy) {
                        if self.grid.types[index] == ct::EMPTY {
                            self.set_food_at(index, each);
                            placed += 1;
                        }
                    }
                    dx += 1;
                }
                dy += 1;
            }
            r += 1;
        }
        // 周围塞不下的部分离开生物圈
        self.heat_pool += each * (count - placed) as f64;
    }

    // ────────────────────────────── 繁殖 ──────────────────────────────

    /// 这一格能不能站一个**刚出生**的生物（不考虑形状）。
    ///
    /// 只在 `growth_enabled` 时用：芽只有 1 格，所以「能不能放」退化成一格判断。
    /// 语义对齐 `can_place_at`（用 `food_blocks_reproduction` 而不是 movement）。
    fn cell_is_free_for_birth(&self, idx: usize) -> bool {
        let ty = self.grid.types[idx];
        if ty == ct::EMPTY {
            return true;
        }
        // 有主 = 被别的生物占着
        if self.grid.owners[idx] >= 0 {
            return false;
        }
        if ty == ct::FOOD && !self.config.combat.food_blocks_reproduction {
            return true;
        }
        crate::cell_type::is_passable(ty)
    }

    /// 出芽是否生效。**依赖成长过程**：芽必须有 `growth_plan` 才谈得上"长成"，
    /// 所以两个开关都得开。
    fn budding(&self) -> bool {
        self.config.life.budding_enabled && self.config.life.growth_enabled
    }

    /// 这个个体能同时挂几个未成熟的芽。**0 = 不能出芽**。
    ///
    /// `floor(n / bud_cells_per_slot)`，**没有下限**。
    ///
    /// 第一版给了 `max(1)`（1 格也有 1 个芽位），实测直接把体型压到 1.85：
    /// 1 格身体拿到了和 10 格一样的"最低配置"，而它便宜得多。
    /// 去掉下限之后出芽变成**大体型专属**的机制——这也更符合生物学：
    /// 单细胞是分裂，不是出芽。
    fn bud_slots(&self, i: usize) -> usize {
        let per = self.config.life.bud_cells_per_slot.max(1.0);
        let n = self.organisms[i].cell_count() as f64;
        (n / per).floor() as usize
    }

    /// 这个个体当前还挂着几个未成熟的芽。
    ///
    /// 只在"母体攒够钱、准备产芽"那一下调用，所以不需要维护计数器——
    /// 每 tick 全量扫一遍是 O(个体数²)，但这里一 tick 只发生几次。
    fn live_buds(&self, id: u32) -> usize {
        self.organisms
            .iter()
            .filter(|o| o.parent_id == Some(id))
            .count()
    }

    /// 紧贴母体身体的空格里随机挑一个（出芽的位置）。
    ///
    /// 和普通繁殖的区别：不要 4~6 格外的空地、不要直线路径——
    /// **芽就长在母体身上**。这也顺带把"出生失败率 70%"那个问题绕开了：
    /// 母体自己的体表一定有空格可挂（除非被围死）。
    fn adjacent_free_cell(&mut self, i: usize) -> Option<(i32, i32)> {
        let mut cands: Vec<usize> = Vec::new();
        for cell in &self.organisms[i].anatomy.cells {
            let (c, r) = (
                self.organisms[i].real_col(cell),
                self.organisms[i].real_row(cell),
            );
            for n in NEIGHBORS_4 {
                let Some(idx) = self.grid.index(c + n.0, r + n.1) else {
                    continue;
                };
                if !self.cell_is_free_for_birth(idx) {
                    continue;
                }
                if !cands.contains(&idx) {
                    cands.push(idx);
                }
            }
        }
        if cands.is_empty() {
            return None;
        }
        let k = self.rng.int(cands.len());
        let (c, r) = self.grid.coords(cands[k]);
        Some((c, r))
    }

    /// 向母体取一份能量。**内部转移**，不进热池——账上总量不变。
    ///
    /// 取多少取三者的最小值：`bud_feed`（母体愿意给的）、芽的容量余量、
    /// 母体身上剩的。第三项让"母体快饿死时芽也挨饿"自然发生，不需要特判。
    fn feed_from_parent(&mut self, i: usize, pid: u32) {
        let Some(&pi) = self.id_index.get(&pid) else {
            return;
        };
        if pi == i || pi >= self.organisms.len() || !self.organisms[pi].alive {
            return;
        }
        let cap = self.energy_capacity(i);
        let room = (cap - self.organisms[i].energy).max(0.0);
        let give = self
            .config
            .life
            .bud_feed
            .min(room)
            .min(self.organisms[pi].energy.max(0.0));
        if give <= 0.0 {
            return;
        }
        self.organisms[pi].energy -= give;
        self.organisms[i].energy += give;
        self.organisms[i].gain_gift += give;
    }

    /// 出芽：产一个挂在母体身上的芽。
    ///
    /// 和 `reproduce_organism` 的四处关键差别（也是"大个体终于能赢"的来源）：
    ///
    /// | | 普通繁殖 | 出芽 |
    /// |---|---|---|
    /// | 落点 | 4~6 格外的空地 | **紧贴母体** |
    /// | 一次性成本 | `food_needed = 细胞数`（随体型涨） | `bud_start_cost`（**固定**） |
    /// | 同时能生几个 | 1 个 | `floor(n / bud_cells_per_slot)` 个 |
    /// | 未成熟期 | 自己攒能量 | 母体每 tick 喂 `bud_feed` |
    fn reproduce_bud(&mut self, i: usize) -> bool {
        self.repro_tries += 1;
        if self.organisms[i].cell_count() < self.config.life.min_breed_cells {
            self.repro_blocked_too_small += 1;
            return false;
        }
        if !self.can_add_organism() {
            self.repro_blocked_cap += 1;
            return false;
        }
        let id = self.organisms[i].id;
        if self.live_buds(id) >= self.bud_slots(i) {
            self.repro_blocked_buds += 1;
            return false;
        }
        let Some((nc, nr)) = self.adjacent_free_cell(i) else {
            self.repro_blocked_space += 1;
            return false;
        };

        // 固定的启动成本。**不随体型涨**——这就是超线性的全部来源。
        let cost = self.config.life.bud_start_cost.max(0.0);
        let paid: f64;
        {
            let org = &mut self.organisms[i];
            paid = cost.min(org.energy.max(0.0));
            org.energy -= cost;
            if org.energy < 0.0 {
                org.energy = 0.0;
            }
        }
        self.heat_pool += paid;

        let rotation = self.rng.int(4) as u8;
        let mut child = Organism::new(&self.config, &mut self.rng, &self.grid, nc, nr);
        child.brain.copy_from(&self.organisms[i].brain);
        child.rotation = rotation;
        child.direction = self.rng.int(4) as u8;
        child.parent_id = Some(id);
        child.growth_plan = Some(self.organisms[i].anatomy.clone());

        // 变异作用在**目标蓝图**上（同成长模式，否则体型永远变不了）
        child.anatomy = child.growth_plan.take().unwrap();
        {
            let parent = &self.organisms[i];
            let cfg = &self.config;
            mutate_organism(&mut child, parent, &mut self.rng, cfg);
        }
        let plan = child.anatomy.clone();
        if plan.cells.is_empty() {
            return false;
        }
        child.anatomy = Anatomy::default();
        let seed = plan.cell_at(0, 0).copied().unwrap_or(plan.cells[0]);
        child.anatomy.add_cell(seed.type_id, 0, 0, seed.dir);
        child.growth_plan = Some(plan);
        self.repro_ok += 1;
        self.add_organism(child);
        true
    }

    /// 收一次繁殖成本，记进热池。返回实际付出去的量。
    ///
    /// 抽成函数是因为「失败也要付账」的旧行为要在**每个失败分支**里各调一次，
    /// 复制四遍必然走样。
    fn charge_repro(&mut self, i: usize, to_child: f64) -> f64 {
        let need = self.organisms[i].food_needed(&self.config);
        // 繁殖成本付出去就没了（新生儿从 0 能量起步），所以这笔也要进热池。
        let paid: f64;
        {
            let org = &mut self.organisms[i];
            paid = need.min(org.energy.max(0.0));
            org.energy -= need;
            if org.energy < 0.0 {
                org.energy = 0.0;
            }
        }
        // `to_child` 那部分不是"离开生物圈"，是转给了后代——不进热池。
        self.heat_pool += paid * (1.0 - to_child.clamp(0.0, 1.0));
        paid
    }

    /// 繁殖（外层）：负责**失败冷却**，逻辑本身在 `reproduce_organism_inner`。
    ///
    /// 为什么包一层而不是在每个 `return false` 上各写一句：`inner` 里有 5 条失败
    /// 路径（太小 / 到达上限 / 找不到空地 / 路径被挡 / 变异把身体删空），
    /// 逐条维护迟早漏一条。包一层就只有一个地方需要关心。
    fn reproduce_organism(&mut self, i: usize) -> bool {
        let ok = self.reproduce_organism_inner(i);
        if !ok {
            let cd = self.config.life.repro_cooldown;
            if cd > 0 {
                if let Some(o) = self.organisms.get_mut(i) {
                    o.repro_cooldown = cd;
                }
            }
        }
        ok
    }

    fn reproduce_organism_inner(&mut self, i: usize) -> bool {
        // 有芽位才出芽；**没有芽位就退回普通繁殖**。
        //
        // 第一版这里是"出芽开着就一律走芽"，于是 `bud_slots = 0` 的小个体
        // 直接绝育——`bud_cells_per_slot >= 4` 时世界当场全灭。
        // 退回之后语义才对：小体型照旧在空地上生，大体型才用得上芽。
        if self.budding() && self.bud_slots(i) > 0 {
            return self.reproduce_bud(i);
        }
        self.repro_tries += 1;
        // 旧行为（默认）：**先扣钱**，之后任何一步失败都白付。
        // 新行为：等所有空间检查都通过才扣钱——「找不到地方就继续攒着」。
        //
        // 这个开关不是口味问题：实测 86.5% 的尝试会失败，所以旧行为下繁殖的
        // 实际支出是名义值的 7.4 倍，而且大体型付得更多（要更多连通空地）。
        // 它单独就把体型锁死在 3 格，让 `breed_cost_exponent < 1` 的规模经济失效。
        let strict = self.config.life.repro_fail_costs_energy;

        // 体型门槛：太小的个体攒再多能量也不能繁殖。
        //
        // 这一条是「规模中性」的对症药——见 `LifeCfg::min_breed_cells` 的注释：
        // 给大体型发奖励赢不了「小身体更好找地方」，只能让太小直接出局。
        if self.organisms[i].cell_count() < self.config.life.min_breed_cells {
            self.repro_blocked_too_small += 1;
            return false;
        }

        let mut paid = 0.0;
        if strict {
            paid = self.charge_repro(i, 0.0);
        }

        if !self.can_add_organism() {
            self.repro_blocked_cap += 1;
            return false;
        }

        // ── 找地方 ──
        //
        // `repro_search_tries = 1`（默认）逐位等于旧行为：随机挑一个方向 + 距离，
        // 不行就放弃。调大之后会连续试多个候选位置。
        //
        // 为什么需要它：实测 86% 的尝试失败，其中 62% 卡在「整块空地找不到」，
        // 而全局占用只有 5%——这两个数只能同时成立，如果瓶颈是**局部**拥挤：
        // 候选位置只有 12 个（4 方向 × 距离 4~6），而它们恰好被邻居占了。
        let tries = self.config.life.repro_search_tries.max(1);
        let birth_d = self.organisms[i].anatomy.birth_distance;
        let mut target: Option<(i32, i32, u8)> = None;
        let mut saw_space_fail = false;
        for _ in 0..tries {
            let v = DIR_VECTORS[self.rng.int(4)];
            let distance = birth_d + self.rng.int(3) as i32;
            let nc = self.organisms[i].col + v.0 * distance;
            let nr = self.organisms[i].row + v.1 * distance;
            let rot = self.rng.int(4) as u8;
            // 第一道预检（便宜）：
            // - 旧行为用**母体蓝图**查整块空地；
            // - 成长模式下芽只有 1 格，就只查这一格——这正是「出生难度掉一个量级」的来源。
            if self.config.life.growth_enabled {
                let Some(idx) = self.grid.index(nc, nr) else {
                    saw_space_fail = true;
                    continue;
                };
                if !self.cell_is_free_for_birth(idx) {
                    saw_space_fail = true;
                    continue;
                }
            } else if !self.organisms[i].is_clear(&self.grid, &self.config, nc, nr, rot) {
                saw_space_fail = true;
                continue;
            }
            let (pc, pr, pid) = (
                self.organisms[i].col,
                self.organisms[i].row,
                self.organisms[i].id,
            );
            if !self
                .organisms[i]
                .is_straight_path(&self.grid, nc, nr, pc, pr, pid)
            {
                continue;
            }
            target = Some((nc, nr, rot));
            break;
        }
        let Some((new_col, new_row, rotation)) = target else {
            if saw_space_fail {
                self.repro_blocked_space += 1;
            } else {
                self.repro_blocked_path += 1;
            }
            if strict {
                self.charge_repro(i, 0.0);
            }
            return false;
        };

        let mut child = Organism::new(&self.config, &mut self.rng, &self.grid, new_col, new_row);
        child.brain.copy_from(&self.organisms[i].brain);
        child.rotation = rotation;
        child.direction = self.rng.int(4) as u8;

        let growth = self.config.life.growth_enabled;
        if growth {
            // 芽：只有 1 格，外加一份「要长成什么样」的蓝图。
            child.growth_plan = Some(self.organisms[i].anatomy.clone());
        } else {
            child.anatomy.copy_from(&self.organisms[i].anatomy);
        }

        {
            let parent = &self.organisms[i];
            let cfg = &self.config;
            if growth {
                // **变异必须作用在目标蓝图上**，不能只变异芽那 1 格——
                // 否则体型永远变不了，演化直接停摆。
                // 做法：临时把蓝图塞进 `anatomy` 让 `mutate_organism` 去改，
                // 改完再把蓝图拆出来、重新取 1 格当种子。
                child.anatomy = child.growth_plan.take().unwrap();
                mutate_organism(&mut child, parent, &mut self.rng, cfg);
                let plan = child.anatomy.clone();
                if plan.cells.is_empty() {
                    // 变异把身体删空了：这个后代生不下来
                    if strict {
                        self.charge_repro(i, 0.0);
                    }
                    return false;
                }
                child.anatomy = Anatomy::default();
                let seed = plan.cell_at(0, 0).copied().unwrap_or(plan.cells[0]);
                child.anatomy.add_cell(seed.type_id, 0, 0, seed.dir);
                child.growth_plan = Some(plan);
            } else {
                mutate_organism(&mut child, parent, &mut self.rng, cfg);
            }
        }

        if !child.can_place_at(
            &self.grid,
            &self.config,
            new_col,
            new_row,
            rotation,
            &child.anatomy,
        ) {
            self.repro_blocked_mutated += 1;
            if strict {
                self.charge_repro(i, 0.0);
            }
            return false;
        }

        // ── 卵生（批次 27）──
        //
        // 和出芽是两条并行的路：出芽是 K（芽贴母体、母体每 tick 供养），
        // 卵生是 r（产在空地上、母体不管）。卵**占一格、挡路、能被吃**——
        // 这就是"捕食压力第一次出现在繁殖环节"。
        //
        // 收费和普通繁殖完全一样（含 `seed_frac` 那笔给后代的投资），
        // 只是去向从"直接生成个体"变成"塞进一枚卵"。
        if self.config.life.egging {
            let seed_frac = if growth {
                self.config.life.growth_seed_fraction.clamp(0.0, 1.0)
            } else {
                0.0
            };
            if !strict {
                paid = self.charge_repro(i, seed_frac);
            } else if seed_frac > 0.0 {
                // strict 模式开头已经按全款进热池收过了，把该给后代那份挪回来
                self.heat_pool -= paid * seed_frac;
            }
            child.energy = paid * seed_frac;
            let Some(idx) = self.grid.index(new_col, new_row) else {
                return false;
            };
            self.set_cell_at(idx, ct::EGG, -1);
            let hatch_at = self.tick + self.config.life.egg_hatch_ticks.max(1) as u64;
            self.eggs.insert(
                idx,
                Egg {
                    hatch_at,
                    parent: self.organisms[i].id,
                    child,
                },
            );
            self.repro_ok += 1;
            return true;
        }

        // 所有检查都过了，这时候才真正收费（非 strict 模式）
        let seed_frac = if growth {
            self.config.life.growth_seed_fraction.clamp(0.0, 1.0)
        } else {
            0.0
        };
        if !strict {
            paid = self.charge_repro(i, seed_frac);
        } else if seed_frac > 0.0 {
            // strict 模式在开头已经按"全款进热池"收过了，
            // 这里把该给芽的那一份从热池挪回来——不是凭空造，只是改去向。
            self.heat_pool -= paid * seed_frac;
        }
        if seed_frac > 0.0 {
            child.energy = paid * seed_frac;
        }
        self.repro_ok += 1;
        self.add_organism(child);
        true
    }

    // ────────────────────────────── 重置 ──────────────────────────────

    pub fn reset(&mut self, keep_terrain: bool) {
        self.clear_organisms();
        self.grid.wrap = self.config.world.wrap_edges;
        self.grid.fill(ct::EMPTY, keep_terrain);
        self.refresh_light();
        self.init_food_index();
        self.init_terrain_index();
        self.species = SpeciesRegistry::default();
        self.next_organism_id = 1;
        self.tick = 0;
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.config.world.cols = cols;
        self.config.world.rows = rows;
        self.organisms.clear();
        self.id_index.clear();
        self.grid = Grid::new(cols, rows, self.config.world.wrap_edges);
        self.soil = vec![255; self.grid.len];
        self.mover_deaths = (0, 0);
        self.still_deaths = (0, 0);
        self.fields.resize(self.grid.len);
        self.fields.clear();
        self.light.resize(cols, rows);
        self.refresh_light();
        self.init_food_index();
        self.init_terrain_index();
        self.species = SpeciesRegistry::default();
        self.next_organism_id = 1;
        self.tick = 0;
    }

    // ────────────────────────────── 环境场 ──────────────────────────────

    /// 推进一格环境场（温度 / 辐射）并重算光场。**由主程序按墙钟 60 Hz 调用**，
    /// 不在 `step()` 里。
    ///
    /// 光场在这里重算是图省事：它是地形的纯函数，`O(cols×rows)` 一次扫描约 40 µs，
    /// 60 Hz 下占 0.24% CPU。换成「地形变了才重算」要找出所有改 `floor` 的路径
    /// （画/擦/填充/重置/读档/工坊放置……），漏一个就是光场和地形不同步的怪 bug。
    pub fn update_fields(&mut self) {
        // 光场只在必要时重算：它是地形/光源/天空光的纯函数，这三样都变得很慢。
        // 天空光单独比对，因为它由滑块驱动，随时可能变。
        let sky = self.sky_level();
        let patch = self.patchiness();
        let floor = self.patch_floor();
        if self.light_dirty
            || self.light.sky != sky
            || self.light.patchiness != patch
            || self.light.patch_floor != floor
        {
            self.refresh_light();
        }
        let World { grid, fields, .. } = self;
        fields.update(grid);
    }

    /// 天空光强度（0–255），由 `config.world.sun_intensity`（0–1）换算。
    ///
    /// 拨到 0 就是「永夜」：世界里只剩玩家放的点光源。这是「世界不再强行分上下」
    /// 的关键——光从哪来完全由玩家决定。
    fn sky_level(&self) -> u8 {
        let v = self.config.world.sun_intensity.clamp(0.0, 1.0);
        (v * crate::light::SUN_MAX as f64).round() as u8
    }

    /// 重算光场（改了光源或天空光之后调）。
    fn refresh_light(&mut self) {
        let sky = self.sky_level();
        let patch = self.patchiness();
        let floor = self.patch_floor();
        self.light_dirty = false;
        let World { grid, light, .. } = self;
        light.sky = sky;
        light.patchiness = patch;
        light.patch_floor = floor;
        light.rebuild(grid);
    }

    /// 云影强度（0~1）。面板上拖动时逐帧比较，所以单独抽出来。
    fn patchiness(&self) -> f32 {
        self.config.world.sky_patchiness.clamp(0.0, 1.0) as f32
    }

    /// 富贫斑块的地板：暗廊里还剩多少比例的天空光（1.0 = 无效果）。
    ///
    /// `patch_richness = r` → 地板 `1 - r`。做成派生值而不是第二个配置项，
    /// 是因为它们本来就是一件事的两面：**越"富贫悬殊"，暗处就越穷**。
    /// 让用户面对两个互相牵制的滑块只会调不明白。
    fn patch_floor(&self) -> f32 {
        let r = self.config.world.patch_richness.clamp(0.0, 1.0);
        (1.0 - r) as f32
    }

    // ────────────────────────────── 点光源 ──────────────────────────────

    /// 在 (col,row) 放一个点光源。`strength = 0` 等于删除。
    ///
    /// 光源**独立于物质世界**：不占格子、不挡路、画笔擦不到。生物可以直接穿过它。
    pub fn set_light_source(&mut self, col: i32, row: i32, strength: u8) {
        if strength == 0 {
            self.light.remove_source_at(col, row);
        } else {
            self.light.set_source(col, row, strength);
        }
        self.refresh_light();
    }

    /// 删掉 (col,row) 的光源，返回有没有删掉
    pub fn remove_light_source(&mut self, col: i32, row: i32) -> bool {
        let removed = self.light.remove_source_at(col, row);
        if removed {
            self.refresh_light();
        }
        removed
    }

    /// 删掉光标附近**最近**的那个点光源，返回有没有删掉。
    ///
    /// 为什么不按精确坐标删：光源在画面上是一团光晕，视觉上的"这个光源"
    /// 和它的格子中心差几格是常事；拖拽放置还会一次留下一串。
    /// 按"最近的"删，点哪儿都能删掉眼前那个，不用对准。
    pub fn remove_nearest_light_source(&mut self, col: i32, row: i32, radius: i32) -> bool {
        let mut best: Option<(usize, i32)> = None;
        for (k, s) in self.light.sources.iter().enumerate() {
            let d = (s.col - col).abs().max((s.row - row).abs());
            if d <= radius && best.is_none_or(|(_, bd)| d < bd) {
                best = Some((k, d));
            }
        }
        match best {
            Some((k, _)) => {
                self.light.sources.remove(k);
                self.refresh_light();
                true
            }
            None => false,
        }
    }

    /// **温度视图用的场**：把「高于环境的温差」换算成绝对温度再归一化。
    ///
    /// 不能直接拿 `fields.temp` 去着色——那是**温差**，世界 30℃ 时全场都是 0，
    /// 画面纯黑，看起来像坏了。用户调了环境温度却看不到任何变化，
    /// 就是因为这件事。
    ///
    /// 归一化范围固定 `0 .. TEMP_VIEW_MAX_C`，**不随环境温度移动**：
    /// 如果范围跟着 `ambient + span` 一起动，调环境温度时底色会跟着一起漂，
    /// 反而看不出变化。
    pub fn temp_view(&self) -> Vec<u8> {
        let ambient = self.config.world.ambient_temp;
        let span = self.config.world.temp_span.max(1.0);
        self.fields
            .temp
            .iter()
            .map(|&t| {
                let celsius = ambient + (t as f64 / 255.0) * span;
                (celsius / Self::TEMP_VIEW_MAX_C * 255.0).clamp(0.0, 255.0) as u8
            })
            .collect()
    }

    /// 这一格有没有点光源（UI 的「点一下放置、再点一下移除」用）
    pub fn light_source_at(&self, col: i32, row: i32) -> Option<u8> {
        self.light
            .source_index_at(col, row)
            .map(|k| self.light.sources[k].strength)
    }

    // ────────────────────────────── 统计 ──────────────────────────────

    pub fn snapshot_stats(&self) -> WorldStats {
        let counts = self.grid.count_by_type();
        let food = self.food_cells.len();
        let mut body = 0usize;
        for &t in cell_type::LIVING_CELL_TYPES.iter() {
            body += counts[t as usize] as usize;
        }
        WorldStats {
            tick: self.tick,
            cols: self.grid.cols,
            rows: self.grid.rows,
            cells: self.grid.len,
            non_empty: food + counts[ct::WALL as usize] as usize + body,
            food,
            wall: counts[ct::WALL as usize] as usize,
            body,
            organisms: self.organisms.len(),
        }
    }

    pub fn organism_stats(&self) -> OrgStats {
        let n = self.organisms.len();
        if n == 0 {
            return OrgStats::default();
        }
        let mut cells = 0usize;
        let mut max_cells = 0usize;
        let (mut movers, mut eyes, mut killers) = (0usize, 0usize, 0usize);
        let mut age = 0f64;
        let mut mutability = 0f64;
        // 收入来源分布（批次 22.1）：按**实际到账能量**归类，不是看有没有嘴。
        //
        // 这是回答"到底有没有捕食者"的唯一正确口径——「生产者 + 嘴」在结构上是自养，
        // 但九成收入来自咬活体时，它在生态里就是捕食者。
        let mut income = [0usize; 5];
        for org in &self.organisms {
            let c = org.cell_count();
            cells += c;
            if c > max_cells {
                max_cells = c;
            }
            if org.anatomy.has(Flags::MOVER) {
                movers += 1;
            }
            if org.anatomy.has(Flags::EYE) {
                eyes += 1;
            }
            if org.anatomy.has(Flags::KILLER) {
                killers += 1;
            }
            age += org.age as f64;
            mutability += org.mutability;
            income[org.income_source().slot()] += 1;
        }
        // ── 批次 38：移动者 vs 非移动者的一阶量 ──
        //
        // 收入用 `gain_total / age`（单位时间），而不是累计值：累计值随年龄增长，
        // 比"谁活得久"而不是"谁挣得多"。`age == 0` 的刚出生个体跳过。
        let (mut m_n, mut m_age, mut m_inc, mut m_energy, mut m_soil) = (0usize, 0f64, 0f64, 0f64, 0f64);
        let (mut s_n, mut s_age, mut s_inc) = (0usize, 0f64, 0f64);
        for org in &self.organisms {
            // 死掉的还没被 `compact_organisms` 清走，必须自己跳过——
            // 否则"活着的移动者"会把刚死的也数进去。
            if !org.alive {
                continue;
            }
            let age = org.age as f64;
            if age < 1.0 {
                continue;
            }
            let inc = org.income_total() / age;
            if org.anatomy.has(Flags::MOVER) {
                m_n += 1;
                m_age += age;
                m_inc += inc;
                m_energy += org.energy;
                if self.config.life.soil_deplete > 0 {
                    if let Some(k) = self.grid.index(org.col, org.row) {
                        m_soil += self.soil[k] as f64 * 100.0 / 255.0;
                    }
                }
            } else {
                s_n += 1;
                s_age += age;
                s_inc += inc;
            }
        }
        let mf = m_n.max(1) as f64;
        let sf = s_n.max(1) as f64;
        let life = |(sum, cnt): (u64, u64)| if cnt == 0 { 0.0 } else { sum as f64 / cnt as f64 };

        let nf = n as f64;
        let adult_n = self
            .organisms
            .iter()
            .filter(|o| o.growth_plan.is_none())
            .count();
        let adult_cells: usize = self
            .organisms
            .iter()
            .filter(|o| o.growth_plan.is_none())
            .map(|o| o.cell_count())
            .sum();
        OrgStats {
            count: n,
            avg_cells: cells as f64 / nf,
            avg_cells_adult: if adult_n == 0 {
                0.0
            } else {
                adult_cells as f64 / adult_n as f64
            },
            buds_pct: (n - adult_n) as f64 * 100.0 / nf,
            species_clustered: self.species_clusters(2),
            max_cells,
            movers,
            eyes,
            killers,
            avg_age: age / nf,
            avg_mutability: mutability / nf,
            income: income.map(|k| k as f64 * 100.0 / nf),
            mover_n: m_n,
            mover_age: m_age / mf,
            mover_income: m_inc / mf,
            mover_energy: m_energy / mf,
            mover_life: life(self.mover_deaths),
            mover_soil: if self.config.life.soil_deplete > 0 {
                m_soil / mf
            } else {
                100.0
            },
            still_age: s_age / sf,
            still_income: s_inc / sf,
            still_life: life(self.still_deaths),
        }
    }

    /// 状态的稳定指纹（黄金快照对照用）：
    /// 网格类型 + 归属 + 地形 + 每个个体的关键字段
    pub fn state_digest(&self) -> String {
        let mut h: u32 = 0x811c_9dc5;
        let mut mix = |v: u32| {
            h ^= v;
            h = h.wrapping_mul(0x0100_0193);
        };
        for i in 0..self.grid.len {
            mix(self.grid.types[i] as u32);
            mix(self.grid.owners[i] as u32);
            mix(self.grid.floor[i] as u32);
        }
        for org in &self.organisms {
            mix(org.id);
            mix(org.col as u32);
            mix(org.row as u32);
            mix(org.rotation as u32);
            mix(org.direction as u32);
            mix(org.age as u32);
            mix((org.energy * 1000.0).round() as i32 as u32);
            mix((org.damage as i32) as u32);
            mix(org.anatomy.cells.len() as u32);
            for c in &org.anatomy.cells {
                mix(c.type_id as u32);
                mix(c.dx as u32);
                mix(c.dy as u32);
                mix(c.dir as u32);
            }
        }
        format!("{:08x}", h)
    }
}

#[derive(Debug, Default, Clone)]
pub struct WorldStats {
    pub tick: u64,
    pub cols: usize,
    pub rows: usize,
    pub cells: usize,
    pub non_empty: usize,
    pub food: usize,
    pub wall: usize,
    pub body: usize,
    pub organisms: usize,
}

#[derive(Debug, Default, Clone)]
pub struct OrgStats {
    pub count: usize,
    /// 全体平均体型。**注意：被 1 格的芽稀释**（实测芽占 47%），
    /// 要"长成的身体多大"看 `avg_cells_adult`。
    pub avg_cells: f64,
    /// **成体**平均体型（批次 32）。这是"长成的身体多大"的正确读数。
    pub avg_cells_adult: f64,
    /// 芽（`growth_plan` 还在）占全体的百分比。芽占比**随参数变**，
    /// 所以跨配置比 `avg_cells` 时必须把它一起看，否则量的是芽的多少。
    pub buds_pct: f64,
    /// **聚类物种数**（构成距离 ≤ `kin_distance`，个体数 ≥ 2）。
    /// 和 `species.alive_count`（精确指纹）是两个口径，前者才是"多少种形态"。
    pub species_clustered: usize,
    pub max_cells: usize,
    pub movers: usize,
    pub eyes: usize,
    pub killers: usize,
    pub avg_age: f64,
    pub avg_mutability: f64,
    /// 按**实际收入来源**归类的个体占比（%），顺序见 `Income::slot`：
    /// 光合 / 吃食物 / 咬活体 / 供养 / 没收入。五项加起来是 100。
    pub income: [f64; 5],

    // ── 批次 38：移动者 vs 非移动者的**一阶量** ──
    //
    // 种群是二阶量（繁殖 × 死亡 × 空间竞争），3 种子的噪声能到 ±30%（批次 37 实测）。
    // 这一组是「每个个体自己的账」：年龄、单位时间收入、能量、寿命。用它才量得出
    // 「这个策略好不好」，而不是「这次跑出来多少人」。
    /// 移动者（有运动细胞）数量
    pub mover_n: usize,
    /// 移动者平均年龄（tick，只算活着的）
    pub mover_age: f64,
    /// 移动者平均**单位时间总收入**（`gain_total / age`，只算活着的）
    pub mover_income: f64,
    /// 移动者平均当前能量
    pub mover_energy: f64,
    /// 移动者平均**寿命**（tick，只算已经死掉的）
    pub mover_life: f64,
    /// 移动者脚下平均地力（0~100%，地力关闭时恒 100）
    pub mover_soil: f64,
    /// 非移动者平均年龄
    pub still_age: f64,
    /// 非移动者平均单位时间总收入
    pub still_income: f64,
    /// 非移动者平均寿命
    pub still_life: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::factory::{seed_origin_of_life, spawn_population};
    use crate::worldgen::generate_demo_world;

    /// 回归：物种统计必须被 step 更新
    ///
    /// 曾经的 bug：`species.update()` 从来没被调用，于是 population 恒为 0 ——
    /// 物种面板空、存活/峰值/灭绝全是 0。
    #[test]
    fn species_stats_are_updated_by_step() {
        let mut w = World::new(Config::default(), Rng::new(1001));
        generate_demo_world(&mut w, 1.0, 1.0);
        seed_origin_of_life(&mut w);
        spawn_population(&mut w, 8, 3);

        for _ in 0..100 {
            w.step();
        }

        assert!(w.species.alive_count > 0, "存活物种数应大于 0");
        assert!(w.species.peak_alive_count > 0, "峰值存活应大于 0");

        let total: u32 = w.species.list.iter().map(|s| s.population).sum();
        assert_eq!(
            total as usize,
            w.organisms.len(),
            "各物种个体数之和应等于种群数"
        );

        // 每个存活物种的 population 都应该 > 0
        let alive = w.species.list.iter().filter(|s| s.population > 0).count();
        assert_eq!(alive, w.species.alive_count, "存活计数应与实际相符");
    }

    /// 腐殖质必须能**长在自己那一格**上。
    ///
    /// 用户反馈："腐殖质只在自己周围生成食物，能不能也长在它自己上"。
    /// 这里把四邻域全砌成墙，只剩腐殖质自己那格能放食物——
    /// 如果实现只考虑邻居，这一步什么都不会发生。
    #[test]
    fn humus_can_grow_food_on_itself() {
        let mut cfg = Config::default();
        cfg.world.cols = 8;
        cfg.world.rows = 8;
        cfg.terrain.humus_food_prob = 100.0; // 每 tick 必成功
        cfg.world.food_decay_rate = 0.0; // 别让食物刚长出来就腐败
        let mut w = World::new(cfg, Rng::new(7));
        w.fill_grid(ct::EMPTY, false);

        for (c, r) in [(3, 4), (5, 4), (4, 3), (4, 5)] {
            w.set_cell(c, r, ct::WALL, -1);
        }
        w.set_cell(4, 4, ct::HUMUS, -1);

        w.step();

        assert_eq!(w.type_at(4, 4), ct::FOOD, "腐殖质自己那格应该长出食物");
        assert_eq!(w.floor_at(4, 4), ct::HUMUS, "地形本身要留着（食物被吃后露回来）");
    }

    /// 反向：腐殖质周围的空地也要能长（不能为了"长在自己身上"把邻域弄丢）
    #[test]
    fn humus_still_grows_on_neighbors() {
        let mut cfg = Config::default();
        cfg.world.cols = 8;
        cfg.world.rows = 8;
        cfg.terrain.humus_food_prob = 100.0;
        cfg.world.food_decay_rate = 0.0;
        let mut w = World::new(cfg, Rng::new(11));
        w.fill_grid(ct::EMPTY, false);
        w.set_cell(4, 4, ct::HUMUS, -1);

        // 跑若干 tick，统计食物出现在邻域的次数
        let mut on_neighbor = 0;
        for _ in 0..40 {
            w.step();
            for (c, r) in [(3, 4), (5, 4), (4, 3), (4, 5)] {
                if w.type_at(c, r) == ct::FOOD {
                    on_neighbor += 1;
                    w.reveal_floor(w.grid.index(c, r).unwrap());
                }
            }
            if w.type_at(4, 4) == ct::FOOD {
                w.reveal_floor(w.grid.index(4, 4).unwrap());
            }
        }
        assert!(on_neighbor > 0, "腐殖质应该也会在周围长食物");
    }

    // ────────────────────────── 每细胞倍率真的接到模拟上了吗 ──────────────────────────
    //
    // 光"编译通过 + 默认值不漂移"证明不了参数有用：倍率乘错地方、或者压根没乘，
    // 编译器和黄金快照都不会报错（默认 1.0 时乘不乘结果一样）。这一组测试
    // 每一项都把倍率**调离 1.0**，断言模拟结果真的跟着变、且方向正确。

    /// 造一个 8×8 的空世界 + 一个指定身体的小个体，返回世界和它的 id
    fn tiny_world(cells: &[(u8, i32, i32)]) -> (World, u32) {
        let mut cfg = Config::default();
        cfg.world.cols = 8;
        cfg.world.rows = 8;
        // **单细胞功能测试一律关掉代谢**：这些测试量的是"这一口吃到多少 / 这一格产出多少"，
        // 代谢开着的话每 tick 都会先扣掉一笔开销，测出来的就是"收入减开销"。
        // 代谢本身由 `upkeep_*` 那组测试单独覆盖（它们自己建世界）。
        cfg.life.metabolism_scale = 0.0;
        let mut w = World::new(cfg, Rng::new(1));
        w.fill_grid(ct::EMPTY, false);

        let mut rng = Rng::new(1);
        let mut org = Organism::new(&w.config, &mut rng, &w.grid, 3, 3);
        for (t, dx, dy) in cells {
            org.anatomy.add_cell(*t, *dx, *dy, 0);
        }
        org.anatomy.recompute();
        let id = w.add_organism(org);
        (w, id)
    }

    /// 新食物默认装满一份（`food_energy_scale`）。
    #[test]
    fn new_food_carries_one_portion() {
        let (mut w, _) = tiny_world(&[(ct::MOUTH, 0, 0)]);
        w.config.life.food_energy_scale = 2.5;
        let idx = w.grid.index(5, 5).unwrap();
        w.set_cell_at(idx, ct::FOOD, -1);
        assert!(
            (w.grid.food[idx] - 2.5).abs() < 1e-6,
            "新食物应该装满一份，实测 {}",
            w.grid.food[idx]
        );
    }

    /// 腐败是**渐进**的：能量一点点掉，不是整格突然消失。
    ///
    /// 旧实现是「每 tick 有 N% 概率随机删一格」，同一块肉要么在、要么突然没了，
    /// 看不出过程，也没法做"守尸 / 抢食"这种依赖时间的行为。
    #[test]
    fn food_decays_gradually() {
        let (mut w, _) = tiny_world(&[(ct::MOUTH, 0, 0)]);
        w.config.world.food_decay_rate = 10.0; // 10% / tick
        w.config.world.food_decay_floor = 0.0;
        let idx = w.grid.index(5, 5).unwrap();
        w.set_food_at(idx, 1.0);

        w.apply_food_decay();
        assert_eq!(w.grid.types[idx], ct::FOOD, "一格不该在第一次衰减就消失");
        assert!(
            (w.grid.food[idx] - 0.9).abs() < 1e-5,
            "应该掉 10%，实测 {}",
            w.grid.food[idx]
        );
        assert!(
            (w.heat_pool - 0.1).abs() < 1e-5,
            "流失的能量必须进热池（不能凭空消失），实测 {}",
            w.heat_pool
        );

        // 一直衰减到能量耗尽才清格
        for _ in 0..200 {
            w.apply_food_decay();
        }
        assert_eq!(w.grid.types[idx], ct::EMPTY, "能量耗尽后应该清格");
        assert!(
            (w.heat_pool - 1.0).abs() < 1e-3,
            "整份能量最终都该记进热池，实测 {}",
            w.heat_pool
        );
    }

    /// 绝对下限：小块食物不会拖着 0.001 点能量赖着不走。
    #[test]
    fn decay_floor_kills_tiny_food() {
        let (mut w, _) = tiny_world(&[(ct::MOUTH, 0, 0)]);
        w.config.world.food_decay_rate = 0.0; // 只留绝对项
        w.config.world.food_decay_floor = 0.05;
        let idx = w.grid.index(5, 5).unwrap();
        w.set_food_at(idx, 0.02);
        w.apply_food_decay();
        assert_eq!(
            w.grid.types[idx],
            ct::EMPTY,
            "低于绝对下限的小食物应该一次就消失"
        );
    }

    /// 嘴吃掉的是**这一格剩下的全部能量**，不是固定的「一份」。
    #[test]
    fn mouth_takes_whatever_is_left_in_the_cell() {
        let run = |e: f64| -> f64 {
            // 带一个储食细胞 + 超高的 storage_food_bonus，把繁殖判定隔离掉
            // （`act_organism` 末尾会扣繁殖成本，不隔离就测不准）
            let (mut w, _) = tiny_world(&[(ct::MOUTH, 0, 0), (ct::STORAGE, 0, -1)]);
            w.config.life.storage_food_bonus = 1000.0;
            let idx = w.grid.index(4, 3).unwrap();
            w.set_food_at(idx, e);
            w.organisms[0].energy = 0.0;
            w.act_organism(0);
            w.organisms[0].energy
        };
        assert!(
            (run(3.0) - 3.0).abs() < 1e-6,
            "满能量食物应该给 3.0，实测 {}",
            run(3.0)
        );
        assert!(
            (run(0.2) - 0.2).abs() < 1e-6,
            "快烂完的食物只值剩下的 0.2，实测 {}",
            run(0.2)
        );
    }

    /// 死亡残骸携带死者**体内攒下的能量**，而不是按细胞类型给的固定值。
    ///
    /// 这是「狩猎才有意义」的地基：肥的死了留肥尸体。
    #[test]
    fn corpse_carries_the_organisms_energy() {
        let (mut w, id) = tiny_world(&[(ct::MOUTH, 0, 0)]);
        let i = w.id_index[&id];
        w.organisms[i].energy = 4.0;
        w.kill_organism(i);

        let total: f64 = w.food_cells.iter().map(|&k| w.grid.food[k] as f64).sum();
        assert!(
            (total - 4.0).abs() < 1e-3,
            "尸体总能量应该等于死者体内能量 4.0，实测 {}",
            total
        );
        assert!(!w.food_cells.is_empty(), "死了至少该留下一格尸体");
    }

    /// 体内没能量时不该留下"空食物格"（否则立刻被腐败清掉，白折腾）。
    #[test]
    fn broke_corpse_leaves_no_food() {
        let (mut w, id) = tiny_world(&[(ct::MOUTH, 0, 0)]);
        let i = w.id_index[&id];
        w.organisms[i].energy = 0.0;
        w.kill_organism(i);
        assert!(
            w.food_cells.is_empty(),
            "零能量尸体不该留下食物格，实测 {} 格",
            w.food_cells.len()
        );
    }

    /// 嘴的「摄取倍率」要乘在食物能量上。
    #[test]
    fn bite_mult_scales_energy_gain() {
        let run = |mult: f64| -> f64 {
            // 带一个储食细胞并把「储食加成」拉到 1000：繁殖阈值变成 2*(1+1000)，
            // 吃 4 份食物远远够不着，就不会触发繁殖扣费。
            //
            // 为什么必须隔离：`act_organism` 末尾**包含繁殖判定**，
            // 能量攒够阈值就扣掉一份成本再生一个——不隔离的话测到的是
            // "吃到的能量减去繁殖成本"，倍率关系会被这个常数偏移毁掉。
            let (mut w, _) = tiny_world(&[(ct::MOUTH, 0, 0), (ct::STORAGE, 1, 0)]);
            w.config.life.storage_food_bonus = 1000.0;
            w.config.cells[ct::MOUTH as usize].bite_mult = mult;
            // 嘴在 (3,3)，四邻域各放一份食物
            for (dx, dy) in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
                let i = w.grid.index(3 + dx, 3 + dy).unwrap();
                w.set_cell_at(i, ct::FOOD, -1);
            }
            let e0 = w.organisms[0].energy;
            w.act_organism(0);
            w.organisms[0].energy - e0
        };
        let base = run(1.0);
        assert!(base > 0.0, "嘴应该吃到东西（默认 4 份食物）");
        assert!(
            (run(2.0) - base * 2.0).abs() < 1e-9,
            "摄取倍率 2.0 应该拿到双倍能量（{} vs {}）",
            run(2.0),
            base
        );
        assert_eq!(run(0.0), 0.0, "摄取倍率 0 应该吃不到任何能量");
    }

    // ── 批次 9.5：嘴咬活体 ──

    /// 捕食者在 (3,3)，猎物在 (4,3)——正好贴着捕食者嘴细胞的右邻。
    ///
    /// 两个个体要分开 `add_organism`（同一个 id 的话 `owners` 里就分不出敌我，
    /// 而"不能咬自己"正是要测的东西之一）。
    fn predator_and_prey(
        pred: &[(u8, i32, i32)],
        prey: &[(u8, i32, i32)],
    ) -> (World, usize, usize) {
        let mut cfg = Config::default();
        cfg.world.cols = 12;
        cfg.world.rows = 12;
        // 这一组测试量的是"这一口吃到多少"，不该被**专精奖励**污染：
        // 它们构造的捕食者都是纯捕食者（没有生产者细胞），
        // 默认 `predator_specialization = 2.0` 会让每一口都翻倍。
        // 专精本身由 `spec_*` 那组测试单独覆盖。
        cfg.life.predator_specialization = 1.0;
        let mut w = World::new(cfg, Rng::new(1));
        w.fill_grid(ct::EMPTY, false);
        let mut rng = Rng::new(1);
        let mut a = Organism::new(&w.config, &mut rng, &w.grid, 3, 3);
        for (t, dx, dy) in pred {
            a.anatomy.add_cell(*t, *dx, *dy, 0);
        }
        a.anatomy.recompute();
        let pid = w.add_organism(a);
        let mut b = Organism::new(&w.config, &mut rng, &w.grid, 4, 3);
        for (t, dx, dy) in prey {
            b.anatomy.add_cell(*t, *dx, *dy, 0);
        }
        b.anatomy.recompute();
        let vid = w.add_organism(b);
        let pi = w.organism_index(pid).unwrap();
        let vi = w.organism_index(vid).unwrap();
        (w, pi, vi)
    }

    /// 直接跑嘴细胞的结算，绕过 `act_organism`（它末尾含繁殖判定，
    /// 会把这口饭扣成繁殖成本——批次 6 踩过）。
    fn bite_once(w: &mut World, pi: usize) {
        let cell = w.organisms[pi].anatomy.cells[0];
        w.run_cell_function(pi, cell);
    }

    /// 嘴会把相邻活体身上的能量抽过来，到账的只是 `bite_efficiency` 那部分。
    #[test]
    fn mouth_drains_a_living_neighbour() {
        let (mut w, pi, vi) = predator_and_prey(&[(ct::MOUTH, 0, 0)], &[(ct::PRODUCER, 0, 0)]);
        w.config.life.bite_rate = 0.3;
        w.config.life.bite_efficiency = 0.8;
        w.organisms[vi].energy = 5.0;
        bite_once(&mut w, pi);
        assert!(
            (w.organisms[vi].energy - 4.7).abs() < 1e-9,
            "猎物应该被抽走 0.3，实测剩 {}",
            w.organisms[vi].energy
        );
        assert!(
            (w.organisms[pi].energy - 0.24).abs() < 1e-9,
            "捕食者应该拿到 0.3 × 0.8 = 0.24，实测 {}",
            w.organisms[pi].energy
        );
        assert!(
            (w.energy_wasted - 0.06).abs() < 1e-9,
            "消化损耗 0.06 应该进 energy_wasted，实测 {}",
            w.energy_wasted
        );
    }

    /// 抽走量被**对方当时剩的能量**卡住：咬空壳什么也得不到，也不会把对方咬成负数。
    #[test]
    fn bite_is_capped_by_what_the_victim_has() {
        let (mut w, pi, vi) = predator_and_prey(&[(ct::MOUTH, 0, 0)], &[(ct::PRODUCER, 0, 0)]);
        w.config.life.bite_rate = 0.3;
        w.config.life.bite_efficiency = 0.8;
        w.organisms[vi].energy = 0.05;
        bite_once(&mut w, pi);
        assert_eq!(w.organisms[vi].energy, 0.0, "猎物能量不该变成负数");
        assert!(
            (w.organisms[pi].energy - 0.04).abs() < 1e-9,
            "只能拿到 0.05 × 0.8 = 0.04，实测 {}",
            w.organisms[pi].energy
        );
    }

    /// 装甲格按 `ArmorResist` 减免被抽走的能量（出厂 1.0 = 完全免疫）。
    #[test]
    fn armor_resists_being_bitten() {
        let (mut w, pi, vi) = predator_and_prey(&[(ct::MOUTH, 0, 0)], &[(ct::ARMOR, 0, 0)]);
        w.config.life.bite_rate = 0.3;
        w.config.life.bite_efficiency = 0.8;
        w.organisms[vi].energy = 5.0;
        bite_once(&mut w, pi);
        assert_eq!(
            w.organisms[vi].energy, 5.0,
            "出厂减免 1.0 时装甲格应该完全咬不动"
        );

        // 调到 0.5：抽走量减半，到账 0.15 × 0.8 = 0.12
        w.config.cells[ct::ARMOR as usize].armor_resist = 0.5;
        bite_once(&mut w, pi);
        assert!(
            (w.organisms[vi].energy - 4.85).abs() < 1e-9,
            "减免 0.5 应该只抽走 0.15，实测剩 {}",
            w.organisms[vi].energy
        );
        assert!((w.organisms[pi].energy - 0.12).abs() < 1e-9);
    }

    /// `bite_rate = 0` 等于关掉「咬活体」——但嘴照样吃食物。
    #[test]
    fn bite_rate_zero_keeps_mouth_on_food_only() {
        let (mut w, pi, vi) = predator_and_prey(&[(ct::MOUTH, 0, 0)], &[(ct::PRODUCER, 0, 0)]);
        w.config.life.bite_rate = 0.0;
        w.organisms[vi].energy = 5.0;
        bite_once(&mut w, pi);
        assert_eq!(w.organisms[vi].energy, 5.0, "关掉之后不该抽走任何能量");
        assert_eq!(w.organisms[pi].energy, 0.0);

        // 同一套配置下，食物照吃
        let idx = w.grid.index(3, 2).unwrap();
        w.set_cell_at(idx, ct::FOOD, -1);
        bite_once(&mut w, pi);
        assert!(
            w.organisms[pi].energy > 0.0,
            "关掉咬活体不该影响吃食物"
        );
    }

    /// 嘴不会咬自己的身体（同一 id 的格子直接跳过）。
    #[test]
    fn a_mouth_never_bites_its_own_body() {
        let (mut w, _) = tiny_world(&[(ct::MOUTH, 0, 0), (ct::STORAGE, 1, 0)]);
        w.organisms[0].energy = 5.0;
        let cell = w.organisms[0].anatomy.cells[0];
        w.run_cell_function(0, cell);
        assert_eq!(
            w.organisms[0].energy, 5.0,
            "嘴不该从自己的身体里抽能量"
        );
        assert_eq!(w.energy_wasted, 0.0);
    }

    /// 摄取倍率同样乘在「咬活体」上——一个嘴细胞越"大"，一口抽得越多。
    #[test]
    fn bite_mult_scales_live_biting() {
        let (mut w, pi, vi) = predator_and_prey(&[(ct::MOUTH, 0, 0)], &[(ct::PRODUCER, 0, 0)]);
        w.config.life.bite_rate = 0.3;
        w.config.life.bite_efficiency = 1.0;
        w.config.cells[ct::MOUTH as usize].bite_mult = 2.0;
        w.organisms[vi].energy = 5.0;
        bite_once(&mut w, pi);
        assert!(
            (w.organisms[pi].energy - 0.6).abs() < 1e-9,
            "摄取倍率 2.0 应该抽 0.6，实测 {}",
            w.organisms[pi].energy
        );
    }

    // ── 批次 21：一张嘴一口 ──

    /// 一张嘴左右各贴一只猎物：这一 tick 只有**一只**被抽走能量。
    ///
    /// 批次 20 的扫描发现单嘴上限是 `4 × bite_rate`——嘴因此成了光合的 10 倍收益，
    /// 76% 的个体都靠它活着。这个测试盯的就是那个"4 倍"消失。
    #[test]
    fn a_mouth_bites_only_one_target_per_tick() {
        let mut cfg = Config::default();
        cfg.world.cols = 12;
        cfg.world.rows = 12;
        let mut w = World::new(cfg, Rng::new(1));
        w.fill_grid(ct::EMPTY, false);
        w.config.life.bite_rate = 0.3;
        w.config.life.bite_efficiency = 0.8;
        // 纯捕食者 + 默认专精 2.0 会让每一口翻倍，这里量的是"咬几口"，先关掉
        w.config.life.predator_specialization = 1.0;
        let mut rng = Rng::new(1);
        let mut mouth = Organism::new(&w.config, &mut rng, &w.grid, 5, 5);
        mouth.anatomy.add_cell(ct::MOUTH, 0, 0, 0);
        mouth.anatomy.recompute();
        let pid = w.add_organism(mouth);
        // 左右各一只，都贴着这张嘴（四邻域里有两格是猎物）
        for (col, row) in [(6, 5), (4, 5)] {
            let mut p = Organism::new(&w.config, &mut rng, &w.grid, col, row);
            p.anatomy.add_cell(ct::PRODUCER, 0, 0, 0);
            p.anatomy.recompute();
            let id = w.add_organism(p);
            let i = w.organism_index(id).unwrap();
            w.organisms[i].energy = 5.0;
        }
        let pi = w.organism_index(pid).unwrap();
        let cell = w.organisms[pi].anatomy.cells[0];
        w.run_cell_function(pi, cell);

        let drained: f64 = w
            .organisms
            .iter()
            .filter(|o| o.id != pid)
            .map(|o| 5.0 - o.energy)
            .sum();
        assert!(
            (drained - 0.3).abs() < 1e-9,
            "两个邻居只该被抽走一口 0.3，实测共 {drained}"
        );
        assert!(
            (w.organisms[pi].energy - 0.24).abs() < 1e-9,
            "到账 0.3 × 0.8 = 0.24，实测 {}",
            w.organisms[pi].energy
        );
    }

    /// 关掉「一张嘴一口」退回批次 9.5 的行为：两个邻居各抽一次（0.6）。
    #[test]
    fn old_mouth_mode_bites_every_neighbour() {
        let mut cfg = Config::default();
        cfg.world.cols = 12;
        cfg.world.rows = 12;
        let mut w = World::new(cfg, Rng::new(1));
        w.fill_grid(ct::EMPTY, false);
        w.config.life.bite_rate = 0.3;
        w.config.life.bite_efficiency = 0.8;
        w.config.life.predator_specialization = 1.0;
        w.config.life.mouth_one_bite = false;
        let mut rng = Rng::new(1);
        let mut mouth = Organism::new(&w.config, &mut rng, &w.grid, 5, 5);
        mouth.anatomy.add_cell(ct::MOUTH, 0, 0, 0);
        mouth.anatomy.recompute();
        let pid = w.add_organism(mouth);
        for (col, row) in [(6, 5), (4, 5)] {
            let mut p = Organism::new(&w.config, &mut rng, &w.grid, col, row);
            p.anatomy.add_cell(ct::PRODUCER, 0, 0, 0);
            p.anatomy.recompute();
            let id = w.add_organism(p);
            let i = w.organism_index(id).unwrap();
            w.organisms[i].energy = 5.0;
        }
        let pi = w.organism_index(pid).unwrap();
        let cell = w.organisms[pi].anatomy.cells[0];
        w.run_cell_function(pi, cell);

        let drained: f64 = w
            .organisms
            .iter()
            .filter(|o| o.id != pid)
            .map(|o| 5.0 - o.energy)
            .sum();
        assert!(
            (drained - 0.6).abs() < 1e-9,
            "旧行为应该两个邻居各抽 0.3，实测共 {drained}"
        );
    }

    /// 「一口」只约束活体：四邻域的食物照样可以一次舔干净。
    ///
    /// 食物是躺在格子里的存量，一口能舔到几格由几何决定；"咬住不放"的配额
    /// 不该被它占掉，否则嘴在尸体堆里反而变弱。
    #[test]
    fn one_bite_does_not_limit_food() {
        let mut cfg = Config::default();
        cfg.world.cols = 12;
        cfg.world.rows = 12;
        let mut w = World::new(cfg, Rng::new(1));
        w.fill_grid(ct::EMPTY, false);
        let mut rng = Rng::new(1);
        let mut mouth = Organism::new(&w.config, &mut rng, &w.grid, 5, 5);
        mouth.anatomy.add_cell(ct::MOUTH, 0, 0, 0);
        mouth.anatomy.recompute();
        let pid = w.add_organism(mouth);
        let pi = w.organism_index(pid).unwrap();
        // 四邻域全铺食物，每格 0.1
        for (col, row) in [(5, 4), (6, 5), (5, 6), (4, 5)] {
            let idx = w.grid.index(col, row).unwrap();
            w.set_cell_at(idx, ct::FOOD, -1);
            w.grid.food[idx] = 0.1;
        }
        let cell = w.organisms[pi].anatomy.cells[0];
        w.run_cell_function(pi, cell);
        assert!(
            (w.organisms[pi].energy - 0.4).abs() < 1e-9,
            "四格食物应该全吃掉（0.4），实测 {}",
            w.organisms[pi].energy
        );
    }

    // ── 批次 22：昼夜循环 ──

    /// `day_length = 0`（默认）等于永远是正午。
    #[test]
    fn zero_day_length_keeps_permanent_noon() {
        let (mut w, _) = tiny_world(&[(ct::PRODUCER, 0, 0)]);
        assert_eq!(w.config.world.day_length, 0);
        for t in [0u64, 1, 37, 1000] {
            w.tick = t;
            w.update_daylight();
            assert_eq!(w.light.daylight, 1.0, "关闭昼夜时日照必须恒为 1");
        }
    }

    /// 余弦相位：0 = 正午、1/4 = 半亮、1/2 = 午夜。
    #[test]
    fn daylight_follows_a_cosine() {
        let (mut w, _) = tiny_world(&[(ct::PRODUCER, 0, 0)]);
        w.config.world.day_length = 100;
        w.config.world.night_floor = 0.1;

        let at = |w: &mut World, t: u64| {
            w.tick = t;
            w.update_daylight();
            w.light.daylight
        };
        assert!((at(&mut w, 0) - 1.0).abs() < 0.01, "phase 0 是正午");
        assert!((at(&mut w, 25) - 0.55).abs() < 0.01, "phase 1/4 是半亮");
        assert!((at(&mut w, 50) - 0.1).abs() < 0.01, "phase 1/2 是午夜");
        assert!((at(&mut w, 75) - 0.55).abs() < 0.01, "phase 3/4 回到半亮");
        // 周期性：下一轮的同一相位必须一样
        assert!((at(&mut w, 150) - 0.1).abs() < 0.01, "第二天的午夜还是午夜");
    }

    /// 夜间光合收入必须明显低于正午——这是整个机制的落点。
    #[test]
    fn photosynthesis_drops_at_night() {
        let (mut w, id) = tiny_world(&[(ct::PRODUCER, 0, 0)]);
        w.config.world.day_length = 100;
        w.config.world.night_floor = 0.1;
        w.refresh_light();
        let i = w.organism_index(id).unwrap();
        let cell = w.organisms[i].anatomy.cells[0];

        w.tick = 0;
        w.update_daylight();
        w.organisms[i].energy = 0.0;
        w.run_cell_function(i, cell);
        let noon = w.organisms[i].energy;

        w.tick = 50;
        w.update_daylight();
        w.organisms[i].energy = 0.0;
        w.run_cell_function(i, cell);
        let night = w.organisms[i].energy;

        assert!(noon > 0.0, "正午必须有产出");
        assert!(
            night > 0.0,
            "夜间不能全黑——月光要让个体能靠储能熬过去，而不是必然死亡"
        );
        assert!(
            night < noon * 0.2,
            "夜间产出应该只有正午的一成上下，实测 {night:.4} vs {noon:.4}"
        );
    }

    // ── 批次 23：捕食端体型差 ──

    /// `size_advantage = 0`（默认）必须逐位等于旧行为。
    #[test]
    fn zero_size_advantage_is_a_no_op() {
        let (mut w, pi, vi) = predator_and_prey(&[(ct::MOUTH, 0, 0)], &[(ct::PRODUCER, 0, 0)]);
        w.config.life.bite_rate = 0.1;
        w.config.life.bite_efficiency = 1.0;
        w.config.life.size_advantage = 0.0;
        w.organisms[vi].energy = 5.0;
        bite_once(&mut w, pi);
        assert!(
            (w.organisms[vi].energy - 4.9).abs() < 1e-9,
            "关掉体型差时应该还是抽 0.1，实测剩 {}",
            w.organisms[vi].energy
        );
    }

    /// 大咬小加成、小咬大打折——这是给大体型的第一条**进攻性**收益。
    #[test]
    fn size_advantage_rewards_big_predators() {
        // 3 格捕食者（1 嘴 + 2 储食）咬 1 格猎物
        let (mut w, pi, vi) = predator_and_prey(
            &[(ct::MOUTH, 0, 0), (ct::STORAGE, 0, 1), (ct::STORAGE, 0, -1)],
            &[(ct::PRODUCER, 0, 0)],
        );
        w.config.life.bite_rate = 0.1;
        w.config.life.bite_efficiency = 1.0;
        w.config.life.size_advantage = 0.5;
        w.organisms[vi].energy = 5.0;
        bite_once(&mut w, pi);
        let want = 0.1 * 3f64.sqrt();
        assert!(
            (w.organisms[vi].energy - (5.0 - want)).abs() < 1e-9,
            "3 格咬 1 格应该抽 0.1×√3 = {want:.4}，实测剩 {}",
            w.organisms[vi].energy
        );

        // 反过来：1 格捕食者咬 3 格猎物 → 打折
        let (mut w2, pi2, vi2) = predator_and_prey(
            &[(ct::MOUTH, 0, 0)],
            &[(ct::PRODUCER, 0, 0), (ct::STORAGE, 0, 1), (ct::STORAGE, 0, -1)],
        );
        w2.config.life.bite_rate = 0.1;
        w2.config.life.bite_efficiency = 1.0;
        w2.config.life.size_advantage = 0.5;
        w2.organisms[vi2].energy = 5.0;
        bite_once(&mut w2, pi2);
        let want2 = 0.1 / 3f64.sqrt();
        assert!(
            (w2.organisms[vi2].energy - (5.0 - want2)).abs() < 1e-9,
            "1 格咬 3 格应该只抽 0.1/√3 = {want2:.4}，实测剩 {}",
            w2.organisms[vi2].energy
        );
    }

    /// 倍率有上下限：20 格咬 1 格夹到 4.0，1 格咬 20 格夹到 0.25。
    #[test]
    fn size_advantage_is_clamped() {
        let mut big = vec![(ct::MOUTH, 0, 0)];
        for k in 1..20 {
            big.push((ct::STORAGE, (k % 5) as i32 - 2, (k / 5) as i32 + 1));
        }
        let (mut w, pi, vi) = predator_and_prey(&big, &[(ct::PRODUCER, 0, 0)]);
        w.config.life.size_advantage = 0.5;
        let vi_id = w.organisms[vi].id;
        let adv = w.size_advantage(pi, vi_id);
        assert!(
            (adv - 4.0).abs() < 1e-9,
            "20 格咬 1 格应该被夹到 4.0，实测 {adv}"
        );

        let (mut w2, pi2, vi2) = predator_and_prey(&[(ct::MOUTH, 0, 0)], &big[..]);
        w2.config.life.size_advantage = 0.5;
        let vi2_id = w2.organisms[vi2].id;
        let adv2 = w2.size_advantage(pi2, vi2_id);
        assert!(
            (adv2 - 0.25).abs() < 1e-9,
            "1 格咬 20 格应该被夹到 0.25，实测 {adv2}"
        );
    }

    // ── 批次 24：同种相残惩罚 ──

    /// `kin_bite_penalty = 0`（默认）必须逐位等于旧行为。
    #[test]
    fn zero_kin_penalty_is_a_no_op() {
        let (mut w, pi, vi) = predator_and_prey(&[(ct::MOUTH, 0, 0)], &[(ct::PRODUCER, 0, 0)]);
        w.config.life.bite_rate = 0.1;
        w.config.life.bite_efficiency = 1.0;
        w.config.life.kin_bite_penalty = 0.0;
        w.organisms[vi].energy = 5.0;
        bite_once(&mut w, pi);
        assert!((w.organisms[vi].energy - 4.9).abs() < 1e-9);
    }

    /// 同种之间咬的收益打折；异种不受影响。
    #[test]
    fn same_species_bites_are_discounted() {
        let (mut w, pi, vi) = predator_and_prey(&[(ct::MOUTH, 0, 0)], &[(ct::PRODUCER, 0, 0)]);
        w.config.life.bite_rate = 0.1;
        w.config.life.bite_efficiency = 1.0;
        w.config.life.kin_bite_penalty = 0.5;

        // 不同种（默认指纹不同）→ 全额
        w.organisms[vi].energy = 5.0;
        bite_once(&mut w, pi);
        assert!(
            (w.organisms[vi].energy - 4.9).abs() < 1e-9,
            "异种应该抽满 0.1，实测剩 {}",
            w.organisms[vi].energy
        );

        // 同种 → 打对折
        let fp: std::sync::Arc<str> = std::sync::Arc::from("同一个蓝图");
        w.organisms[pi].species_fingerprint = Some(fp.clone());
        w.organisms[vi].species_fingerprint = Some(fp);
        w.organisms[vi].energy = 5.0;
        w.organisms[pi].energy = 0.0;
        bite_once(&mut w, pi);
        assert!(
            (w.organisms[vi].energy - 4.95).abs() < 1e-9,
            "同种应该只抽 0.05，实测剩 {}",
            w.organisms[vi].energy
        );
    }

    /// 惩罚开到 1.0 = 同种完全咬不动。
    #[test]
    fn full_kin_penalty_blocks_cannibalism() {
        let (mut w, pi, vi) = predator_and_prey(&[(ct::MOUTH, 0, 0)], &[(ct::PRODUCER, 0, 0)]);
        w.config.life.bite_rate = 0.1;
        w.config.life.bite_efficiency = 1.0;
        w.config.life.kin_bite_penalty = 1.0;
        let fp: std::sync::Arc<str> = std::sync::Arc::from("同一个蓝图");
        w.organisms[pi].species_fingerprint = Some(fp.clone());
        w.organisms[vi].species_fingerprint = Some(fp);
        w.organisms[vi].energy = 5.0;
        bite_once(&mut w, pi);
        assert_eq!(w.organisms[vi].energy, 5.0, "同种完全咬不动");
    }

    // ── 批次 26：繁殖失败冷却 ──

    /// 繁殖失败之后必须设上冷却，冷却期内不再尝试。
    #[test]
    fn failed_reproduction_sets_a_cooldown() {
        let (mut w, id) = tiny_world(&[(ct::PRODUCER, 0, 0)]);
        w.config.life.repro_cooldown = 5;
        // 走普通繁殖（出芽不需要空地，测不出"失败"）
        w.config.life.budding_enabled = false;
        // 把整个世界填满墙：一定找不到空地
        for i in 0..w.grid.len {
            if w.grid.types[i] == ct::EMPTY {
                w.set_cell_at(i, ct::WALL, -1);
            }
        }
        let i = w.organism_index(id).unwrap();
        w.organisms[i].energy = 100.0;
        w.act_organism(i);
        assert!(
            w.organisms[i].repro_cooldown > 0,
            "繁殖失败之后应该进入冷却，实测 {}",
            w.organisms[i].repro_cooldown
        );

        // 冷却期内不该再尝试
        let tries_before = w.repro_tries;
        w.organisms[i].energy = 100.0;
        w.act_organism(i);
        assert_eq!(
            w.repro_tries, tries_before,
            "冷却期内不该再发起繁殖尝试"
        );
    }

    /// `repro_cooldown = 0`（默认）逐位等于旧行为：失败了下一 tick 立刻重试。
    #[test]
    fn zero_repro_cooldown_retries_immediately() {
        let (mut w, id) = tiny_world(&[(ct::PRODUCER, 0, 0)]);
        w.config.life.repro_cooldown = 0;
        w.config.life.budding_enabled = false;
        for i in 0..w.grid.len {
            if w.grid.types[i] == ct::EMPTY {
                w.set_cell_at(i, ct::WALL, -1);
            }
        }
        let i = w.organism_index(id).unwrap();
        w.organisms[i].energy = 100.0;
        w.act_organism(i);
        assert_eq!(w.organisms[i].repro_cooldown, 0);

        let tries_before = w.repro_tries;
        w.organisms[i].energy = 100.0;
        w.act_organism(i);
        assert!(
            w.repro_tries > tries_before,
            "关掉冷却时应该每 tick 都重试"
        );
    }

    // ── 批次 27：卵生 ──

    /// `egging = false`（默认）时繁殖直接产出个体，不产卵。
    #[test]
    fn egging_off_births_directly() {
        let (mut w, id) = tiny_world(&[(ct::PRODUCER, 0, 0)]);
        w.config.life.budding_enabled = false;
        w.config.life.egging = false;
        let i = w.organism_index(id).unwrap();
        w.organisms[i].energy = 100.0;
        assert!(w.reproduce_organism(i));
        assert!(w.eggs.is_empty(), "关掉卵生时不该有卵");
        assert_eq!(w.organisms.len(), 2, "应该直接多一个个体");
    }

    /// 打开卵生：先产卵（个体数不变），到期才孵出芽。
    #[test]
    fn egging_lays_then_hatches() {
        let (mut w, id) = tiny_world(&[(ct::PRODUCER, 0, 0)]);
        w.config.life.budding_enabled = false;
        w.config.life.egging = true;
        w.config.life.egg_hatch_ticks = 5;
        let i = w.organism_index(id).unwrap();
        w.organisms[i].energy = 100.0;

        assert!(w.reproduce_organism(i), "产卵应该算繁殖成功");
        assert_eq!(w.eggs.len(), 1, "应该产下一枚卵");
        assert_eq!(w.organisms.len(), 1, "孵化前不该多出个体");

        // 卵还没到期：孵不出来
        w.tick += 1;
        w.hatch_eggs();
        assert_eq!(w.organisms.len(), 1, "没到期不该孵化");

        // 推到到期
        for _ in 0..6 {
            w.tick += 1;
            w.hatch_eggs();
        }
        assert!(w.eggs.is_empty(), "到期后卵应该消失");
        assert_eq!(w.organisms.len(), 2, "卵应该孵出一个个体");
    }

    /// 嘴可以吃卵：吃掉之后卵消失，能量进捕食者。
    #[test]
    fn a_mouth_can_eat_an_egg() {
        let (mut w, id) = tiny_world(&[(ct::MOUTH, 0, 0)]);
        let idx = w.grid.index(4, 3).unwrap();
        let mut rng = Rng::new(1);
        let mut child = Organism::new(&w.config, &mut rng, &w.grid, 4, 3);
        child.energy = 1.0;
        w.set_cell_at(idx, ct::EGG, -1);
        w.eggs.insert(
            idx,
            Egg {
                hatch_at: 10_000,
                parent: 999,
                child,
            },
        );

        let i = w.organism_index(id).unwrap();
        let cell = w.organisms[i].anatomy.cells[0];
        w.run_cell_function(i, cell);

        assert!(w.eggs.is_empty(), "卵应该被吃掉");
        assert_eq!(w.grid.types[idx], ct::EMPTY, "格子应该空出来");
        assert!(
            w.organisms[i].energy > 0.0,
            "捕食者应该拿到卵里的能量，实测 {}",
            w.organisms[i].energy
        );
    }

    /// 卵里存着的能量必须进审计，否则 `注入 == 存量` 会凭空少一块。
    #[test]
    fn egg_energy_counts_in_the_audit() {
        let (mut w, id) = tiny_world(&[(ct::PRODUCER, 0, 0)]);
        w.config.life.budding_enabled = false;
        w.config.life.egging = true;
        let i = w.organism_index(id).unwrap();
        w.organisms[i].energy = 100.0;
        assert!(w.reproduce_organism(i));

        let audit = w.energy_audit();
        assert!(
            audit.eggs > 0.0,
            "卵里应该有能量（`growth_seed_fraction` 那一份），实测 {}",
            audit.eggs
        );
    }

    // ── 批次 31：捕食专精奖励 ──

    /// `predator_specialization = 1.0`（默认）必须逐位等于旧行为。
    #[test]
    fn spec_one_is_a_no_op() {
        let (mut w, pi, vi) = predator_and_prey(&[(ct::MOUTH, 0, 0)], &[(ct::PRODUCER, 0, 0)]);
        w.config.life.bite_rate = 0.1;
        w.config.life.bite_efficiency = 1.0;
        w.config.life.predator_specialization = 1.0;
        w.organisms[vi].energy = 5.0;
        bite_once(&mut w, pi);
        assert!((w.organisms[vi].energy - 4.9).abs() < 1e-9);
    }

    /// 纯捕食者（无生产者细胞）拿到倍率；混养体拿不到。
    #[test]
    fn spec_rewards_pure_predators_only() {
        // 纯捕食者：只有嘴
        let (mut w, pi, vi) = predator_and_prey(&[(ct::MOUTH, 0, 0)], &[(ct::PRODUCER, 0, 0)]);
        w.config.life.bite_rate = 0.1;
        w.config.life.bite_efficiency = 1.0;
        w.config.life.predator_specialization = 1.5;
        w.organisms[vi].energy = 5.0;
        bite_once(&mut w, pi);
        assert!(
            (w.organisms[vi].energy - 4.85).abs() < 1e-9,
            "纯捕食者应该抽 0.15，实测剩 {}",
            w.organisms[vi].energy
        );

        // 混养体：嘴 + 生产者
        let (mut w2, pi2, vi2) = predator_and_prey(
            &[(ct::MOUTH, 0, 0), (ct::PRODUCER, 1, 0)],
            &[(ct::PRODUCER, 0, 0)],
        );
        w2.config.life.bite_rate = 0.1;
        w2.config.life.bite_efficiency = 1.0;
        w2.config.life.predator_specialization = 1.5;
        w2.organisms[vi2].energy = 5.0;
        bite_once(&mut w2, pi2);
        assert!(
            (w2.organisms[vi2].energy - 4.9).abs() < 1e-9,
            "混养体不该拿到专精奖励，实测剩 {}",
            w2.organisms[vi2].energy
        );
    }

    // ── 批次 38：移动者一阶量 ──

    /// 一阶量按「有没有运动细胞」分组：收入是 `累计收入 / 年龄`，寿命来自死亡登记。
    #[test]
    fn mover_metrics_split_by_has_mover() {
        let (mut w, id) = tiny_world(&[(ct::MOVER, 0, 0)]);
        let i = w.organism_index(id).unwrap();
        w.organisms[i].age = 10;
        w.organisms[i].gain_photo = 2.0;

        let st = w.organism_stats();
        assert_eq!(st.mover_n, 1, "这一只是移动者");
        assert!(
            (st.mover_income - 0.2).abs() < 1e-9,
            "单位时间收入应该是 2.0/10，实测 {}",
            st.mover_income
        );
        assert_eq!(st.mover_life, 0.0, "还没死，寿命应该是 0（没有样本）");

        // 死掉之后进寿命登记
        w.kill_organism(i);
        let st2 = w.organism_stats();
        assert!(
            (st2.mover_life - 10.0).abs() < 1e-9,
            "寿命应该是 10 tick，实测 {}",
            st2.mover_life
        );
        assert_eq!(st2.mover_n, 0, "死了就不算在活着的那一栏里");
    }

    // ── 批次 35：地力耗竭 ──

    /// 光合抽地力、恢复加地力。
    #[test]
    fn soil_depletes_under_a_producer_and_recovers() {
        let (mut w, id) = tiny_world(&[(ct::PRODUCER, 0, 0)]);
        w.config.life.soil_deplete = 8;
        w.config.life.soil_recover = 1;
        let i = w.organism_index(id).unwrap();
        let idx = w.grid.index(w.organisms[i].col, w.organisms[i].row).unwrap();
        assert_eq!(w.soil[idx], 255, "新世界地力应该是满的");

        let cell = w.organisms[i].anatomy.cells[0];
        w.run_cell_function(i, cell);
        assert_eq!(w.soil[idx], 247, "光合应该抽走 8 点地力");

        w.apply_soil_recover();
        assert_eq!(w.soil[idx], 248, "恢复 1 点");
    }

    /// 地力耗尽时产量掉到 `soil_floor`——这是「移动」的收益来源。
    #[test]
    fn depleted_soil_cuts_photosynthesis_to_the_floor() {
        let make = |soil: u8| {
            let (mut w, id) = tiny_world(&[(ct::PRODUCER, 0, 0)]);
            w.config.life.soil_deplete = 8;
            w.config.life.soil_floor = 0.25;
            let i = w.organism_index(id).unwrap();
            let idx = w.grid.index(w.organisms[i].col, w.organisms[i].row).unwrap();
            w.soil[idx] = soil;
            let cell = w.organisms[i].anatomy.cells[0];
            w.run_cell_function(i, cell);
            (w.organisms[i].gain_photo, w.organisms[i].energy)
        };
        let (full, e_full) = make(255);
        let (dry, e_dry) = make(0);
        assert!(full > 0.0, "满地下应该有产出（检查 tiny_world 的光照），实测 {}", full);
        let ratio = dry / full;
        assert!(
            (ratio - 0.25).abs() < 0.02,
            "地力 0 时应该只剩 25%，实测 {:.3}（满 {} / 干 {}）",
            ratio,
            e_full,
            e_dry
        );
    }

    /// `soil_deplete = 0`（默认）时地力一动不动，逐位等于旧行为。
    #[test]
    fn soil_off_means_no_change() {
        let (mut w, id) = tiny_world(&[(ct::PRODUCER, 0, 0)]);
        assert_eq!(w.config.life.soil_deplete, 0, "默认应该是关的");
        let i = w.organism_index(id).unwrap();
        let idx = w.grid.index(w.organisms[i].col, w.organisms[i].row).unwrap();
        let cell = w.organisms[i].anatomy.cells[0];
        w.run_cell_function(i, cell);
        w.apply_soil_recover();
        assert_eq!(w.soil[idx], 255, "关掉时地力不该变");
    }

    // ── 批次 33 / 34：大脑消融 + 捕食者抗饿 ──

    /// 消融模式 1：恒定方向——一直朝当前朝向走，大脑完全不参与。
    #[test]
    fn brain_ablation_constant_mode_goes_straight() {
        let (mut w, id) = tiny_world(&[(ct::MOVER, 0, 0)]);
        w.config.life.brain_ablation = 1;
        let i = w.organism_index(id).unwrap();
        w.organisms[i].rotation = 2;
        w.think_organism(i);
        assert!(!w.organisms[i].plan_turn, "恒定方向模式不该转身");
        assert_eq!(w.organisms[i].plan_direction, 2, "应该朝当前朝向（2）走");
    }

    /// 消融模式 2：每 tick 伪随机方向——合法范围、且和朝向无关。
    #[test]
    fn brain_ablation_random_mode_is_in_range() {
        let (mut w, id) = tiny_world(&[(ct::MOVER, 0, 0)]);
        w.config.life.brain_ablation = 2;
        let i = w.organism_index(id).unwrap();
        w.organisms[i].rotation = 2;
        w.think_organism(i);
        assert!(!w.organisms[i].plan_turn);
        assert!(w.organisms[i].plan_direction < 4);
    }

    /// 抗饿倍率只对**没有生产者**的个体生效。
    #[test]
    fn starve_limit_scales_only_for_pure_predators() {
        let (mut w, _) = tiny_world(&[(ct::PRODUCER, 0, 0)]);
        w.config.life.starvation_ticks = 60;
        w.config.life.predator_starve_mult = 3.0;
        // 生产者：宽限期不变
        assert_eq!(w.starve_limit(0), 60);

        let (mut w2, _) = tiny_world(&[(ct::MOUTH, 0, 0)]);
        w2.config.life.starvation_ticks = 60;
        w2.config.life.predator_starve_mult = 3.0;
        assert_eq!(w2.starve_limit(0), 180, "纯捕食者应该能扛 3 倍时间");

        // 关掉（1.0）时逐位等于旧行为
        w2.config.life.predator_starve_mult = 1.0;
        assert_eq!(w2.starve_limit(0), 60);
    }

    // ── 批次 32：物种聚类与构成判据 ──

    /// 同构成的两个身体算一个物种，构成差得远的分开。
    #[test]
    fn species_clusters_groups_similar_bodies() {
        let mut cfg = Config::default();
        cfg.world.cols = 12;
        cfg.world.rows = 12;
        let mut w = World::new(cfg, Rng::new(1));
        w.fill_grid(ct::EMPTY, false);
        let mut rng = Rng::new(1);

        for (col, cells) in [
            (2, vec![(ct::PRODUCER, 0, 0), (ct::MOUTH, 0, 1)]),
            (5, vec![(ct::PRODUCER, 0, 0), (ct::MOUTH, 0, 1)]),
            // 位置换了，构成一样 → 还是同一种
            (8, vec![(ct::PRODUCER, 0, 0), (ct::MOUTH, 1, 0)]),
            // 只有一格生产者 → 完全不同的一种
            (10, vec![(ct::PRODUCER, 0, 0)]),
        ] {
            let mut o = Organism::new(&w.config, &mut rng, &w.grid, col, 3);
            for (t, dx, dy) in cells {
                o.anatomy.add_cell(t, dx, dy, 0);
            }
            o.anatomy.recompute();
            w.add_organism(o);
        }
        assert_eq!(
            w.species_clusters(1),
            2,
            "三个「生产者+嘴」（摆法不同）算一种，单独的 1 格生产者算另一种"
        );
        // 精确指纹是另一个口径：同构成不同摆法也算不同指纹
        assert_eq!(
            w.species.map.len(),
            3,
            "指纹口径会把「嘴在右边」和「嘴在下边」算成两个物种"
        );
    }

    /// `kin_by_species` 打开后，芽按**它要长成的蓝图**判亲缘——母子算同种。
    ///
    /// 这是批次 24 那个「已知脆弱点」的直接修补：指纹判据下母子差一格就是异种。
    #[test]
    fn kin_by_species_treats_a_bud_as_family() {
        let mut cfg = Config::default();
        cfg.world.cols = 12;
        cfg.world.rows = 12;
        cfg.life.predator_specialization = 1.0;
        cfg.life.bite_rate = 0.1;
        cfg.life.bite_efficiency = 1.0;
        cfg.life.kin_bite_penalty = 0.5;
        cfg.life.kin_by_species = true;
        let mut w = World::new(cfg, Rng::new(1));
        w.fill_grid(ct::EMPTY, false);
        let mut rng = Rng::new(1);

        let mut mom = Organism::new(&w.config, &mut rng, &w.grid, 3, 3);
        mom.anatomy.add_cell(ct::MOUTH, 0, 0, 0);
        mom.anatomy.add_cell(ct::PRODUCER, 0, 1, 0);
        mom.anatomy.recompute();
        let mid = w.add_organism(mom);
        let plan = w.organisms[w.organism_index(mid).unwrap()].anatomy.clone();

        // 芽：anatomy 只有 1 格，蓝图是母体那一份
        let mut bud = Organism::new(&w.config, &mut rng, &w.grid, 4, 3);
        bud.anatomy.add_cell(ct::MOUTH, 0, 0, 0);
        bud.anatomy.recompute();
        bud.growth_plan = Some(plan);
        bud.energy = 1.0;
        let bid = w.add_organism(bud);
        let bi = w.organism_index(bid).unwrap();

        let mi = w.organism_index(mid).unwrap();
        let cell = w.organisms[mi].anatomy.cells[0];
        w.run_cell_function(mi, cell);

        assert!(
            (w.organisms[bi].energy - 0.95).abs() < 1e-9,
            "芽应该被当成亲缘（抽 0.05 而不是 0.1），实测剩 {}",
            w.organisms[bi].energy
        );
    }

    /// `kin_by_species = false`（默认）时行为必须和批次 31 逐位相同：只看指纹。
    #[test]
    fn kin_by_species_off_keeps_fingerprint_rule() {
        let (mut w, pi, vi) = predator_and_prey(&[(ct::MOUTH, 0, 0)], &[(ct::PRODUCER, 0, 0)]);
        w.config.life.bite_rate = 0.1;
        w.config.life.bite_efficiency = 1.0;
        w.config.life.kin_bite_penalty = 0.5;
        w.config.life.kin_by_species = false;
        w.organisms[vi].energy = 5.0;
        bite_once(&mut w, pi);
        assert!(
            (w.organisms[vi].energy - 4.9).abs() < 1e-9,
            "不同指纹不该打折，实测剩 {}",
            w.organisms[vi].energy
        );
    }

    // ── 批次 9.6：食物算不算路障（移动 / 繁殖两个开关）──

    /// 默认：食物挡移动（和 1.x 一致）。
    #[test]
    fn food_blocks_movement_by_default() {
        let (mut w, _) = tiny_world(&[(ct::MOVER, 0, 0)]);
        let idx = w.grid.index(4, 3).unwrap();
        w.set_cell_at(idx, ct::FOOD, -1);
        assert!(
            !w.organisms[0].is_clear(&w.grid, &w.config, 4, 3, 0),
            "默认配置下不该能走进食物格"
        );
    }

    /// 放开之后能踩上去，但**那格食物被压碎，能量进热池**（不是吃掉）。
    #[test]
    fn trampling_food_credits_the_heat_pool() {
        let (mut w, _) = tiny_world(&[(ct::MOVER, 0, 0)]);
        w.config.combat.food_blocks_movement = false;
        let idx = w.grid.index(4, 3).unwrap();
        w.set_food_at(idx, 2.5);
        assert!(w.organisms[0].is_clear(&w.grid, &w.config, 4, 3, 0));

        // `write_grid` 移动之后做的就是这件事
        w.set_cell_at(idx, ct::MOVER, 0);
        assert_eq!(w.grid.types[idx], ct::MOVER);
        assert_eq!(w.grid.food[idx], 0.0, "被压碎的食物不该留能量");
        assert!(
            (w.heat_pool - 2.5).abs() < 1e-6,
            "压碎的 2.5 能量应该进热池，实测 {}",
            w.heat_pool
        );
    }

    /// **嘴吃东西不能被算成"压碎"**：它走 `reveal_floor`（owner = -1），
    /// 能量已经记到个体身上了，再进热池就是重复计数。
    #[test]
    fn eating_food_does_not_credit_the_heat_pool() {
        let (mut w, _) = tiny_world(&[(ct::MOUTH, 0, 0)]);
        let idx = w.grid.index(3, 2).unwrap();
        w.set_cell_at(idx, ct::FOOD, -1);
        let cell = w.organisms[0].anatomy.cells[0];
        w.run_cell_function(0, cell);
        assert!(w.organisms[0].energy > 0.0, "嘴应该吃到东西");
        assert_eq!(w.heat_pool, 0.0, "吃掉的能量不该被记成压碎");
    }

    /// 两个开关互不牵连：只放开移动，繁殖照样被食物挡着（反之亦然）。
    #[test]
    fn movement_and_reproduction_food_flags_are_independent() {
        let (mut w, _) = tiny_world(&[(ct::MOVER, 0, 0)]);
        let idx = w.grid.index(4, 3).unwrap();
        w.set_cell_at(idx, ct::FOOD, -1);
        let anat = w.organisms[0].anatomy.clone();

        w.config.combat.food_blocks_movement = false;
        assert!(w.organisms[0].is_clear(&w.grid, &w.config, 4, 3, 0));
        assert!(
            !w.organisms[0].can_place_at(&w.grid, &w.config, 4, 3, 0, &anat),
            "放开移动不该顺带放开繁殖"
        );

        w.config.combat.food_blocks_movement = true;
        w.config.combat.food_blocks_reproduction = false;
        assert!(!w.organisms[0].is_clear(&w.grid, &w.config, 4, 3, 0));
        assert!(
            w.organisms[0].can_place_at(&w.grid, &w.config, 4, 3, 0, &anat),
            "放开繁殖不该顺带放开移动"
        );
    }

    /// 造一个「光合测试个体」：1 个生产者 + 1 个储食，满照，能量清零。
    ///
    /// **必须把繁殖阈值拉到天上去**：`act_organism` 末尾含繁殖判定，能量一过阈值
    /// 就会被扣掉一份繁殖成本，测到的就不是光合产出了。这个坑在批次 6 测
    /// `bite_mult` 时踩过一次。
    ///
    /// 容量 = 2 格 × 2.0 + 1 储食 × 6.0 = 10.0，足够装下这些测试的产出。
    fn photo_world() -> World {
        let (mut w, _) = tiny_world(&[(ct::PRODUCER, 0, 0), (ct::STORAGE, 1, 0)]);
        w.config.life.storage_food_bonus = 1000.0;
        w.config.life.photosynthesis_rate = 0.5;
        w.config.world.sun_intensity = 1.0;
        w.light.sky_level.fill(255);
        w.light.level.fill(255);
        w.organisms[0].energy = 0.0;
        w
    }

    /// 生产者的「生产速度」要乘在光合产出上。
    #[test]
    fn prod_mult_scales_photosynthesis() {
        let run = |mult: f64| -> f64 {
            let mut w = photo_world();
            w.config.cells[ct::PRODUCER as usize].prod_mult = mult;
            w.act_organism(0);
            w.organisms[0].energy
        };
        assert!(
            (run(1.0) - 0.5).abs() < 1e-9,
            "满照下应产出光合速率 0.5，实测 {}",
            run(1.0)
        );
        assert!((run(2.0) - 1.0).abs() < 1e-9, "倍率 2.0 应产出双倍");
        assert_eq!(run(0.0), 0.0, "生产速度 0 时不该产出能量");
    }

    /// 没有光就没有光合——这是「光成了唯一能量来源」的守门测试。
    ///
    /// 以前生产者是永动机（凭空造食物），这条测试在旧模型下根本写不出来。
    #[test]
    fn photosynthesis_requires_light() {
        let run = |level: u8| -> f64 {
            let mut w = photo_world();
            w.light.sky_level.fill(level);
            w.light.level.fill(level);
            w.act_organism(0);
            w.organisms[0].energy
        };
        assert_eq!(run(0), 0.0, "全黑时生产者产出必须为 0（它是纯负担）");
        assert!((run(255) - 0.5).abs() < 1e-9, "满照时产出 = 光合速率");
        // 半照应该正好一半
        let half = run(128);
        assert!(
            (half - 0.5 * 128.0 / 255.0).abs() < 1e-9,
            "半照产出应正比于光强，实测 {}",
            half
        );
    }

    /// 光合只看**光场**，不额外乘天空光系数。
    ///
    /// 这条钉的是一个踩过的坑：`sun_intensity` 曾经同时当「天空光强度」和
    /// 「光合乘数」用，结果关掉天空光之后连玩家放的点光源也一起熄了。
    /// 正确的关系是：`sun_intensity` → 天空光注入强度 → 光场 → 光合。
    #[test]
    fn photosynthesis_follows_the_light_field_not_a_multiplier() {
        let mut w = photo_world();
        w.config.world.sun_intensity = 0.0; // 天空光全关
        w.light.sky = 0;
        w.light.sky_level.fill(0);
        w.light.lamp_level.fill(255); // 但这一格被点光源照亮
        w.light.level.fill(255);
        w.act_organism(0);
        assert!(
            (w.organisms[0].energy - 0.5).abs() < 1e-9,
            "有点光源时，天空光为 0 也该正常光合，实测 {:.4}",
            w.organisms[0].energy
        );
    }

    /// 和 `photo_world` 一样，但世界够大（24×24、个体在 12,12）。
    ///
    /// 深度测试需要个体上方至少有 9 行——`tiny_world` 只有 8×8，放不下。
    fn photo_world_big() -> World {
        let mut cfg = Config::default();
        cfg.world.cols = 24;
        cfg.world.rows = 24;
        let mut w = World::new(cfg, Rng::new(1));
        w.fill_grid(ct::EMPTY, false);
        let mut rng = Rng::new(1);
        let mut org = Organism::new(&w.config, &mut rng, &w.grid, 12, 12);
        org.anatomy.add_cell(ct::PRODUCER, 0, 0, 0);
        org.anatomy.add_cell(ct::STORAGE, 1, 0, 0);
        org.anatomy.recompute();
        w.add_organism(org);
        w.config.life.storage_food_bonus = 1000.0;
        w.config.life.photosynthesis_rate = 0.5;
        w.config.world.sun_intensity = 1.0;
        w.light.sky_level.fill(255);
        w.light.level.fill(255);
        w.organisms[0].energy = 0.0;
        w
    }

    /// 把一个"别人的身体格"塞到 (col,row)。用来测遮挡，不需要真的造第二个个体。
    fn put_foreign_body(w: &mut World, col: i32, row: i32) {
        let i = w.grid.index(col, row).expect("坐标越界");
        w.grid.owners[i] = 999;
        w.grid.types[i] = ct::PRODUCER;
    }

    /// 生产者细胞在世界里的坐标（`tiny_world` 的枢轴位置由它决定）
    fn producer_pos(w: &World) -> (i32, i32) {
        let o = &w.organisms[0];
        let cell = o
            .anatomy
            .cells
            .iter()
            .find(|c| c.type_id == ct::PRODUCER)
            .expect("测试个体应该有生产者");
        (o.real_col(cell), o.real_row(cell))
    }

    /// 别人挡在上面 → 天空光按 (1-k)^n 衰减。
    #[test]
    fn a_foreign_body_above_shades_the_producer() {
        let mut w = photo_world();
        let (c, r) = producer_pos(&w);
        put_foreign_body(&mut w, c, r - 1);
        w.act_organism(0);
        let expect = 0.5 * 0.75; // rate 0.5 × (1-0.25)
        assert!(
            (w.organisms[0].energy - expect).abs() < 1e-9,
            "一格遮挡应该只剩 75% 产出（期望 {:.4}，实测 {:.4}）",
            expect,
            w.organisms[0].energy
        );

        // 再叠一格 → (0.75)^2
        put_foreign_body(&mut w, c, r - 2);
        w.organisms[0].energy = 0.0;
        w.act_organism(0);
        let expect = 0.5 * 0.75 * 0.75;
        assert!(
            (w.organisms[0].energy - expect).abs() < 1e-9,
            "两格遮挡应该剩 56% 产出（期望 {:.4}，实测 {:.4}）",
            expect,
            w.organisms[0].energy
        );
    }

    /// **自己的身体也挡自己**：光不知道格子归谁。
    ///
    /// 第一版只数"别人"，结果演化发现可以把自己叠高、让底下的生产者白嫖光——
    /// 平均体型从 3.0 涨到 4.3，致密格一点没降。这条测试钉住"自己的细胞也数"。
    #[test]
    fn own_body_also_shades_itself() {
        let mut w = photo_world();
        let (c, r) = producer_pos(&w);
        let my_id = w.organisms[0].id as i32;
        let i = w.grid.index(c, r - 1).unwrap();
        w.grid.owners[i] = my_id; // 自己的细胞压在头上
        w.act_organism(0);
        assert!(
            (w.organisms[0].energy - 0.5 * 0.75).abs() < 1e-9,
            "自己的细胞也该挡光（期望 0.375，实测 {:.4}）",
            w.organisms[0].energy
        );
    }

    /// 遮挡只作用于天空光。灯照的地方不该被"上方有身体"罚。
    #[test]
    fn lamp_light_is_not_shaded() {
        let mut w = photo_world();
        w.light.sky_level.fill(0);
        w.light.lamp_level.fill(255);
        w.light.level.fill(255);
        let (c, r) = producer_pos(&w);
        put_foreign_body(&mut w, c, r - 1);
        w.act_organism(0);
        assert!(
            (w.organisms[0].energy - 0.5).abs() < 1e-9,
            "灯照的光不该被遮挡（期望 0.5，实测 {:.4}）",
            w.organisms[0].energy
        );
    }

    /// 强度 0 = 关闭遮挡（对照实验用）。
    #[test]
    fn shade_strength_zero_disables_shading() {
        let mut w = photo_world();
        w.config.life.shade_strength = 0.0;
        let (c, r) = producer_pos(&w);
        put_foreign_body(&mut w, c, r - 1);
        w.act_organism(0);
        assert!((w.organisms[0].energy - 0.5).abs() < 1e-9);
    }

    /// 遮挡只数 `SHADE_DEPTH` 行内的身体：再远的光本来就该由光场自己衰减掉。
    #[test]
    fn shading_stops_at_the_depth_limit() {
        let mut w = photo_world_big();
        let (c, r) = producer_pos(&w);
        assert!(r > SHADE_DEPTH, "测试个体离顶边太近（r = {}）", r);

        // 边界内：应该挡
        put_foreign_body(&mut w, c, r - SHADE_DEPTH);
        w.act_organism(0);
        assert!(
            w.organisms[0].energy < 0.5,
            "深度边界内应该算遮挡，实测 {:.4}",
            w.organisms[0].energy
        );

        // 边界外：不该挡
        let mut w = photo_world_big();
        put_foreign_body(&mut w, c, r - SHADE_DEPTH - 1);
        w.act_organism(0);
        assert!(
            (w.organisms[0].energy - 0.5).abs() < 1e-9,
            "超出深度的身体不该算遮挡，实测 {:.4}",
            w.organisms[0].energy
        );
    }

    /// 垂直方向不能环绕：顶行生产者不该被底部的生物"挡住"。
    ///
    /// 这条钉的是一个真实踩过的坑：`grid.index` 在 `wrap = true` 时会把
    /// `row - d` 折到世界底部，于是最上面几行的光合莫名其妙地被减半。
    #[test]
    fn shading_does_not_wrap_vertically() {
        let mut w = photo_world_big();
        let (c, r) = producer_pos(&w);
        // 在最底一行放一个别人的身体；它和顶部的生产者在"环绕"下只差 1 行
        let last_row = w.grid.rows as i32 - 1;
        put_foreign_body(&mut w, c, last_row);
        let _ = r;
        w.act_organism(0);
        assert!(
            (w.organisms[0].energy - 0.5).abs() < 1e-9,
            "底部的身体不该挡住顶部的光（环绕了），实测 {:.4}",
            w.organisms[0].energy
        );
    }

    /// 维持成本必须每 tick 从能量里扣掉。
    ///
    /// 这条钉的是「多一格要不要付账」在**个体层面**成立。宏观上体型会不会被压住
    /// 还要看演化（`tests/regression.rs` 里那条），但扣费本身必须是精确的。
    #[test]
    fn metabolism_drains_energy_every_tick() {
        let mut w = photo_world();
        w.config.life.metabolism_scale = 0.5;
        w.organisms[0].energy = 1.0;
        let up = w.organisms[0].upkeep_cost(&w.config);
        assert!(up > 0.0, "metabolism_scale = 0.5 时 upkeep 应该大于 0");
        w.act_organism(0);
        // 顺序是「先扣代谢，再加光合」：1.0 − upkeep + 0.5（满照 × rate 0.5）
        let expected = 1.0 - up + 0.5;
        assert!(
            (w.organisms[0].energy - expected).abs() < 1e-9,
            "能量应为 {:.6}（1.0 − {:.6} + 0.5），实测 {:.6}",
            expected,
            up,
            w.organisms[0].energy
        );
    }

    /// 维持成本必须真的记进热池 —— 代谢热也是"离开生物圈"，不能凭空消失。
    ///
    /// 这是批次 10 守恒审计的地基：如果 upkeep 只是把能量减掉而不记账，
    /// `生物总量 + 热池` 就会随时间单调下沉，账永远对不上。
    #[test]
    fn upkeep_is_paid_into_the_heat_pool() {
        let (mut w, _) = tiny_world(&[(ct::MOUTH, 0, 0)]);
        w.config.life.metabolism_scale = 1.0;
        w.organisms[0].energy = 1.0;
        let up = w.organisms[0].upkeep_cost(&w.config);
        assert!((up - 0.010).abs() < 1e-9, "嘴的出厂开销应是 0.010，实测 {}", up);
        w.act_organism(0);
        assert!(
            (w.heat_pool - up).abs() < 1e-9,
            "付出的 {:.4} 应该进热池，实测 {:.4}",
            up,
            w.heat_pool
        );
    }

    /// 付不出全款时，**记的是实际付出**（身上有多少付多少），不是名义开销。
    ///
    /// 否则每 tick 都会往热池里灌一笔"凭空的能量"，审计照样对不上。
    #[test]
    fn upkeep_is_capped_by_the_energy_on_hand() {
        let (mut w, _) = tiny_world(&[(ct::MOUTH, 0, 0)]);
        w.config.life.metabolism_scale = 1.0;
        w.organisms[0].energy = 0.004; // 付不出 0.010
        w.act_organism(0);
        assert_eq!(w.organisms[0].energy, 0.0, "能量不该变成负数");
        assert!(
            (w.heat_pool - 0.004).abs() < 1e-9,
            "只该记 0.004，实测 {:.4}",
            w.heat_pool
        );
        assert_eq!(w.organisms[0].starve_ticks, 1, "付不出钱要开始计饥饿");
    }

    /// `metabolism_scale = 0` 等于关掉代谢：一分不扣，热池不动。
    ///
    /// 能量取 0.5 而不是 1.0：单格身体的繁殖阈值正好是 1.0，
    /// 给 1.0 会直接触发繁殖并扣掉一份成本（`act_organism` 末尾含繁殖判定）。
    #[test]
    fn zero_metabolism_pays_nothing() {
        let (mut w, _) = tiny_world(&[(ct::MOUTH, 0, 0)]);
        w.config.life.metabolism_scale = 0.0;
        w.organisms[0].energy = 0.5;
        w.act_organism(0);
        assert_eq!(w.organisms[0].energy, 0.5);
        assert_eq!(w.heat_pool, 0.0);
    }

    /// 能量不能超过容量——光合直充体内之后必须有上限，否则无限膨胀。
    #[test]
    fn capacity_limits_energy() {
        let mut w = photo_world();
        w.config.life.photosynthesis_rate = 100.0; // 一次就撑爆
        w.act_organism(0);
        // 容量 = 2 格 × 2.0 + 1 储食 × 6.0 = 10.0
        let cap = w.energy_capacity(0);
        assert!((cap - 10.0).abs() < 1e-9, "容量应为 10.0，实测 {}", cap);
        assert!(
            (w.organisms[0].energy - cap).abs() < 1e-9,
            "能量应被夹在容量，实测 {}",
            w.organisms[0].energy
        );
        assert!(
            (w.energy_wasted - (100.0 - cap)).abs() < 1e-6,
            "超出容量的部分应记入 energy_wasted，实测 {}",
            w.energy_wasted
        );
    }

    /// 储食细胞提供额外容量（电池）。
    #[test]
    fn storage_cells_add_capacity() {
        let (w1, _) = tiny_world(&[(ct::PRODUCER, 0, 0)]);
        let (w2, _) = tiny_world(&[(ct::PRODUCER, 0, 0), (ct::STORAGE, 1, 0)]);
        let c1 = w1.energy_capacity(0);
        let c2 = w2.energy_capacity(0);
        let per = w1.config.life.energy_per_cell;
        let extra = w1.config.life.storage_capacity;
        assert!((c1 - per).abs() < 1e-9, "1 格身体容量 = energy_per_cell");
        assert!(
            (c2 - (2.0 * per + extra)).abs() < 1e-9,
            "2 格（含 1 储食）容量 = 2×{} + {}，实测 {}",
            per,
            extra,
            c2
        );
    }

    /// 再生细胞的「再生速率」要乘在每 tick 抹掉的伤害上。
    #[test]
    fn regen_mult_scales_damage_removal() {
        let run = |mult: f64| -> f64 {
            let (mut w, _) = tiny_world(&[(ct::MOUTH, 0, 0), (ct::REGEN, 1, 0)]);
            w.config.life.regen_rate = 0.1;
            w.config.cells[ct::REGEN as usize].regen_mult = mult;
            w.organisms[0].damage = 1.0;
            w.act_organism(0);
            1.0 - w.organisms[0].damage
        };
        assert!((run(1.0) - 0.1).abs() < 1e-9, "默认速率应抹掉 0.1");
        assert!((run(3.0) - 0.3).abs() < 1e-9, "倍率 3.0 应抹掉 0.3");
        assert_eq!(run(0.0), 0.0, "倍率 0 应该完全不再生");
    }

    /// 装甲的「伤害减免」要真的作用在受到的伤害上。
    #[test]
    fn armor_resist_reduces_incoming_damage() {
        // 攻击者带杀手在 (3,3)，目标在 (4,3) 且那一格是装甲
        let run = |resist: f64| -> f64 {
            let (mut w, _) = tiny_world(&[(ct::KILLER, 0, 0)]);
            w.config.cells[ct::KILLER as usize].damage = 1.0;
            w.config.cells[ct::ARMOR as usize].armor_resist = resist;

            let mut rng = Rng::new(2);
            let mut victim = Organism::new(&w.config, &mut rng, &w.grid, 4, 3);
            victim.anatomy.add_cell(ct::ARMOR, 0, 0, 0);
            victim.anatomy.recompute();
            w.add_organism(victim);

            w.act_organism(0); // 杀手出手
            w.organisms[1].damage
        };
        assert_eq!(run(1.0), 0.0, "减免 100%（出厂值）应完全免疫");
        assert!(
            (run(0.5) - 0.5).abs() < 1e-9,
            "减免 50% 时 1.0 伤害应只进 0.5（实际 {}）",
            run(0.5)
        );
        assert!(
            (run(0.0) - 1.0).abs() < 1e-9,
            "减免 0% 时应吃满 1.0 伤害（实际 {}）",
            run(0.0)
        );
    }

    /// 储食细胞的「储备倍率」要乘在繁殖阈值上。
    #[test]
    fn storage_mult_scales_reproduce_threshold() {
        let run = |mult: f64| -> f64 {
            let (mut w, _) = tiny_world(&[(ct::MOUTH, 0, 0), (ct::STORAGE, 1, 0)]);
            w.config.life.storage_food_bonus = 1.0;
            w.config.cells[ct::STORAGE as usize].storage_mult = mult;
            w.organisms[0].reproduce_at(&w.config)
        };
        // 基础成本 = 2 格，1 个储食细胞 → base * (1 + 1 * bonus)
        assert!((run(1.0) - 4.0).abs() < 1e-9, "默认应为 2 * (1+1) = 4");
        assert!((run(2.0) - 6.0).abs() < 1e-9, "倍率 2.0 应为 2 * (1+2) = 6");
        assert!(
            (run(0.0) - 2.0).abs() < 1e-9,
            "倍率 0 应退回基础成本（等于没有储食加成）"
        );
    }

    // ────────────── 批次 11：体型经济学（Kleiber / 最小体型 / 繁殖收费）──────────────

    /// `upkeep_exponent = 1.0` 必须**逐位**等于旧行为。
    ///
    /// 这是默认值，一旦漏掉那个分支，全部生态基线都会悄悄偏移。
    #[test]
    fn linear_upkeep_is_bit_identical() {
        let (w, _) = tiny_world(&[
            (ct::MOUTH, 0, 0),
            (ct::PRODUCER, 1, 0),
            (ct::PRODUCER, 0, 1),
        ]);
        let mut cfg = w.config.clone();
        cfg.life.metabolism_scale = 1.0;
        cfg.life.upkeep_exponent = 1.0;
        let got = w.organisms[0].upkeep_cost(&cfg);
        let want: f64 = w.organisms[0]
            .anatomy
            .cells
            .iter()
            .map(|c| cfg.attr(c.type_id, AttrField::Cost))
            .sum();
        assert!(
            (got - want).abs() < 1e-12,
            "线性模式应该就是 cost 之和：{} vs {}",
            got,
            want
        );
    }

    /// Kleiber（`upkeep_exponent < 1`）：**单位细胞**的开销随体型下降，
    /// 但**总**开销仍然上升。
    ///
    /// 这两条缺一不可：
    /// - 只断言"总开销上升" → 分不清它和线性模式；
    /// - 只断言"单位开销下降" → 分不清它和"越长大越省钱"（那会让体型无限膨胀）。
    #[test]
    fn kleiber_lowers_cost_per_cell_but_raises_total() {
        let (w1, _) = tiny_world(&[(ct::MOUTH, 0, 0)]);
        let (w3, _) = tiny_world(&[
            (ct::MOUTH, 0, 0),
            (ct::PRODUCER, 1, 0),
            (ct::PRODUCER, 0, 1),
        ]);
        let mut cfg = w1.config.clone();
        cfg.life.metabolism_scale = 1.0;
        cfg.life.upkeep_exponent = 0.75;

        let c1 = w1.organisms[0].upkeep_cost(&cfg);
        let c3 = w3.organisms[0].upkeep_cost(&cfg);
        assert!(
            c3 / 3.0 < c1,
            "3 格的单位细胞开销应该更低：{} / 3 vs {}",
            c3,
            c1
        );
        assert!(c3 > c1, "总开销应该随体型上升：{} vs {}", c3, c1);
    }

    /// 体型小于 `min_breed_cells` 的个体不能繁殖，而且**任何模式都不收费**。
    ///
    /// "没资格"和"尝试失败"是两件事：前者不该罚钱，否则一个 1 格个体
    /// 攒够能量就会被反复扣到 0，永远长不大。
    #[test]
    fn small_bodies_cannot_breed() {
        let (mut w, _) = tiny_world(&[(ct::MOUTH, 0, 0)]);
        w.config.life.metabolism_scale = 0.0;
        w.config.life.min_breed_cells = 3;
        w.config.life.repro_fail_costs_energy = false;
        w.organisms[0].energy = 10.0;
        let before = w.organisms.len();
        w.act_organism(0);
        assert_eq!(w.organisms.len(), before, "1 格个体不该产生后代");
        assert_eq!(w.repro_blocked_too_small, 1, "应该被体型门槛挡住");
        assert!(
            (w.organisms[0].energy - 10.0).abs() < 1e-12,
            "被体型门槛挡住不该扣钱，实测剩 {}",
            w.organisms[0].energy
        );
    }

    /// `repro_fail_costs_energy` 的两面：失败时严格模式付全款、宽松模式分文不取。
    ///
    /// 用 `max_organisms = 1` 制造一次**必然失败**的繁殖（个体数到顶），
    /// 这样两条路径都只走 `can_add_organism` 那个分支，不受地形影响。
    #[test]
    fn failed_reproduction_only_charges_in_strict_mode() {
        let run = |strict: bool| -> (f64, f64) {
            let (mut w, _) = tiny_world(&[(ct::MOUTH, 0, 0)]);
            w.config.life.metabolism_scale = 0.0;
            w.config.life.repro_fail_costs_energy = strict;
            w.config.life.max_organisms = 1;
            w.organisms[0].energy = 10.0;
            w.act_organism(0);
            assert_eq!(w.repro_blocked_cap, 1, "应该卡在个体上限");
            (w.organisms[0].energy, w.heat_pool)
        };
        let (e_strict, heat_strict) = run(true);
        let (e_free, heat_free) = run(false);
        assert!(e_strict < 10.0, "严格模式失败要付全款，实测剩 {}", e_strict);
        assert!(heat_strict > 0.0, "付出去的钱要进热池");
        assert!(
            (e_free - 10.0).abs() < 1e-12,
            "宽松模式失败不该扣钱，实测剩 {}",
            e_free
        );
        assert_eq!(heat_free, 0.0);
    }

    // ────────────── 批次 11：环境温度（表面积/体积比）──────────────

    /// `temp_damage = 0` 时温度完全不参与模拟：默认值必须逐位等于旧行为。
    #[test]
    fn zero_temp_damage_is_a_no_op() {
        let (mut w, _) = tiny_world(&[(ct::MOUTH, 0, 0)]);
        w.config.life.temp_damage = 0.0;
        for t in w.fields.temp.iter_mut() {
            *t = 255;
        }
        w.apply_heat_damage(0);
        assert_eq!(w.organisms[0].damage, 0.0);
    }

    /// 舒适温度以下不受伤害；以上才开始掉血。
    ///
    /// 现在温度是**摄氏度**：局部温度 = `环境温度 + 场值/255 × 量程`。
    #[test]
    fn heat_only_hurts_above_the_comfort_threshold() {
        let run = |temp: u8| -> f64 {
            let (mut w, _) = tiny_world(&[(ct::MOUTH, 0, 0)]);
            w.config.life.temp_damage = 1.0;
            w.config.world.ambient_temp = 22.0;
            w.config.world.temp_span = 100.0;
            w.config.life.comfort_temp = 72.0; // = 22 + 50% 量程
            for t in w.fields.temp.iter_mut() {
                *t = temp;
            }
            w.apply_heat_damage(0);
            w.organisms[0].damage
        };
        assert_eq!(run(0), 0.0, "22℃ 低于舒适温度 72℃");
        assert_eq!(run(127), 0.0, "22 + 49.8 = 71.8℃，仍低于 72℃");
        assert!(run(200) > 0.0, "22 + 78.4 = 100.4℃，应该开始受伤");
        // 越热越疼（线性：excess 从 0.28 到 0.5）
        assert!(
            run(255) > run(200) * 1.5,
            "122℃ 应该明显比 100℃ 疼：{} vs {}",
            run(255),
            run(200)
        );
    }

    /// **环境温度是全局旋钮**：调高它，连一格热源都没有的常温区也会开始扣血。
    ///
    /// 这一条钉的是"世界有基础温度"这件事本身——温度场存的是温差，
    /// 绝对温度 = 环境温度 + 温差，所以环境温度一涨，全场都跟着涨。
    #[test]
    fn raising_ambient_temperature_hurts_everywhere() {
        let run = |ambient: f64| -> f64 {
            let (mut w, _) = tiny_world(&[(ct::MOUTH, 0, 0)]);
            w.config.life.temp_damage = 1.0;
            w.config.world.ambient_temp = ambient;
            w.config.world.temp_span = 100.0;
            w.config.life.comfort_temp = 35.0;
            // 场值全 0：世界里没有任何热源
            for t in w.fields.temp.iter_mut() {
                *t = 0;
            }
            w.apply_heat_damage(0);
            w.organisms[0].damage
        };
        assert_eq!(run(22.0), 0.0, "22℃ 是常温，不该受伤");
        assert_eq!(run(35.0), 0.0, "刚好等于舒适温度，仍不受伤");
        assert!(run(45.0) > 0.0, "45℃ 超过舒适温度，常温区也该受伤");
    }

    /// **核心断言**：单位血量的伤害率随体型下降 —— 大个体在热区活得更久。
    ///
    /// 伤害 ∝ `n^temp_size_exponent`（默认 -0.25），血量 = n，
    /// 所以生存时间 ∝ `n^1.25`。
    #[test]
    fn big_bodies_take_less_heat_per_hp() {
        let (mut w1, _) = tiny_world(&[(ct::MOUTH, 0, 0)]);
        let (mut w3, _) = tiny_world(&[
            (ct::MOUTH, 0, 0),
            (ct::PRODUCER, 1, 0),
            (ct::PRODUCER, 0, 1),
        ]);
        for w in [&mut w1, &mut w3] {
            w.config.life.temp_damage = 1.0;
            w.config.life.comfort_temp = 0.0;
            w.config.life.temp_size_exponent = -0.25;
            for t in w.fields.temp.iter_mut() {
                *t = 255;
            }
        }
        w1.apply_heat_damage(0);
        w3.apply_heat_damage(0);
        let hp1 = w1.organisms[0].max_health(&w1.config);
        let hp3 = w3.organisms[0].max_health(&w3.config);
        let rate1 = w1.organisms[0].damage / hp1;
        let rate3 = w3.organisms[0].damage / hp3;
        assert!(
            rate3 < rate1,
            "3 格的单位血量伤害率应该更低：{:.4} vs {:.4}",
            rate3,
            rate1
        );
        // 而且必须是"温和地更低"——不是免疫
        assert!(rate3 > rate1 * 0.1, "3 格不该几乎免疫：{:.4}", rate3);
    }

    /// 身体只要有一角伸进热区，整只就要承受 —— 取的是最热的那一格。
    #[test]
    fn the_hottest_cell_decides() {
        let (mut w, _) = tiny_world(&[(ct::MOUTH, 0, 0), (ct::PRODUCER, 1, 0)]);
        w.config.life.temp_damage = 1.0;
        w.config.life.comfort_temp = 0.0;
        // 只有第二个细胞所在的格子是热的
        for t in w.fields.temp.iter_mut() {
            *t = 0;
        }
        let org = &w.organisms[0];
        let cell = org.anatomy.cells[1];
        let (c, r) = (org.real_col(&cell), org.real_row(&cell));
        let idx = w.grid.index(c, r).unwrap();
        w.fields.temp[idx] = 255;
        w.apply_heat_damage(0);
        assert!(w.organisms[0].damage > 0.0, "一根手指伸进热区也要疼");
    }

    /// **温度视图必须显示绝对温度，不是温差。**
    ///
    /// 这个 bug 是用户报的：「世界温度调成 30℃ 了，但温度视图里看还是完全漆黑」。
    /// 根因是着色时直接把 `fields.temp`（温差）传给了渲染层——30℃ 环境里
    /// 没有任何热源，全场温差都是 0，于是整幅画面纯黑。
    #[test]
    fn temp_view_shows_absolute_temperature_not_delta() {
        let (mut w, _) = tiny_world(&[(ct::MOUTH, 0, 0)]);
        w.config.world.ambient_temp = 30.0;
        w.config.world.temp_span = 100.0;
        // 场值全 0：世界里一格热源都没有
        for t in w.fields.temp.iter_mut() {
            *t = 0;
        }
        let v = w.temp_view();
        // 30℃ / 120℃ × 255 ≈ 63
        assert!(
            v.iter().all(|&x| (60..=66).contains(&x)),
            "常温区应该显示约 64（30℃），实测 {}",
            v[0]
        );
        assert!(v[0] > 40, "不能是纯黑——这正是用户看到的 bug");
    }

    /// 环境温度越高，同一个温差格子在视图上越亮。
    #[test]
    fn temp_view_brightens_with_ambient() {
        let build = |ambient: f64| -> u8 {
            let (mut w, _) = tiny_world(&[(ct::MOUTH, 0, 0)]);
            w.config.world.ambient_temp = ambient;
            w.config.world.temp_span = 100.0;
            for t in w.fields.temp.iter_mut() {
                *t = 0;
            }
            w.temp_view()[0]
        };
        assert!(
            build(40.0) > build(22.0),
            "环境 40℃ 应该比 22℃ 更亮：{} vs {}",
            build(40.0),
            build(22.0)
        );
    }

    /// 岩浆（场值满）在任何环境温度下都顶到最亮。
    #[test]
    fn temp_view_saturates_at_lava() {
        let (mut w, _) = tiny_world(&[(ct::MOUTH, 0, 0)]);
        w.config.world.ambient_temp = 22.0;
        w.config.world.temp_span = 100.0;
        w.fields.temp[0] = 255; // 22 + 100 = 122℃ > 120℃ 上限
        assert_eq!(w.temp_view()[0], 255, "岩浆应该顶满");
    }

    // ────────────── 批次 12：成长过程（芽 → 成体）──────────────

    /// 芽长一格：体型 +1、扣 `growth_cost`、差额进热池；长满了自动成年。
    #[test]
    fn growing_adds_a_cell_and_charges_energy() {
        let (mut w, _) = tiny_world(&[(ct::MOUTH, 0, 0)]);
        w.config.life.growth_enabled = true;
        // 这几条量的是细胞层面的机制（长一格 / 血量 / 兜底），不是年龄门槛，
        // 所以显式把最短成长期关掉，否则"出生即成年"的前提不成立。
        w.config.life.growth_min_ticks = 0;
        w.config.life.growth_cost = 1.0;
        let mut plan = w.organisms[0].anatomy.clone();
        plan.add_cell(ct::PRODUCER, 1, 0, 0);
        w.organisms[0].growth_plan = Some(plan);
        w.organisms[0].energy = 5.0;

        w.grow_organism(0);
        assert_eq!(w.organisms[0].cell_count(), 2, "应该长到 2 格");
        assert!(
            (w.organisms[0].energy - 4.0).abs() < 1e-9,
            "应该扣 1 能量，实测剩 {}",
            w.organisms[0].energy
        );
        assert!((w.heat_pool - 1.0).abs() < 1e-9, "生长耗能要进热池");
        assert!(
            w.organisms[0].growth_plan.is_none(),
            "长满目标体型之后应该成年"
        );
    }

    /// **未成年不能繁殖**——这是整个成长机制的意义所在。
    #[test]
    fn a_sprout_cannot_breed() {
        let (mut w, _) = tiny_world(&[(ct::MOUTH, 0, 0)]);
        w.config.life.growth_enabled = true;
        // 这几条量的是细胞层面的机制（长一格 / 血量 / 兜底），不是年龄门槛，
        // 所以显式把最短成长期关掉，否则"出生即成年"的前提不成立。
        w.config.life.growth_min_ticks = 0;
        w.config.life.metabolism_scale = 0.0;
        // 目标 2 格，但生长成本高到这一 tick 长不动
        let mut plan = w.organisms[0].anatomy.clone();
        plan.add_cell(ct::PRODUCER, 1, 0, 0);
        w.organisms[0].growth_plan = Some(plan);
        w.config.life.growth_cost = 100.0;
        w.organisms[0].energy = 10.0;

        let before = w.organisms.len();
        w.act_organism(0);
        assert_eq!(
            w.organisms.len(),
            before,
            "长不动的芽攒再多能量也不该繁殖"
        );
    }

    /// 芽的血量按**目标蓝图**算，不是按当前格数。
    ///
    /// 如果按当前格数，1 格的芽就是 1 HP，会被捕食者一口一个——
    /// 实测那种设定下杀手占比从 14% 飙到 50%+，两个种子直接灭绝。
    #[test]
    fn a_sprout_has_the_health_of_its_target() {
        let (mut w, _) = tiny_world(&[(ct::MOUTH, 0, 0)]);
        let mut plan = w.organisms[0].anatomy.clone();
        plan.add_cell(ct::PRODUCER, 1, 0, 0);
        plan.add_cell(ct::PRODUCER, 0, 1, 0);
        w.organisms[0].growth_plan = Some(plan);
        let hp = w.organisms[0].max_health(&w.config);
        assert!(
            (hp - 3.0).abs() < 1e-9,
            "3 格目标的芽应该有 3 HP，实测 {}",
            hp
        );
    }

    /// 目标格全被占时，芽要能**退化成长到相邻空位**，而不是永远卡在 1 格。
    ///
    /// 这条是实测救回来的：严格按蓝图长的话，芽生在母体旁边那片拥挤区域，
    /// 目标格大概率被邻居占着，于是永远长不大、永远不能繁殖——1002/1003 直接灭绝。
    #[test]
    fn growth_falls_back_to_any_free_neighbour() {
        let (mut w, _) = tiny_world(&[(ct::MOUTH, 0, 0)]);
        w.config.life.growth_enabled = true;
        // 这几条量的是细胞层面的机制（长一格 / 血量 / 兜底），不是年龄门槛，
        // 所以显式把最短成长期关掉，否则"出生即成年"的前提不成立。
        w.config.life.growth_min_ticks = 0;
        w.config.life.growth_cost = 1.0;
        let mut plan = w.organisms[0].anatomy.clone();
        plan.add_cell(ct::PRODUCER, 1, 0, 0);
        w.organisms[0].growth_plan = Some(plan);
        w.organisms[0].energy = 5.0;
        // 把蓝图指定的那一格 (1,0) 用墙堵死
        let org = &w.organisms[0];
        let (c, r) = (org.col + 1, org.row);
        let idx = w.grid.index(c, r).unwrap();
        w.set_cell_at(idx, ct::WALL, -1);

        w.grow_organism(0);
        assert_eq!(
            w.organisms[0].cell_count(),
            2,
            "目标格被堵死时应该退化长到别的相邻空位"
        );
    }

    /// **环境可塑**：芽长不动够久就提前成熟，而且体型比目标小。
    ///
    /// 这条钉的是 r/K 分化的机制本身：食物充足时长满（K），
    /// 稀缺/拥挤时提前成熟成小体型（r）。
    #[test]
    fn a_stalled_sprout_matures_early_and_stays_small() {
        let (mut w, _) = tiny_world(&[(ct::MOUTH, 0, 0)]);
        w.config.life.growth_enabled = true;
        // 这几条量的是细胞层面的机制（长一格 / 血量 / 兜底），不是年龄门槛，
        // 所以显式把最短成长期关掉，否则"出生即成年"的前提不成立。
        w.config.life.growth_min_ticks = 0;
        w.config.life.growth_cost = 1.0;
        w.config.life.growth_stall_mature = 3;
        // 目标 3 格，但一格都不给它长（能量 0）
        let mut plan = w.organisms[0].anatomy.clone();
        plan.add_cell(ct::PRODUCER, 1, 0, 0);
        plan.add_cell(ct::PRODUCER, 0, 1, 0);
        w.organisms[0].growth_plan = Some(plan);
        w.organisms[0].energy = 0.0;

        for _ in 0..3 {
            w.grow_organism(0);
        }
        assert!(
            w.organisms[0].growth_plan.is_none(),
            "卡满 3 tick 应该提前成熟"
        );
        assert_eq!(
            w.organisms[0].cell_count(),
            1,
            "提前成熟的体型应该停在 1 格，而不是长到目标的 3 格"
        );
    }

    /// 阈值设为 0 时**永远不提前成熟**：必须长满，保持旧行为。
    #[test]
    fn zero_stall_threshold_never_matures_early() {
        let (mut w, _) = tiny_world(&[(ct::MOUTH, 0, 0)]);
        w.config.life.growth_enabled = true;
        // 这几条量的是细胞层面的机制（长一格 / 血量 / 兜底），不是年龄门槛，
        // 所以显式把最短成长期关掉，否则"出生即成年"的前提不成立。
        w.config.life.growth_min_ticks = 0;
        w.config.life.growth_stall_mature = 0;
        let mut plan = w.organisms[0].anatomy.clone();
        plan.add_cell(ct::PRODUCER, 1, 0, 0);
        w.organisms[0].growth_plan = Some(plan);
        w.organisms[0].energy = 0.0;
        for _ in 0..100 {
            w.grow_organism(0);
        }
        assert!(
            w.organisms[0].growth_plan.is_some(),
            "阈值 0 时不该提前成熟"
        );
    }

    // ────────────── 批次 14：最短成长期（堵掉「1 格 = 出生即成年」）──────────────

    /// 1 格目标的芽在满 `growth_min_ticks` 之前**不能成年**。
    ///
    /// 这条是整批的核心。`growth_plan` 是母体成熟时的体型，所以 1 格母体的后代
    /// 目标就是 1 格——没有年龄门槛的话它出生那一 tick 就"长满了"，
    /// 立刻成年、立刻繁殖。实测那会让整个种群塌到平均 1.69 格，
    /// 而加上 20 tick 的门槛能到 4~8 格。
    #[test]
    fn a_one_cell_sprout_must_wait_the_minimum_time() {
        let (mut w, _) = tiny_world(&[(ct::MOUTH, 0, 0)]);
        w.config.life.growth_enabled = true;
        w.config.life.growth_min_ticks = 5;
        w.config.life.growth_cost = 0.0;
        // 1 格目标：出生就已经"长满"
        w.organisms[0].growth_plan = Some(w.organisms[0].anatomy.clone());

        for tick in 1..5 {
            w.grow_organism(0);
            assert!(
                w.organisms[0].growth_plan.is_some(),
                "第 {} tick 就成年了，应该等到第 5 tick",
                tick
            );
        }
        w.grow_organism(0);
        assert!(
            w.organisms[0].growth_plan.is_none(),
            "满 5 tick 应该成年"
        );
    }

    /// `growth_min_ticks = 0` 时**逐位等于旧行为**：1 格目标当 tick 成年。
    #[test]
    fn zero_min_time_matures_immediately() {
        let (mut w, _) = tiny_world(&[(ct::MOUTH, 0, 0)]);
        w.config.life.growth_enabled = true;
        w.config.life.growth_min_ticks = 0;
        w.config.life.growth_cost = 0.0;
        w.organisms[0].growth_plan = Some(w.organisms[0].anatomy.clone());
        w.grow_organism(0);
        assert!(w.organisms[0].growth_plan.is_none(), "0 应该立刻成年");
    }

    /// 年龄门槛**不阻止长身体**——它只管"什么时候算成年"。
    ///
    /// 如果实现成"不够年龄就不长"，大体型就永远长不出来，整批白做。
    #[test]
    fn the_minimum_time_does_not_block_growing() {
        let (mut w, _) = tiny_world(&[(ct::MOUTH, 0, 0)]);
        w.config.life.growth_enabled = true;
        w.config.life.growth_min_ticks = 10;
        w.config.life.growth_cost = 1.0;
        let mut plan = w.organisms[0].anatomy.clone();
        plan.add_cell(ct::PRODUCER, 1, 0, 0);
        w.organisms[0].growth_plan = Some(plan);
        w.organisms[0].energy = 5.0;

        w.grow_organism(0);
        assert_eq!(w.organisms[0].cell_count(), 2, "第一 tick 就该长出第二格");
        assert!(
            w.organisms[0].growth_plan.is_some(),
            "长满了但还不够年龄，仍算未成年"
        );
    }

    /// 「长不动就提前成熟」这条路径**也要过年龄门槛**。
    ///
    /// 否则最短成长期会被绕过去——而"长不动"正好是最需要它的场景
    /// （贫斑块里芽长不动，本来会立刻变成小体型）。
    #[test]
    fn early_maturity_also_respects_the_minimum_time() {
        let (mut w, _) = tiny_world(&[(ct::MOUTH, 0, 0)]);
        w.config.life.growth_enabled = true;
        w.config.life.growth_min_ticks = 10;
        w.config.life.growth_stall_mature = 1; // 卡一 tick 就该成熟
        w.config.life.growth_cost = 1.0;
        let mut plan = w.organisms[0].anatomy.clone();
        plan.add_cell(ct::PRODUCER, 1, 0, 0);
        plan.add_cell(ct::PRODUCER, 0, 1, 0);
        w.organisms[0].growth_plan = Some(plan);
        w.organisms[0].energy = 0.0; // 长不动

        for tick in 1..10 {
            w.grow_organism(0);
            assert!(
                w.organisms[0].growth_plan.is_some(),
                "第 {} tick 就提前成熟了，应该等到第 10 tick",
                tick
            );
        }
        w.grow_organism(0);
        assert!(
            w.organisms[0].growth_plan.is_none(),
            "满 10 tick 之后才允许提前成熟"
        );
        assert_eq!(w.organisms[0].cell_count(), 1, "提前成熟的体型停在 1 格");
    }

    // ────────────── 批次 15：出芽（芽挂在母体上）──────────────

    /// 芽位随体型涨，而且**没有下限**：不到门槛就退回普通繁殖。
    ///
    /// 下限那一版（`max(1)`）让 1 格身体也白拿一个芽位，实测把体型压到 1.85；
    /// 现在只有够大的身体才出芽，这也更符合生物学（单细胞是分裂，不是出芽）。
    #[test]
    fn bud_slots_scale_with_body_size() {
        let (w1, _) = tiny_world(&[(ct::MOUTH, 0, 0)]);
        let (w4, _) = tiny_world(&[
            (ct::MOUTH, 0, 0),
            (ct::PRODUCER, 1, 0),
            (ct::PRODUCER, 0, 1),
            (ct::PRODUCER, 1, 1),
        ]);
        for w in [&w1, &w4] {
            assert_eq!(w.bud_slots(0), 0, "4 格以下（budper = 8）不该有芽位");
        }
        let mut cfg = Config::default();
        cfg.life.bud_cells_per_slot = 2.0;
        let (mut w4, _) = tiny_world(&[
            (ct::MOUTH, 0, 0),
            (ct::PRODUCER, 1, 0),
            (ct::PRODUCER, 0, 1),
            (ct::PRODUCER, 1, 1),
        ]);
        w4.config = cfg;
        assert_eq!(w4.bud_slots(0), 2, "4 格 / 每芽位 2 格 = 2 个芽位");
    }

    /// 芽生**紧贴母体**，而不是 4~6 格外的空地。
    #[test]
    fn a_bud_is_born_next_to_its_parent() {
        let (mut w, _) = tiny_world(&[
            (ct::MOUTH, 0, 0),
            (ct::PRODUCER, 1, 0),
            (ct::PRODUCER, 0, 1),
            (ct::PRODUCER, 1, 1),
        ]);
        w.config.life.budding_enabled = true;
        w.config.life.growth_enabled = true;
        w.config.life.bud_cells_per_slot = 2.0;
        w.config.life.bud_start_cost = 1.0;
        w.organisms[0].energy = 10.0;
        let before = w.organisms.len();

        assert!(w.reproduce_bud(0), "应该产出一个芽");
        assert_eq!(w.organisms.len(), before + 1);
        let child = &w.organisms[before];
        assert_eq!(child.parent_id, Some(w.organisms[0].id), "要记下母体");
        assert!(child.growth_plan.is_some(), "芽必须是未成熟的");

        let parent = &w.organisms[0];
        let adjacent = parent.anatomy.cells.iter().any(|c| {
            let (pc, pr) = (parent.real_col(c), parent.real_row(c));
            (pc - child.col).abs() + (pr - child.row).abs() == 1
        });
        assert!(adjacent, "芽必须紧贴母体的某一格");
    }

    /// 产芽的成本是**固定的** `bud_start_cost`，不随体型涨。
    ///
    /// 这是超线性的来源：普通繁殖收 `food_needed = 细胞数`，
    /// 4 格母体要付 4；出芽只付 1。
    #[test]
    fn budding_charges_a_fixed_cost() {
        let (mut w, _) = tiny_world(&[
            (ct::MOUTH, 0, 0),
            (ct::PRODUCER, 1, 0),
            (ct::PRODUCER, 0, 1),
            (ct::PRODUCER, 1, 1),
        ]);
        w.config.life.budding_enabled = true;
        w.config.life.growth_enabled = true;
        w.config.life.bud_cells_per_slot = 2.0;
        w.config.life.bud_start_cost = 1.0;
        w.organisms[0].energy = 10.0;
        let need = w.organisms[0].food_needed(&w.config);
        assert!(need > 1.5, "普通繁殖应该贵得多（实测 {}）", need);

        w.reproduce_bud(0);
        assert!(
            (w.organisms[0].energy - 9.0).abs() < 1e-9,
            "出芽应该只扣 1.0，实测剩 {}",
            w.organisms[0].energy
        );
    }

    /// 母体每 tick 喂芽一份能量，而且是**内部转移**——总能量不变。
    #[test]
    fn a_bud_draws_energy_from_its_parent() {
        let (mut w, _) = tiny_world(&[
            (ct::MOUTH, 0, 0),
            (ct::PRODUCER, 1, 0),
            (ct::PRODUCER, 0, 1),
            (ct::PRODUCER, 1, 1),
        ]);
        w.config.life.budding_enabled = true;
        w.config.life.growth_enabled = true;
        w.config.life.bud_cells_per_slot = 2.0;
        w.config.life.bud_start_cost = 1.0;
        w.config.life.bud_feed = 0.5;
        w.organisms[0].energy = 10.0;
        w.reproduce_bud(0);

        let ci = w.organisms.len() - 1;
        let pid = w.organisms[ci].parent_id.unwrap();
        let before_parent = w.organisms[0].energy;
        let before_child = w.organisms[ci].energy;
        w.feed_from_parent(ci, pid);
        let gave = before_parent - w.organisms[0].energy;
        assert!((gave - 0.5).abs() < 1e-9, "应该喂 0.5，实测 {}", gave);
        assert!(
            (w.organisms[ci].energy - (before_child + 0.5)).abs() < 1e-9,
            "芽应该收到 0.5"
        );
        // 内部转移：两人之和不变（这条守住"喂食不能凭空造能量"）
        let sum_before = before_parent + before_child;
        let sum_after = w.organisms[0].energy + w.organisms[ci].energy;
        assert!((sum_after - sum_before).abs() < 1e-12);
    }

    /// 没有芽位时**退回普通繁殖**，而不是绝育。
    ///
    /// 第一版"出芽开着就一律走芽"，于是 `bud_slots = 0` 的小个体再也生不了——
    /// `bud_cells_per_slot >= 4` 时世界当场全灭（实测 0 个体）。
    #[test]
    fn bodies_without_a_slot_fall_back_to_normal_reproduction() {
        let (mut w, _) = tiny_world(&[(ct::MOUTH, 0, 0)]);
        w.config.life.budding_enabled = true;
        w.config.life.growth_enabled = true;
        w.config.life.bud_cells_per_slot = 8.0;
        w.config.life.metabolism_scale = 0.0;
        w.organisms[0].energy = 50.0;
        assert_eq!(w.bud_slots(0), 0, "1 格没有芽位");

        let before = w.organisms.len();
        // 普通繁殖要 4~6 格外的空地；8x8 的测试世界放得下
        let ok = w.reproduce_organism(0);
        assert!(ok, "没有芽位时应该走普通繁殖");
        assert_eq!(w.organisms.len(), before + 1);
        let child = &w.organisms[before];
        assert_eq!(child.parent_id, None, "普通繁殖的后代不该挂靠母体");
    }

    // ────────────── 批次 16：整口咬合（击杀时间 = 目标血量 / 咬合力）──────────────

    /// 造一个「决斗场」：捕食者放在 pc，猎物放在 vc，两边身体形状自己给。
    fn arena(
        pred: &[(u8, i32, i32)],
        pc: (i32, i32),
        prey: &[(u8, i32, i32)],
        vc: (i32, i32),
    ) -> (World, usize, usize) {
        let mut cfg = Config::default();
        cfg.world.cols = 16;
        cfg.world.rows = 16;
        cfg.life.metabolism_scale = 0.0;
        let mut w = World::new(cfg, Rng::new(1));
        w.fill_grid(ct::EMPTY, false);
        let mut rng = Rng::new(1);

        let mut a = Organism::new(&w.config, &mut rng, &w.grid, pc.0, pc.1);
        for (t, dx, dy) in pred {
            a.anatomy.add_cell(*t, *dx, *dy, 0);
        }
        a.anatomy.recompute();
        let pid = w.add_organism(a);

        let mut b = Organism::new(&w.config, &mut rng, &w.grid, vc.0, vc.1);
        for (t, dx, dy) in prey {
            b.anatomy.add_cell(*t, *dx, *dy, 0);
        }
        b.anatomy.recompute();
        let vid = w.add_organism(b);

        let pi = w.organism_index(pid).unwrap();
        let vi = w.organism_index(vid).unwrap();
        (w, pi, vi)
    }

    /// **核心断言**：击杀需要的 tick 数 = 目标血量 ÷ 咬合力。
    ///
    /// 1 格猎物 1 HP、5 格猎物 5 HP，捕食者都只有 1 个杀手细胞（咬合力 1.0），
    /// 所以应该是 1 tick 和 5 tick。这就是"体型买得到生存时间"。
    #[test]
    fn kill_time_scales_with_the_victims_health() {
        let ticks_to_kill = |prey: &[(u8, i32, i32)]| -> usize {
            let (mut w, pi, vi) = arena(&[(ct::KILLER, 0, 0)], (3, 3), prey, (4, 3));
            let mut t = 0;
            while w.organisms[vi].alive && t < 200 {
                w.act_organism(pi);
                t += 1;
            }
            t
        };
        // 竖排：只有最上面那一格贴着捕食者，所以接触面恒为 1
        let one = ticks_to_kill(&[(ct::PRODUCER, 0, 0)]);
        let five = ticks_to_kill(&[
            (ct::PRODUCER, 0, 0),
            (ct::PRODUCER, 0, 1),
            (ct::PRODUCER, 0, 2),
            (ct::PRODUCER, 0, 3),
            (ct::PRODUCER, 0, 4),
        ]);
        assert_eq!(one, 1, "1 格猎物应该 1 tick 就死，实测 {}", one);
        assert_eq!(five, 5, "5 格猎物应该 5 tick，实测 {}", five);
    }

    /// 同一个目标被同一只捕食者的**多个接触格**同时咬，伤害只算一次。
    ///
    /// 这是整批的病根：旧逻辑按接触面积叠加，20 格猎物被 3~5 格咬住，
    /// 每 tick 挨 3~5 点，20 点血几 tick 就没了 —— 体型买不到任何生存时间。
    #[test]
    fn extra_contact_cells_do_not_add_damage() {
        // 猎物是 U 形（5 格），捕食者卡在缺口里，同时贴着 3 个格子
        let prey = [
            (ct::PRODUCER, 0, 0),
            (ct::PRODUCER, 0, 1),
            (ct::PRODUCER, 0, 2),
            (ct::PRODUCER, 1, 0),
            (ct::PRODUCER, 1, 2),
        ];
        let dmg = |capped: bool| -> f64 {
            let (mut w, pi, vi) = arena(&[(ct::KILLER, 0, 0)], (5, 4), &prey, (4, 3));
            w.config.combat.killer_bite_capped = capped;
            w.act_organism(pi);
            w.organisms[vi].damage
        };
        let capped = dmg(true);
        let uncapped = dmg(false);
        assert!(
            (capped - 1.0).abs() < 1e-9,
            "整口咬合：3 个接触格也只该挨 1.0，实测 {}",
            capped
        );
        assert!(
            (uncapped - 3.0).abs() < 1e-9,
            "旧行为应该按接触面积叠加成 3.0，实测 {}",
            uncapped
        );
    }

    /// 杀手细胞越多，咬合力越大——大捕食者仍然更强，只是不再"按面积翻倍"。
    #[test]
    fn more_killer_cells_bite_harder() {
        let prey = [(ct::PRODUCER, 0, 0), (ct::PRODUCER, 0, 1), (ct::PRODUCER, 0, 2)];
        let (mut w, pi, vi) = arena(
            &[
                (ct::KILLER, 0, 0),
                (ct::KILLER, 0, 1),
                (ct::KILLER, 0, 2),
            ],
            (3, 2),
            &prey,
            (4, 3),
        );
        w.act_organism(pi);
        let dmg = w.organisms[vi].damage;
        assert!(
            (dmg - 3.0).abs() < 1e-9,
            "3 个杀手细胞应该咬 3.0，实测 {}",
            dmg
        );
    }

    /// 关掉「整口咬合」必须逐位退回旧行为。
    #[test]
    fn turning_the_cap_off_restores_the_old_rule() {
        let prey = [(ct::PRODUCER, 0, 0)];
        let (mut w, pi, vi) = arena(&[(ct::KILLER, 0, 0)], (3, 3), &prey, (4, 3));
        w.config.combat.killer_bite_capped = false;
        w.act_organism(pi);
        // 1 格猎物、1 点伤害 → 当 tick 就死
        assert!(!w.organisms[vi].alive, "关掉之后仍然是 1 tick 咬死");
        assert!(w.organisms[vi].damage >= 1.0);
    }
}
