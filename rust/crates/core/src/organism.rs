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
        };
        org.normalize_position(grid);
        org
    }

    // ────────────────────────────── 派生量 ──────────────────────────────

    pub fn cell_count(&self) -> usize {
        self.anatomy.cells.len()
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
        let bonus = cfg.life.storage_food_bonus;
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
        cost * scale
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
    pub fn think(&mut self, grid: &Grid, cfg: &Config) {
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
                has_mover,
            };
            sensors::sense(&view, grid, cfg, &mut inputs);
        }
        // 保留一份给检视器看
        self.sensors.copy_from_slice(&inputs);
        self.brain.forward(&inputs);

        if !has_mover {
            return;
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
    pub fn is_clear(&self, grid: &Grid, cfg: &Config, col: i32, row: i32, rotation: u8) -> bool {
        let blocks_food = cfg.combat.food_blocks_reproduction;
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
