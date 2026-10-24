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
    /// 基因组（批次 45）。为空时按当前身体反推一个退化单段基因组。
    ///
    /// 为什么允许为空：工厂投放、工坊蓝图、旧存档这些路径都是"先有身体"，
    /// 让它们各自记得同步基因组是重复劳动。`effective_genome()` 会兜底。
    pub genome: crate::genome::Genome,
    /// 发育蓝图（批次 45）。原来是 `Option<Anatomy>`，现在带**发育顺序**和父子关系。
    pub growth_plan: Option<crate::genome::BodyPlan>,

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
            genome: crate::genome::Genome::default(),
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
    pub fn expressed_composition(&self) -> [u16; 17] {
        match &self.growth_plan {
            Some(p) => p.composition(),
            None => self.anatomy.composition(),
        }
    }

    /// 有效基因组：没设过就按当前身体反推一个退化单段。
    pub fn effective_genome(&self) -> crate::genome::Genome {
        if self.genome.is_empty() {
            crate::genome::Genome::from_anatomy(&self.anatomy)
        } else {
            self.genome.clone()
        }
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

    /// **体表/体积比**：`8n / 暴露面数`。1 格身体恒为 1.0（基准）。
    ///
    /// 这是批次 47「体型收益」的唯一几何量。它只数**当前已长出来的身体**，
    /// 不数蓝图——所以芽从 1.0 起步、随着长大一路爬到成体的值。
    /// 这是有意的：收益必须来自"真的长成了"，不能来自"基因里写着大"。
    ///
    /// 量级参考：3×3 方块 = 3.0，9 格直线 = 1.8，2 格并排 = 1.14。
    pub fn surface_volume_ratio(&self) -> f64 {
        let n = self.anatomy.cells.len();
        if n == 0 {
            return 1.0;
        }
        let perim = self.anatomy.perimeter.max(1) as f64;
        (8.0 * n as f64 / perim).max(1.0)
    }

    /// 光合产出的体型倍率（`exposure_prod_exp = 0` 时恒为 1.0）。
    pub fn size_prod_mult(&self, cfg: &Config) -> f64 {
        let k = cfg.life.exposure_prod_exp;
        if !(k > 0.0) {
            return 1.0;
        }
        self.surface_volume_ratio().powf(k).clamp(1.0, 4.0)
    }

    /// 维持成本的体型折扣（`exposure_upkeep_exp = 0` 时恒为 1.0）。
    pub fn size_upkeep_mult(&self, cfg: &Config) -> f64 {
        let k = cfg.life.exposure_upkeep_exp;
        if !(k > 0.0) {
            return 1.0;
        }
        self.surface_volume_ratio()
            .powf(-k)
            .clamp(0.25, 1.0)
    }

    pub fn upkeep_cost(&self, cfg: &Config) -> f64 {
        let scale = cfg.life.metabolism_scale;
        if !(scale > 0.0) {
            return 0.0;
        }
        // **成本按器官负载缩放**（阶段 3）：`photo = 128` 的格子只付一半。
        // 出厂预设的负载正好是 1.0（生产者 255+0、嘴 0+255、其余无器官 → 1.0），
        // 所以这一行对旧世界是恒等的。
        let cost: f64 = self
            .anatomy
            .cells
            .iter()
            .map(|c| cfg.attr(c.type_id, AttrField::Cost) * c.org.load())
            .sum();
        // Kleiber：`总开销 ∝ 体型^p`，p = 1 时逐位等于旧行为。
        //
        // 等价写法是「每格成本乘 `n^(p-1)`」——n > 1 且 p < 1 时那是个 < 1 的折扣，
        // 也就是**单位体重的代谢率随体型下降**（大动物吃得少、活得久）。
        // 1 格个体 `n^(p-1) = 1`，所以它一分钱折扣都拿不到。
        let p = cfg.life.upkeep_exponent;
        let shaped = if (p - 1.0).abs() < 1e-12 {
            cost * scale
        } else {
            let n = self.anatomy.cells.len().max(1) as f64;
            cost * n.powf(p - 1.0) * scale
        };
        // 几何折扣（批次 47）：按**形状**而不是按格数。直线身体拿不到，
        // 紧凑身体拿得到——见 `LifeCfg::exposure_upkeep_exp`。
        shaped * self.size_upkeep_mult(cfg)
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

        // 原来这里是 `if best == out_n-1 { plan_turn = true; return; }`。
        // 批次 50 改成打标记而不是直接返回，因为**追猎必须有机会推翻大脑**——
        // 大脑想转身的那一刻，往往正是"眼睛看见猎物但还没对准"的那一刻。
        let mut turning = false;
        if best == out_n - 1 && self.turn_cooldown <= 0 {
            self.plan_turn = true;
            turning = true;
        }

        if !turning {
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

        self.chase_steer(grid, cfg);
    }

    /// 追猎（批次 50）：看到（或碰到）身体、而身上**没有**贴着的猎物时，
    /// 把移动方向掰向最近的那具身体。
    ///
    /// 两档感知，这是刻意设计的：
    ///
    /// | 档 | 谁有 | 范围 | 精度 |
    /// |---|---|---|---|
    /// | 接触 | 任何有嘴/杀手的个体 | 8 邻域（实际只剩对角） | 精确方向 |
    /// | 视觉 | 还要有眼睛 | `SenseRange`（默认 20） | 4 个方向之一 |
    ///
    /// **为什么必须有"接触"这一档**：实测 6000 tick 之后，同时拥有
    /// 眼睛 + 移动器 + 嘴/杀手的个体占比是 **0.0%**——眼睛的维持成本是
    /// 生产者的 3 倍（0.030 vs 0.010），而它此前唯一的用处是给大脑多加
    /// 36 维输入，大脑又是个随机网络。**眼睛在演化里已经死了。**
    /// 如果追猎必须靠眼睛，那这个机制永远不会有任何个体能触发
    /// （实测 chase=0/0.1/0.3/1.0 四个取值的扫描结果**逐位相同**）。
    ///
    /// 所以第一档不要求眼睛：嘴和杀手本身就是化学感受器，挨着就能"尝到"。
    /// 这样机制立刻对**现存的**「移动器 + 嘴」个体生效；眼睛则从
    /// "没有回报的装饰"变成"把 1 格感知放大到 20 格"的升级件——
    /// 这正好打破了「没眼睛 → 追猎不触发 → 眼睛没价值 → 没眼睛」的死循环。
    fn chase_steer(&mut self, grid: &Grid, cfg: &Config) {
        let p = cfg.life.chase_prob;
        if !(p > 0.0) {
            return;
        }
        // 得能"利用"看到的东西。没有嘴也没有杀手，追上去也是白追。
        if !self.anatomy.has(Flags::MOUTH) && !self.anatomy.has(Flags::KILLER) {
            return;
        }
        // **已经贴着了就别追**：继续朝猎物走会被 `attempt_move` 挡下，
        // 然后走 `attempt_rotate` 转身——结果是在猎物旁边打转而不是啃它。
        if self.touching_prey(grid) {
            return;
        }
        // 接触档优先：对角贴着的猎物方向是**确定**的，比"4 个方向里挑一个"准。
        let dir = match self.contact_prey_dir(grid) {
            Some(d) => d,
            None => match self.seen_prey_dir() {
                Some(d) => d,
                None => return,
            },
        };
        // 不碰世界 RNG：`think()` 跑在 rayon 并行路径上，用 `id + age` 的哈希
        // （和大脑消融模式 2 同一个手法，见上面）。
        let h = (self.id as u64)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ (self.age as u64).wrapping_mul(0xD1B5_4A32_D192_ED03);
        let roll = ((h >> 40) & 0xFFFF) as f64 / 65536.0;
        if roll >= p {
            return;
        }
        self.plan_turn = false;
        self.plan_direction = ((self.rotation as usize + dir as usize) & 3) as u8;
    }

    /// 接触档：八邻域里有没有别人的活体，有就返回朝向它的**相对**方向。
    ///
    /// 因为 `touching_prey` 已经把"四邻域有活体"的情况拦掉了，
    /// 这里实际只会命中**对角**接触——也就是"它贴着我但我还咬不到"的位置。
    fn contact_prey_dir(&self, grid: &Grid) -> Option<u8> {
        const N8: [(i32, i32); 8] = [
            (1, 0),
            (-1, 0),
            (0, 1),
            (0, -1),
            (1, 1),
            (1, -1),
            (-1, 1),
            (-1, -1),
        ];
        for cell in &self.anatomy.cells {
            let c = self.real_col(cell);
            let r = self.real_row(cell);
            for (dx, dy) in N8 {
                let Some(idx) = grid.index(c + dx, r + dy) else {
                    continue;
                };
                let owner = grid.owners[idx];
                if owner < 0 || owner == self.id as i32 {
                    continue;
                }
                // 世界方向 → 相对方向：先减掉自身朝向
                let abs = if dx.abs() >= dy.abs() {
                    if dx > 0 { 1 } else { 3 }
                } else if dy > 0 {
                    2
                } else {
                    0
                };
                return Some(((abs + 4 - self.rotation) & 3) as u8);
            }
        }
        None
    }

    /// 视觉档：从已经算好的感知向量里挑最近的那具身体，返回**相对**方向。
    ///
    /// `sense()` 刚填过 `self.sensors`：4 个方向 × 9 维，第 9 维是"近度"
    /// （`1 - 距离/射程`）。槽位顺序见 `VISIBLE_CELL_TYPES`。
    fn seen_prey_dir(&self) -> Option<u8> {
        if !self.anatomy.has(Flags::EYE) {
            return None;
        }
        let mut best_dir: i32 = -1;
        let mut best_near = 0.0f64;
        for d in 0..crate::sensors::VISION_DIRS {
            let base = d * crate::sensors::VISUAL_SLOTS;
            let near = self.sensors[base + crate::sensors::VISUAL_SLOTS - 1] as f64;
            if near <= best_near {
                continue;
            }
            // 这一方向看到的是不是**身体**？食物和墙不算猎物。
            // 感知向量里没有"所有权"，所以同种也会被追——那正好，
            // 亲缘折扣（`kin_penalty`）会在咬的时候把它扣回来。
            let is_body = (0..crate::cell_type::VISIBLE_CELL_TYPES.len()).any(|slot| {
                self.sensors[base + slot] > 0.5
                    && crate::cell_type::is_body_cell(crate::cell_type::VISIBLE_CELL_TYPES[slot])
            });
            if is_body {
                best_near = near;
                best_dir = d as i32;
            }
        }
        if best_dir < 0 {
            None
        } else {
            Some(best_dir as u8)
        }
    }

    /// 身体四邻域里有没有**别人**的活体（不是食物、不是地形）。
    ///
    /// 只看四邻域而不是八邻域：嘴和杀手都只够到四邻域，用八邻域判定
    /// 会让"斜对角有人"的个体误以为自己在进食、于是停止追猎。
    fn touching_prey(&self, grid: &Grid) -> bool {
        const N: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
        for cell in &self.anatomy.cells {
            let c = self.real_col(cell);
            let r = self.real_row(cell);
            for (dx, dy) in N {
                if let Some(idx) = grid.index(c + dx, r + dy) {
                    let owner = grid.owners[idx];
                    if owner >= 0 && owner != self.id as i32 {
                        return true;
                    }
                }
            }
        }
        false
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

// ────────────────── 批次 50：追猎（感知驱动的趋近）──────────────────
#[cfg(test)]
mod chase_tests {
    use super::*;

    fn empty_grid() -> Grid {
        Grid::new(20, 20, false)
    }

    /// 一只「移动器 + 嘴 + 眼睛(朝右)」的捕食者，横排三格，眼睛在**最右端**。
    ///
    /// 眼睛放最右端不是随手排的：`sense()` 遇到自己的格子会 `break`，
    /// 一只把眼睛藏在身体中间、朝自己身体看过去的个体**永远看不见东西**。
    /// 真实演化出来的捕食者也只能靠"眼睛长在外侧"才能追猎——这正是
    /// `see_through_self` 那个开关的意义。
    fn hunter(cfg: &Config, with_eye: bool, with_mouth: bool) -> Organism {
        let mut rng = Rng::new(7);
        let g = empty_grid();
        let mut o = Organism::new(cfg, &mut rng, &g, 10, 10);
        o.anatomy.add_cell(ct::MOVER, 0, 0, 0);
        if with_mouth {
            o.anatomy.add_cell(ct::MOUTH, 1, 0, 0);
        }
        if with_eye {
            o.anatomy.add_cell(ct::EYE, 2, 0, 1); // 朝右
        }
        o.anatomy.recompute();
        o.id = 5;
        o
    }

    /// 往某个方向的视觉槽里写「看到一具身体，近度 near」。
    fn see(o: &mut Organism, dir: usize, ty: u8, near: f32) {
        let base = dir * crate::sensors::VISUAL_SLOTS;
        let slot = crate::cell_type::VISION_SLOT[ty as usize];
        assert!(slot >= 0, "{} 不在视觉槽里", crate::cell_type::label(ty));
        o.sensors[base + slot as usize] = 1.0;
        o.sensors[base + crate::sensors::VISUAL_SLOTS - 1] = near;
    }

    /// `chase_prob = 1` 时方向被掰向看到的那具身体，并且取消转身。
    #[test]
    fn aims_at_the_seen_body() {
        let mut cfg = Config::default();
        cfg.life.chase_prob = 1.0;
        let mut o = hunter(&cfg, true, true);
        see(&mut o, 1, ct::PRODUCER, 0.9); // 右边有人
        o.plan_direction = 0; // 大脑本来想往上
        o.plan_turn = true;
        o.chase_steer(&empty_grid(), &cfg);
        assert_eq!(o.plan_direction, 1, "必须转向猎物所在的绝对方向");
        assert!(!o.plan_turn, "追猎时不能还在转身");
    }

    /// 多个方向都有目标 → 挑**最近**的那个，而不是先扫到的那个。
    #[test]
    fn picks_the_closest_of_several_targets() {
        let mut cfg = Config::default();
        cfg.life.chase_prob = 1.0;
        let mut o = hunter(&cfg, true, true);
        see(&mut o, 0, ct::PRODUCER, 0.30); // 前面，很远
        see(&mut o, 3, ct::KILLER, 0.95); // 左边，很近
        o.chase_steer(&empty_grid(), &cfg);
        assert_eq!(o.plan_direction, 3, "应该追更近的那个");
    }

    /// 食物和墙**不是**猎物——只有身体才值得追。
    #[test]
    fn food_and_wall_are_not_prey() {
        let mut cfg = Config::default();
        cfg.life.chase_prob = 1.0;
        let mut o = hunter(&cfg, true, true);
        see(&mut o, 1, ct::FOOD, 0.9);
        see(&mut o, 2, ct::WALL, 0.9);
        o.plan_direction = 0;
        o.chase_steer(&empty_grid(), &cfg);
        assert_eq!(o.plan_direction, 0, "食物和墙不该引发追猎");
    }

    /// 关掉（默认 0）必须什么都不做。
    #[test]
    fn zero_prob_is_a_no_op() {
        let cfg = Config::default();
        assert_eq!(cfg.life.chase_prob, 0.0, "默认必须是关的");
        let mut o = hunter(&cfg, true, true);
        see(&mut o, 1, ct::PRODUCER, 0.9);
        o.plan_direction = 0;
        o.plan_turn = true;
        o.chase_steer(&empty_grid(), &cfg);
        assert_eq!(o.plan_direction, 0);
        assert!(o.plan_turn, "关掉时连转身标记都不能动");
    }

    /// **视觉档**需要眼睛：没有眼睛就看不见远处的东西。
    /// （近处的东西还能"尝到"，那是接触档，见下一条。）
    #[test]
    fn vision_tier_needs_an_eye() {
        let mut cfg = Config::default();
        cfg.life.chase_prob = 1.0;
        let mut o = hunter(&cfg, false, true);
        see(&mut o, 1, ct::PRODUCER, 0.9);
        o.plan_direction = 0;
        o.chase_steer(&empty_grid(), &cfg);
        assert_eq!(o.plan_direction, 0, "没有眼睛不该走视觉档");
    }

    /// **接触档不需要眼睛**：嘴本身就是化学感受器，对角贴着就能转过去。
    ///
    /// 这一条是整个机制能不能落地的关键——实测世界里同时有
    /// 眼睛 + 移动器 + 嘴/杀手的个体是 **0.0%**，眼睛已经灭绝。
    /// 如果只有视觉档，`chase_prob` 调多大都是空旋钮。
    #[test]
    fn contact_tier_works_without_eye() {
        let mut cfg = Config::default();
        cfg.life.chase_prob = 1.0;
        let mut o = hunter(&cfg, false, true); // 没有眼睛
        o.plan_direction = 0;
        let mut g = empty_grid();
        // 猎物在 (12,9)：和嘴 (11,10) 对角相邻，但不与任何一格四邻相接
        let idx = g.index(12, 9).unwrap();
        g.types[idx] = ct::PRODUCER;
        g.owners[idx] = 99;
        o.chase_steer(&g, &cfg);
        assert_eq!(o.plan_direction, 1, "应该朝对角那具身体转过去");
    }

    /// 没有攻击器官 → 追上去也白追（嘴和杀手都没有，吃不了人家）。
    #[test]
    fn needs_a_weapon() {
        let mut cfg = Config::default();
        cfg.life.chase_prob = 1.0;
        let mut o = hunter(&cfg, true, false);
        see(&mut o, 1, ct::PRODUCER, 0.9);
        o.plan_direction = 0;
        o.chase_steer(&empty_grid(), &cfg);
        assert_eq!(o.plan_direction, 0);
    }

    /// **已经贴着了就别追**：否则它会朝猎物走 → 被挡下 → 转身 → 绕着打转。
    #[test]
    fn stops_chasing_once_in_contact() {
        let mut cfg = Config::default();
        cfg.life.chase_prob = 1.0;
        let mut o = hunter(&cfg, true, true);
        see(&mut o, 1, ct::PRODUCER, 0.9);
        o.plan_direction = 0;
        // 身体在 (10,10)/(11,10)/(12,10)，把猎物放在 (10,9) —— 紧贴第一格
        let mut g = empty_grid();
        let idx = g.index(10, 9).unwrap();
        g.types[idx] = ct::PRODUCER;
        g.owners[idx] = 99;
        o.chase_steer(&g, &cfg);
        assert_eq!(o.plan_direction, 0, "贴脸时应该停下来啃，不是继续走");
    }

    /// 自己的格子不算"贴着的猎物"（否则所有个体一出生就放弃追猎）。
    #[test]
    fn own_body_is_not_prey() {
        let mut cfg = Config::default();
        cfg.life.chase_prob = 1.0;
        let mut o = hunter(&cfg, true, true);
        see(&mut o, 1, ct::PRODUCER, 0.9);
        let mut g = empty_grid();
        for (dx, dy) in [(0, 0), (1, 0), (2, 0)] {
            let idx = g.index(10 + dx, 10 + dy).unwrap();
            g.types[idx] = ct::MOVER;
            g.owners[idx] = 5; // 就是自己
        }
        o.chase_steer(&g, &cfg);
        assert_eq!(o.plan_direction, 1, "自己的格子不该被当成猎物");
    }

    /// 端到端：感知真的从网格里读到猎物，`think()` 真的把方向掰过去。
    ///
    /// 上面几条都是直接喂 `sensors`，只验证了 `chase_steer` 本身；
    /// 这一条走完整条链：`sense()` → `sensors` → `chase_steer` → `plan_direction`。
    #[test]
    fn end_to_end_through_think() {
        let mut cfg = Config::default();
        cfg.life.chase_prob = 1.0;
        let mut o = hunter(&cfg, true, true);
        let mut g = empty_grid();
        // 把身体真的写进网格：否则 `touching_prey` 会看到"猎物贴着我的空壳"。
        for dx in 0..3 {
            let idx = g.index(10 + dx, 10).unwrap();
            g.types[idx] = ct::MOVER;
            g.owners[idx] = 5;
        }
        // 猎物在眼睛（最右端 (12,10)）正前方两格
        let idx = g.index(14, 10).unwrap();
        g.types[idx] = ct::PRODUCER;
        g.owners[idx] = 99;
        o.think(&g, &cfg, 1.0);
        assert_eq!(
            o.plan_direction, 1,
            "眼睛看到正前方的身体，移动方向必须指向它"
        );
    }
}
