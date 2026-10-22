//! 生物个体：结构与派生量。与 JS 版 `src/life/Organism.js` 对应。
//!
//! 注意分工：**纯计算**留在本文件（food_needed / reproduce_at / max_health / think …），
//! 需要写世界或伤害别人的操作（act / reproduce / die / harm）放在 `world.rs`，
//! 因为 Rust 的借用规则不允许"拿着 &mut 自己再借 &mut 世界"。

use serde::{Deserialize, Serialize};

use std::sync::Arc;

use crate::anatomy::{rotate_col, rotate_row, Anatomy, Flags};
use crate::brain::Brain;
use crate::cell_type::{ct, DIR_VECTORS};
use crate::config::AttrField;
use crate::config::Config;
use crate::grid::Grid;
use crate::rng::Rng;
use crate::sensors::{self, SenseView, SENSE_DIM};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Organism {
    pub id: u32,
    pub col: i32,
    pub row: i32,
    /// 身体朝向（枢轴旋转）
    pub rotation: u8,
    /// 移动方向
    pub direction: u8,

    pub age: u64,
    pub energy: f64,
    pub damage: f64,
    /// 连续能量见底的 tick 数
    pub starve_ticks: i32,
    /// 这一 tick 是否站在辐射带里
    pub in_radiation: bool,

    pub alive: bool,
    pub anatomy: Anatomy,

    // ── 遗传信息 ──
    pub mutability: f64,
    pub species_fingerprint: Option<Arc<str>>,
    pub parent_fingerprint: Option<Arc<str>>,
    pub brain: Brain,
    /// 输入缓冲（复用，不分配）
    pub sensors: Vec<f32>,

    // 阶段 A 产出、阶段 B 消费
    pub plan_turn: bool,
    pub plan_direction: u8,
    pub turn_cooldown: i32,

    // ── 仅供检视器显示，不参与模拟 ──
    //
    // 这两项**绝不能进 `state_digest`**：摘要要跟 JS 基线逐位对齐，
    // 而 JS 版没有这两个字段。放在结构体里只是图省事（跟着个体走，
    // 死亡/压缩自动清理），不影响任何决策路径。
    /// 上一 tick 是否真的挪动了（被挡住 / 只在转身 → false）
    pub last_move_ok: bool,
    /// 上一 tick 的能量净收支（正=赚了，负=亏了）
    pub energy_gained_last: f64,

    /// **成长计划**：芽要长成的目标蓝图。`None` = 已经长成（或本来就不是芽）。
    ///
    /// 芽出生时只有 1 格，每 tick 从这份蓝图里挑一个「与身体相邻、目标格可放」
    /// 的细胞长出来，直到长满。**长成之前不能繁殖**——这是「未成年」的全部含义，
    /// 也是把体型变成"过程中的量"的关键。
    #[serde(default)]
    pub growth_plan: Option<Anatomy>,

    /// 芽连续**没能长大**的 tick 数。到 `growth_stall_mature` 就提前成熟。
    ///
    /// 清零的时机只有两个：真的长了一格，或者成年了。
    /// 所以它衡量的是"这个环境让这只芽卡了多久"，不是"它活了多久"。
    #[serde(default)]
    pub grow_stall: u32,

    /// 芽出生以来活了多少 tick。到 `growth_min_ticks` 才允许成年。
    ///
    /// 只在 `growth_plan` 还在时增长；成年之后不再有意义（也不会被读）。
    #[serde(default)]
    pub grow_age: u32,

    /// 繁殖失败后的冷却计时（批次 26）。
    ///
    /// `repro_fail_costs_energy = false` 让失败的个体**下一 tick 立刻重试**，
    /// 于是"找不到空地"的个体会每 tick 白跑一遍四邻域搜索：
    /// 实测尝试次数涨 20 倍、tick 耗时 ×1.67。这个计数器让它们歇一会儿。
    ///
    /// 它同时有个生态上的副作用（可能比性能收益更重要）：
    /// **"什么时候繁殖"变成一件要等的事**，而不是"攒够就立刻生"。
    #[serde(default)]
    pub repro_cooldown: u32,

    // ── 收入来源（诊断用：不进任何判定、只累加）──
    //
    // 存在的理由：光看解剖结构判"自养 / 异养"是不够的。
    // 「生产者 + 嘴」的混养个体在结构上算自养，但它可能 90% 的能量都是吃来的。
    // 这四个计数器回答的是**行为**问题：这只生物到底靠什么活着。
    /// 光合挣到的能量（累计，实际到账的部分）
    #[serde(default)]
    pub gain_photo: f64,
    /// 吃地上食物挣到的
    #[serde(default)]
    pub gain_food: f64,
    /// 咬活体抽到的
    #[serde(default)]
    pub gain_bite: f64,
    /// 母体供养的（出芽）
    #[serde(default)]
    pub gain_gift: f64,

    /// **出芽**时记下母体的 id。`None` = 不是芽（普通出生或已经脱离）。
    ///
    /// 用途只有两个：未成熟期间向母体取能量；成年时清空。
    /// 母体死了 `id_index` 查不到，供养自然停止——不需要额外的清理逻辑。
    #[serde(default)]
    pub parent_id: Option<u32>,
}

/// 一只生物**主要靠什么活着**（按累计能量收入判定）。
///
/// 和「有没有生产者细胞」这种结构判定不同，这里看的是**实际到账的能量**：
/// 一只「生产者 + 嘴」的个体如果九成收入来自咬活体，它算捕食者。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Income {
    /// 光合为主
    Photo,
    /// 吃地上的食物（尸体 / 草甸）为主
    Scavenge,
    /// 咬活体为主
    Predate,
    /// 母体供养为主（还是芽）
    Gift,
    /// 一分钱没挣到（刚出生 / 快饿死）
    Nothing,
}

impl Income {
    /// 在统计数组里的下标（`OrgStats::income` 用这个顺序）。
    ///
    /// 顺序固定：光合 / 吃食物 / 咬活体 / 供养 / 没收入。UI 的图例和横条按这个排。
    #[inline]
    pub fn slot(self) -> usize {
        match self {
            Income::Photo => 0,
            Income::Scavenge => 1,
            Income::Predate => 2,
            Income::Gift => 3,
            Income::Nothing => 4,
        }
    }
}

impl Income {
    /// 给人看的短标签（检视器 / 诊断工具共用）。
    pub fn label(self) -> &'static str {
        match self {
            Income::Photo => "光合",
            Income::Scavenge => "吃食物",
            Income::Predate => "咬活体",
            Income::Gift => "母体供养",
            Income::Nothing => "还没收入",
        }
    }
}

impl Organism {
    /// 累计收入里最大的一份来自哪。
    ///
    /// 用**累计**而不是瞬时：单 tick 的收支噪声太大，看一眼就下结论会误判。
    pub fn income_source(&self) -> Income {
        let (p, f, b, g) = (
            self.gain_photo.max(0.0),
            self.gain_food.max(0.0),
            self.gain_bite.max(0.0),
            self.gain_gift.max(0.0),
        );
        let total = p + f + b + g;
        if total <= 1e-9 {
            return Income::Nothing;
        }
        let mut best = Income::Photo;
        let mut max = p;
        if f > max {
            best = Income::Scavenge;
            max = f;
        }
        if b > max {
            best = Income::Predate;
            max = b;
        }
        if g > max {
            best = Income::Gift;
        }
        best
    }

    /// 累计总收入（四项之和）。
    pub fn income_total(&self) -> f64 {
        self.gain_photo + self.gain_food + self.gain_bite + self.gain_gift
    }
}

impl Organism {
    /// 构造一个空个体。**会消耗 RNG**（大脑随机初始化 545 个高斯数）——
    /// 这一点必须和 JS 版一致，否则整条随机序列就错位了。
    pub fn new(cfg: &Config, rng: &mut Rng, grid: &Grid, col: i32, row: i32) -> Self {
        let mut org = Organism {
            id: 0,
            col,
            row,
            rotation: 0,
            direction: 0,
            age: 0,
            energy: 0.0,
            damage: 0.0,
            starve_ticks: 0,
            in_radiation: false,
            alive: true,
            anatomy: Anatomy::default(),
            mutability: cfg.mutation.initial_mutability,
            species_fingerprint: None,
            parent_fingerprint: None,
            brain: Brain::new(cfg, Some(rng)),
            sensors: vec![0.0; SENSE_DIM],
            plan_turn: false,
            plan_direction: 0,
            turn_cooldown: 0,
            last_move_ok: false,
            energy_gained_last: 0.0,
            growth_plan: None,
            grow_stall: 0,
            grow_age: 0,
            repro_cooldown: 0,
            gain_photo: 0.0,
            gain_food: 0.0,
            gain_bite: 0.0,
            gain_gift: 0.0,
            parent_id: None,
        };
        org.normalize_position(grid);
        org
    }

    // ────────────────────────────── 派生量 ──────────────────────────────

    pub fn cell_count(&self) -> usize {
        self.anatomy.cells.len()
    }

    /// **基因组意义上的身体**：芽用它要长成的蓝图，成体用自己的身体（批次 32）。
    ///
    /// 为什么必须有这一层：芽的 `anatomy` 只有 1 格（蓝图在 `growth_plan` 里），
    /// 拿它算"构成"会把每个芽都当成 1 格的陌生生物——物种数会炸、亲缘识别会漏掉
    /// 最该保护的那一层（母子）。
    pub fn genome(&self) -> &Anatomy {
        self.growth_plan.as_ref().unwrap_or(&self.anatomy)
    }

    /// 繁殖实际扣掉的**基础成本**
    ///
    /// `cell_count^p`，p 来自 `LifeCfg::breed_cost_exponent`，见 [`breed_cost`]。
    pub fn food_needed(&self, cfg: &Config) -> f64 {
        let base = breed_cost(self.cell_count(), cfg.life.breed_cost_exponent);
        let extra = if self.anatomy.has(Flags::MOVER) {
            cfg.life.extra_mover_food_cost
        } else {
            0.0
        };
        base + extra
    }

    /// 触发繁殖的**阈值**（储食细胞抬高它，差额变成储备）
    pub fn reproduce_at(&self, cfg: &Config) -> f64 {
        let base = self.food_needed(cfg);
        if !self.anatomy.has(Flags::STORAGE) {
            return base;
        }
        // 全局「储食加成」× 储食细胞自己的倍率（出厂 1.0 = 与旧行为逐位相同）
        let bonus =
            cfg.life.storage_food_bonus * cfg.attr(ct::STORAGE, AttrField::StorageMult);
        if !(bonus > 0.0) {
            return base;
        }
        base * (1.0 + self.storage_count() as f64 * bonus)
    }

    pub fn storage_count(&self) -> usize {
        self.anatomy
            .cells
            .iter()
            .filter(|c| c.type_id == ct::STORAGE)
            .count()
    }

    /// 能量容量的**上限**（光合直充体内之后必须有，否则无限膨胀）。
    ///
    /// `身体格数 × energy_per_cell + 储食细胞数 × storage_capacity`。
    ///
    /// 储食细胞因此同时是收益和风险：装得多 = 熬得过饥荒 / 黑夜，
    /// 但被杀死时对方从尸体上吃到的也更多。
    ///
    /// 注意它和 [`reproduce_at`](Self::reproduce_at) 是两件事：
    /// 后者是**触发繁殖**的阈值，前者是**能装多少**。容量必须大于阈值，
    /// 否则永远攒不够繁殖的能量——默认参数下 `n×2.0` 对 `n^p` 恒成立。
    pub fn energy_capacity(&self, cfg: &Config) -> f64 {
        let n = self.anatomy.cells.len() as f64;
        n * cfg.life.energy_per_cell
            + self.storage_count() as f64 * cfg.life.storage_capacity
    }

    /// 被攻击时反弹的伤害（所有刺细胞的 damage 之和）
    pub fn spike_damage(&self, cfg: &Config) -> f64 {
        if !self.anatomy.has(Flags::SPIKE) {
            return 0.0;
        }
        self.anatomy
            .cells
            .iter()
            .filter(|c| c.type_id == ct::SPIKE)
            .map(|c| cfg.attr(c.type_id, AttrField::Damage))
            .sum()
    }

    pub fn lifespan(&self, cfg: &Config) -> f64 {
        self.cell_count() as f64 * cfg.life.lifespan_multiplier
    }

    pub fn max_health(&self, cfg: &Config) -> f64 {
        // **芽按目标蓝图算血量**：真实幼体不是"1 HP 的纸片"，它有储备、有保护，
        // 只是还没长开。如果按当前格数算，1 格的芽就是 1 HP——
        // 实测杀手占比会从 14% 暴涨到 50%+，芽刚落地就被吃光，人口从 1359 崩到 117。
        if let Some(plan) = &self.growth_plan {
            return plan
                .cells
                .iter()
                .map(|c| cfg.attr(c.type_id, AttrField::Hp))
                .sum();
        }
        self.anatomy
            .cells
            .iter()
            .map(|c| cfg.attr(c.type_id, AttrField::Hp))
            .sum()
    }

    pub fn upkeep_cost(&self, cfg: &Config) -> f64 {
        let scale = cfg.life.metabolism_scale;
        if !(scale > 0.0) {
            return 0.0;
        }
        let cost: f64 = self
            .anatomy
            .cells
            .iter()
            .map(|c| cfg.attr(c.type_id, AttrField::Cost))
            .sum();
        // Kleiber：`总开销 ∝ 体型^p`，p = 1 时逐位等于旧行为。
        //
        // 等价写法是「每格成本乘 `n^(p-1)`」——n > 1 且 p < 1 时那是个 < 1 的折扣，
        // 也就是**单位体重的代谢率随体型下降**（大动物吃得少、活得久）。
        // 1 格个体 `n^(p-1) = 1`，所以它一分钱折扣都拿不到。
        let p = cfg.life.upkeep_exponent;
        if (p - 1.0).abs() < 1e-12 {
            cost * scale
        } else {
            let n = self.anatomy.cells.len().max(1) as f64;
            cost * n.powf(p - 1.0) * scale
        }
    }

    pub fn food_value_total(&self, cfg: &Config) -> f64 {
        self.anatomy
            .cells
            .iter()
            .map(|c| cfg.attr(c.type_id, AttrField::FoodValue))
            .sum()
    }

    pub fn energy_ratio(&self, cfg: &Config) -> f64 {
        let need = self.reproduce_at(cfg);
        if need <= 0.0 {
            0.0
        } else {
            (self.energy / need).min(1.0)
        }
    }

    pub fn age_ratio(&self, cfg: &Config) -> f64 {
        let life = self.lifespan(cfg);
        if life <= 0.0 {
            0.0
        } else {
            (self.age as f64 / life).min(1.0)
        }
    }

    // ────────────────────────────── 坐标 ──────────────────────────────

    pub fn normalize_position(&mut self, grid: &Grid) {
        if !grid.wrap {
            return;
        }
        self.col = self.col.rem_euclid(grid.cols as i32);
        self.row = self.row.rem_euclid(grid.rows as i32);
    }

    #[inline]
    pub fn real_col(&self, cell: &crate::anatomy::BodyCell) -> i32 {
        self.col + rotate_col(cell.dx, cell.dy, self.rotation)
    }

    #[inline]
    pub fn real_row(&self, cell: &crate::anatomy::BodyCell) -> i32 {
        self.row + rotate_row(cell.dx, cell.dy, self.rotation)
    }

    // ────────────────────────────── 阶段 A：决策 ──────────────────────────────

    /// 感知 → 决策。**只读世界**，不写任何格子（可以并行）。
    /// `soil`：脚下这一格的地力（0~1）。地力关闭时传 1.0（= 旧行为）。
    pub fn think(&mut self, grid: &Grid, cfg: &Config, soil: f64) {
        self.plan_turn = false;

        let has_mover = self.anatomy.has(Flags::MOVER);
        let energy_ratio = self.energy_ratio(cfg);
        let age_ratio = self.age_ratio(cfg);
        let mut inputs = [0f32; SENSE_DIM];
        {
            let view = SenseView {
                cells: &self.anatomy.cells,
                rotation: self.rotation,
                col: self.col,
                row: self.row,
                id: self.id,
                energy_ratio,
                age_ratio,
                soil,
            };
            sensors::sense(&view, grid, cfg, &mut inputs);
        }
        // 保留一份给检视器看
        self.sensors.copy_from_slice(&inputs);
        self.brain.forward(&inputs);

        // 没有运动细胞的个体：默认连决策都不做（转身也没用，因为 `act_move`
        // 被 MOVER 挡住）。`rotate_without_mover` 打开后它们也要算——
        // 转身不改身体格数、只改占用哪些世界格，等于**免费换一块地**。
        if !has_mover && !cfg.life.rotate_without_mover {
            return;
        }

        // ── 大脑消融（批次 33）──
        //
        // 必须放在 `think` 里，不能放在 `World::think_organism`：`step()` 走的是
        // `think_all`（rayon 并行，直接调 `o.think`），那个包装函数根本不会被调用。
        //
        // 两种消融都**不消耗世界 RNG**（模式 2 用 id+age 的哈希），
        // 这样对照组和实验组的随机流一致，差异只来自"决策"本身。
        match cfg.life.brain_ablation {
            1 => {
                // 恒定方向：一直朝当前朝向走（等价于输出全 0 时 argmax 恒选 0 号）
                self.plan_turn = false;
                self.plan_direction = self.rotation;
                return;
            }
            2 => {
                // 每 tick 伪随机方向：大脑完全不参与决策
                let h = (self.id as u64)
                    .wrapping_mul(0x9E37_79B9_7F4A_7C15)
                    .wrapping_add((self.age as u64).wrapping_mul(0xD1B5_4A32_D192_ED03));
                self.plan_turn = false;
                self.plan_direction = ((h >> 33) & 3) as u8;
                return;
            }
            _ => {}
        }

        let out_n = self.brain.out_size;
        let mut out = [0f32; 16];
        out[..out_n].copy_from_slice(&self.brain.output[..out_n]);

        let mut best = 0usize;
        for i in 1..out_n {
            if out[i] > out[best] {
                best = i;
            }
        }

        if best == out_n - 1 && self.turn_cooldown <= 0 {
            self.plan_turn = true;
            return;
        }

        // o[4] 最强但还在冷却 → 退化成"按移动倾向里最强的那个走"
        let mut dir = best;
        if dir == out_n - 1 {
            dir = 0;
            for i in 1..out_n - 1 {
                if out[i] > out[dir] {
                    dir = i;
                }
            }
        }
        self.plan_direction = ((self.rotation as usize + dir) & 3) as u8;
    }

    // ────────────────────────────── 占位判定 ──────────────────────────────

    /// 身体能否整体落在 (col,row) 且朝向 rotation
    ///
    /// 食物是否挡路走 `combat.food_blocks_movement`（和繁殖的开关**分开**：
    /// 「穿过尸体堆」和「生在食物上」是两件不同的事，一个开关管两件事
    /// 会让人没法只放开其中一个）。
    pub fn is_clear(&self, grid: &Grid, cfg: &Config, col: i32, row: i32, rotation: u8) -> bool {
        let blocks_food = cfg.combat.food_blocks_movement;
        for cell in &self.anatomy.cells {
            let c = col + rotate_col(cell.dx, cell.dy, rotation);
            let r = row + rotate_row(cell.dx, cell.dy, rotation);
            let index = match grid.index(c, r) {
                Some(i) => i,
                None => return false,
            };
            let ty = grid.types[index];
            if ty == ct::EMPTY {
                continue;
            }
            if grid.owners[index] == self.id as i32 {
                continue;
            }
            if ty == ct::FOOD && !blocks_food {
                continue;
            }
            if crate::cell_type::is_passable(ty) {
                continue;
            }
            return false;
        }
        true
    }

    /// 繁殖专用：不允许与**任何**生物重叠（含母体自己）
    pub fn can_place_at(
        &self,
        grid: &Grid,
        cfg: &Config,
        col: i32,
        row: i32,
        rotation: u8,
        anatomy: &Anatomy,
    ) -> bool {
        let blocks_food = cfg.combat.food_blocks_reproduction;
        for cell in &anatomy.cells {
            let index = match grid.index(
                col + rotate_col(cell.dx, cell.dy, rotation),
                row + rotate_row(cell.dx, cell.dy, rotation),
            ) {
                Some(i) => i,
                None => return false,
            };
            let ty = grid.types[index];
            if ty == ct::EMPTY {
                continue;
            }
            if ty == ct::FOOD && !blocks_food {
                continue;
            }
            if crate::cell_type::is_passable(ty) {
                continue;
            }
            return false;
        }
        true
    }

    /// 母体到出生点的直线路径是否畅通（假定同行或同列）
    pub fn is_straight_path(
        &self,
        grid: &Grid,
        c1: i32,
        r1: i32,
        c2: i32,
        r2: i32,
        parent_id: u32,
    ) -> bool {
        if c1 == c2 {
            let (from, to) = if r1 < r2 { (r1, r2) } else { (r2, r1) };
            for r in from..=to {
                if !self.passable_at(grid, grid.index(c1, r), parent_id) {
                    return false;
                }
            }
        } else {
            let (from, to) = if c1 < c2 { (c1, c2) } else { (c2, c1) };
            for c in from..=to {
                if !self.passable_at(grid, grid.index(c, r1), parent_id) {
                    return false;
                }
            }
        }
        true
    }

    fn passable_at(&self, grid: &Grid, index: Option<usize>, parent_id: u32) -> bool {
        let index = match index {
            Some(i) => i,
            None => return false,
        };
        let ty = grid.types[index];
        if ty == ct::EMPTY || ty == ct::FOOD {
            return true;
        }
        if crate::cell_type::is_passable(ty) {
            return true;
        }
        let owner = grid.owners[index];
        owner == self.id as i32 || owner == parent_id as i32
    }

    /// 给定方向向量下的新中心坐标（移动一步）
    pub fn step_target(&self) -> (i32, i32) {
        let v = DIR_VECTORS[(self.direction & 3) as usize];
        (self.col + v.0, self.row + v.1)
    }
}

/// 繁殖成本里跟体型有关的那部分：`cell_count^p`。
///
/// 抽成自由函数是为了能直接单测——`food_needed` 挂在 `Organism` 上，造一个实例要
/// 整个网格和 RNG，测一个纯算术公式不值当。
///
/// **p = 1.0 走分支，不调 `powf(1.0)`**：黄金快照要求与 JS 版逐位一致。虽然
/// `x.powf(1.0)` 实际上就等于 x，但没必要为一个纯线性的情况去赌 libm 的实现细节。
/// p <= 0 视为"非法值，退回线性"，免得配置被写成 0 之后成本恒为 1 把种群炸了。
pub fn breed_cost(cells: usize, p: f64) -> f64 {
    let n = cells as f64;
    if (p - 1.0).abs() < 1e-12 || !(p > 0.0) {
        n
    } else {
        n.powf(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 黄金快照的前提：默认配置下成本必须**精确等于**细胞数。
    ///
    /// `tests/golden.rs` 拿 `Config::default()` 跟 JS 基线逐位比对，所以这里不能
    /// 只断言"约等于"——浮点差一位，几千 tick 之后摘要就全变了。
    #[test]
    fn default_exponent_is_exactly_linear() {
        let cfg = Config::default();
        assert_eq!(cfg.life.breed_cost_exponent, 1.0, "默认 p 必须是 1.0");
        assert_eq!(cfg.life.max_body_cells, 0, "默认体型上限必须是 0（不限）");
        for n in 1..=40 {
            assert_eq!(
                breed_cost(n, 1.0),
                n as f64,
                "p=1 时 {} 格的成本必须精确等于 {}",
                n,
                n
            );
        }
    }

    /// 收入来源判定：看**实际到账的能量**，不是看有没有某个细胞。
    ///
    /// 这一条是回答"到底有没有捕食者"的关键——「生产者 + 嘴」在结构上是自养，
    /// 但如果九成收入来自咬活体，它行为上是捕食者。
    #[test]
    fn income_source_looks_at_energy_not_anatomy() {
        let cfg = Config::default();
        let mut rng = Rng::new(1);
        let grid = crate::grid::Grid::new(8, 8, false);
        let mut org = Organism::new(&cfg, &mut rng, &grid, 3, 3);

        assert_eq!(org.income_source(), Income::Nothing, "一分没挣到");

        // 只有光合
        org.gain_photo = 10.0;
        assert_eq!(org.income_source(), Income::Photo);

        // 咬活体占大头 —— 即使它带着生产者细胞，行为上也是捕食者
        org.gain_bite = 25.0;
        org.anatomy.add_cell(ct::PRODUCER, 0, 0, 0);
        org.anatomy.recompute();
        assert!(org.anatomy.has(Flags::PRODUCER), "结构上确实是自养");
        assert_eq!(
            org.income_source(),
            Income::Predate,
            "但收入主要来自咬，就该算捕食"
        );

        // 吃食物超过咬
        org.gain_food = 40.0;
        assert_eq!(org.income_source(), Income::Scavenge);

        // 母体供养超过一切
        org.gain_gift = 100.0;
        assert_eq!(org.income_source(), Income::Gift);
        assert!((org.income_total() - 175.0).abs() < 1e-9);
    }

    /// p < 1 让大体型便宜，p > 1 让它变贵——这是整个"大体型"实验的杠杆方向。
    #[test]
    fn exponent_moves_cost_in_the_right_direction() {
        let n = 6;
        let linear = breed_cost(n, 1.0);
        assert!(breed_cost(n, 0.8) < linear, "p<1 应该让 {} 格更便宜", n);
        assert!(breed_cost(n, 1.3) > linear, "p>1 应该让 {} 格更贵", n);

        // 1 格个体在超线性成本下也应该保持 1.0（1^x = 1），否则最小体型会被额外惩罚
        for p in [0.5, 0.8, 1.0, 1.5, 2.0] {
            assert!((breed_cost(1, p) - 1.0).abs() < 1e-12, "1 格在 p={} 时成本应为 1", p);
        }
    }

    /// 非法 p（0 或负数）退回线性，而不是让成本恒为 1 把种群炸掉。
    #[test]
    fn bogus_exponent_falls_back_to_linear() {
        assert_eq!(breed_cost(7, 0.0), 7.0);
        assert_eq!(breed_cost(7, -1.0), 7.0);
    }
}
