//! 世界环境：网格 + 生物集合 + 环境规则 + 两阶段主循环。
//! 与 JS 版 `src/core/World.js` 对应。
//!
//! 借用策略：**纯计算**在 `organism.rs`，**写世界的动作**在本文件。
//! 原因是 Rust 不允许"拿着 `&mut 自己` 再去借 `&mut 世界`"，
//! 所以 act / reproduce / die / harm 都写成了 `&mut self` 的方法，按索引访问个体。

use serde::{Deserialize, Serialize};

use std::collections::HashMap;
use std::sync::Arc;

use crate::anatomy::{BodyCell, Flags};
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
            light: Light::new(config.world.cols, config.world.rows),
            energy_wasted: 0.0,
            heat_pool: 0.0,
            light_dirty: true,
            order: Vec::new(),
            config,
            rng,
        };
        w.init_food_index();
        w.init_terrain_index();
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
                self.grid.food[index] = self.config.food_energy_gain() as f32;
            }
        } else if was_food {
            self.remove_food(index);
            // 格子不再是食物 → 剩下的能量不能留着（否则以后再变成食物会"复活"）
            //
            // **但能量不能凭空消失**：`owner >= 0` 说明是一具身体盖在了食物上
            // （只可能发生在 `food_blocks_movement = false` 时）。那格食物是被
            // **压碎**的，不是被吃掉的，所以记进热池——离开生物圈，但账要对得上。
            // 嘴吃东西走的是 `reveal_floor`（owner = -1），它已经把能量记到自己身上了，
            // 所以这里必须排除掉，否则每一口都会被重复计入热池。
            if owner >= 0 {
                self.heat_pool += self.grid.food[index] as f64;
            }
            self.grid.food[index] = 0.0;
        }
    }

    pub fn set_cell(&mut self, col: i32, row: i32, ty: u8, owner: i32) -> Option<usize> {
        let index = self.grid.index(col, row)?;
        self.set_cell_at(index, ty, owner);
        Some(index)
    }

    /// 把地板重新铺回顶层（生物离开 / 食物被吃掉时用）
    /// 把一格变成食物，并指定它携带多少能量。
    ///
    /// 尸体和额外投放走这条路；普通画笔/腐殖质走 `set_cell_at` 的默认值。
    pub fn set_food_at(&mut self, index: usize, energy: f64) {
        if index >= self.grid.len {
            return;
        }
        self.set_cell_at(index, ct::FOOD, -1);
        if self.grid.types[index] == ct::FOOD {
            self.grid.food[index] = energy.max(0.0) as f32;
        }
    }

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
            let e = self.grid.food[index] as f64;
            let decay = (e * rel).max(floor).min(e);
            let left = e - decay;
            self.heat_pool += decay;
            if left <= 1e-6 {
                self.reveal_floor(index); // 会从 food_cells 里 swap-remove
            } else {
                self.grid.food[index] = left as f32;
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

        // ── 环境更新 ──
        self.apply_terrain_tick();
        self.apply_food_decay();
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
            ..
        } = self;
        organisms
            .par_iter_mut()
            .filter(|o| o.alive)
            .for_each(|o| o.think(grid, config));
    }

    /// 单个体版（测试与串行对照用）
    pub fn think_organism(&mut self, i: usize) {
        let World {
            grid,
            config,
            organisms,
            ..
        } = self;
        organisms[i].think(grid, config);
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
        self.act_organism_inner(i);
        if let Some(org) = self.organisms.get_mut(i) {
            org.energy_gained_last = org.energy - before;
        }
    }

    fn act_organism_inner(&mut self, i: usize) {
        // 年龄
        self.organisms[i].age += 1;
        let lifespan = self.organisms[i].lifespan(&self.config);
        if self.organisms[i].age as f64 > lifespan {
            self.kill_organism(i);
            return;
        }

        self.organisms[i].in_radiation = false;

        // 代谢
        let upkeep = self.organisms[i].upkeep_cost(&self.config);
        if upkeep > 0.0 {
            let limit = self.config.life.starvation_ticks;
            let mut starved = false;
            {
                let org = &mut self.organisms[i];
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

        // 繁殖
        let at = self.organisms[i].reproduce_at(&self.config);
        if self.organisms[i].energy >= at {
            self.reproduce_organism(i);
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
                    let org = &mut self.organisms[i];
                    org.energy -= cost;
                    if org.energy < 0.0 {
                        org.energy = 0.0;
                    }
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

    /// 这个个体能存多少能量（见 [`Organism::energy_capacity`]）。
    fn energy_capacity(&self, i: usize) -> f64 {
        self.organisms[i].energy_capacity(&self.config)
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
                for k in 0..4 {
                    let n = NEIGHBORS_4[k];
                    let Some(index) = self.grid.index(col + n.0, row + n.1) else {
                        continue;
                    };
                    if self.grid.types[index] == ct::FOOD {
                        let e = self.grid.food[index] as f64;
                        self.reveal_floor(index);
                        self.organisms[i].energy += e * mult;
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
                    let gain = self.bite_from(owner as u32, index, mult);
                    if gain > 0.0 {
                        self.organisms[i].energy += gain;
                    }
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
                let gain = lit
                    * self.config.life.photosynthesis_rate
                    * self.config.attr(cell.type_id, AttrField::ProdMult);
                if gain <= 0.0 {
                    return;
                }
                // 容量上限：光合直充体内之后必须有，否则能量无限膨胀。
                // 超出部分丢弃并记账（批次 10 的守恒审计要用）。
                let cap = self.energy_capacity(i);
                let room = (cap - self.organisms[i].energy).max(0.0);
                if gain <= room {
                    self.organisms[i].energy += gain;
                } else {
                    self.organisms[i].energy = cap;
                    self.energy_wasted += gain - room;
                }
            }
            Behavior::Killer => {
                let damage = self.config.attr(cell.type_id, AttrField::Damage);
                if damage <= 0.0 {
                    return;
                }
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
                                let eff = damage * (1.0 - resist);
                                self.harm_organism(owner as u32, eff, Some(my_id));
                                continue;
                            }
                        }
                        self.harm_organism(owner as u32, damage, Some(my_id));
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

    pub fn kill_organism(&mut self, i: usize) {
        if i >= self.organisms.len() || !self.organisms[i].alive {
            return;
        }
        self.organisms[i].alive = false;

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

    fn reproduce_organism(&mut self, i: usize) -> bool {
        let need = self.organisms[i].food_needed(&self.config);
        {
            let org = &mut self.organisms[i];
            org.energy -= need;
            if org.energy < 0.0 {
                org.energy = 0.0;
            }
        }

        if !self.can_add_organism() {
            return false;
        }

        let v = DIR_VECTORS[self.rng.int(4)];
        let distance = self.organisms[i].anatomy.birth_distance + self.rng.int(3) as i32;
        let new_col = self.organisms[i].col + v.0 * distance;
        let new_row = self.organisms[i].row + v.1 * distance;
        let rotation = self.rng.int(4) as u8;

        // 第一道预检用母体蓝图（便宜）
        if !self
            .organisms[i]
            .is_clear(&self.grid, &self.config, new_col, new_row, rotation)
        {
            return false;
        }
        let (pc, pr, pid) = (
            self.organisms[i].col,
            self.organisms[i].row,
            self.organisms[i].id,
        );
        if !self
            .organisms[i]
            .is_straight_path(&self.grid, new_col, new_row, pc, pr, pid)
        {
            return false;
        }

        let mut child = Organism::new(&self.config, &mut self.rng, &self.grid, new_col, new_row);
        child.anatomy.copy_from(&self.organisms[i].anatomy);
        child.brain.copy_from(&self.organisms[i].brain);
        child.rotation = rotation;
        child.direction = self.rng.int(4) as u8;

        {
            let parent = &self.organisms[i];
            let cfg = &self.config;
            mutate_organism(&mut child, parent, &mut self.rng, cfg);
        }

        if !child.can_place_at(
            &self.grid,
            &self.config,
            new_col,
            new_row,
            rotation,
            &child.anatomy,
        ) {
            return false;
        }

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
        if self.light_dirty || self.light.sky != sky {
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
        self.light_dirty = false;
        let World { grid, light, .. } = self;
        light.sky = sky;
        light.rebuild(grid);
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
        }
        OrgStats {
            count: n,
            avg_cells: cells as f64 / n as f64,
            max_cells,
            movers,
            eyes,
            killers,
            avg_age: age / n as f64,
            avg_mutability: mutability / n as f64,
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
    pub avg_cells: f64,
    pub max_cells: usize,
    pub movers: usize,
    pub eyes: usize,
    pub killers: usize,
    pub avg_age: f64,
    pub avg_mutability: f64,
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
}
