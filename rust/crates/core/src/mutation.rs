//! 变异算子。与 JS 版 `src/life/Mutation.js` 对应。
//!
//! 顺序固定：元变异 → 结构变异 → 权重变异。
//! **调用顺序决定 RNG 消耗顺序**，任何一处提前/延后都会让黄金快照错位。

use std::sync::Arc;

use crate::anatomy::Anatomy;
use crate::brain::Brain;
use crate::cell_type::{is_directional, LIVING_CELL_TYPES};
use crate::config::{Config, MutationCfg};
use crate::organism::Organism;
use crate::rng::Rng;

const MAX_ADD_ATTEMPTS: usize = 6;

/// 八邻域（与 JS 的顺序完全一致，顺序影响 RNG 取值 → 影响结果）
const NEIGHBORS_8: [(i32, i32); 8] = [
    (0, -1),
    (1, 0),
    (0, 1),
    (-1, 0),
    (-1, -1),
    (1, 1),
    (-1, 1),
    (1, -1),
];

/// 对子代应用全部变异。就地修改 child。
pub fn mutate_organism(
    child: &mut Organism,
    parent: &Organism,
    rng: &mut Rng,
    cfg: &Config,
) -> bool {
    let m = &cfg.mutation;

    // ── 1. 元变异：变异率从母体继承 ──
    child.mutability = parent.mutability;

    // ── 2. 结构变异 ──
    let base = if m.use_global_mutability {
        m.global_mutability
    } else {
        child.mutability
    };

    // 辐射带加成：站在辐射里的个体，**后代**更容易突变（临时乘，不写回 mutability）
    let rad_boost = if parent.in_radiation {
        cfg.terrain.rad_mutation_boost.max(1.0)
    } else {
        1.0
    };
    let rate = base * rad_boost;

    let mut changed = false;
    if rate > 0.0 && rng.chance(rate.min(100.0)) {
        changed = mutate_anatomy(&mut child.anatomy, rng, m, cfg.life.max_body_cells);
        if changed {
            child.mutability =
                (parent.mutability + rng.gaussian(0.0, m.mutability_sigma)).clamp(1.0, 100.0);
        }
    }

    // ── 3. 权重变异 ──
    mutate_brain(&mut child.brain, rng, m);

    // ── 指纹 ──
    child.parent_fingerprint = parent.species_fingerprint.clone();
    child.species_fingerprint = if changed {
        Some(Arc::from(child.anatomy.fingerprint().as_str()))
    } else {
        parent.species_fingerprint.clone()
    };

    changed
}

/// 结构变异：三个算子各自独立判定
///
/// `max_cells` 是体型上限（0 = 不限）。**`max_cells == 0` 时走的代码路径与加这个参数
/// 之前逐字节相同**——这是硬要求，`tests/golden.rs` 拿 `Config::default()` 跟 JS 基线
/// 逐位比对，默认值一变就得重做基线。
///
/// 到了上限就整个跳过"加细胞"算子（连同它那一次 `rng.chance`），所以启用上限之后
/// 随机流会和无限版本错开。这是可以接受的：启用上限本来就改变了模拟。
/// 反过来，如果为了让随机流对齐而"先掷骰子再丢弃结果"，会白白消耗随机数，
/// 而且 `add_random_cell` 内部的消耗量取决于身体内容，本来也对不齐——不值得。
pub fn mutate_anatomy(
    anatomy: &mut Anatomy,
    rng: &mut Rng,
    m: &MutationCfg,
    max_cells: usize,
) -> bool {
    let mut changed = false;
    // 上限只挡"变大"，缩小体型的算子照常判定——否则到了上限就再也变不回来了。
    let can_add = max_cells == 0 || anatomy.cells.len() < max_cells;
    if can_add && rng.chance(m.add_prob) && add_random_cell(anatomy, rng) {
        changed = true;
    }
    if rng.chance(m.change_prob) && change_random_cell(anatomy, rng) {
        changed = true;
    }
    if rng.chance(m.remove_prob) && anatomy.remove_random_cell(rng) {
        changed = true;
    }
    changed
}

fn add_random_cell(anatomy: &mut Anatomy, rng: &mut Rng) -> bool {
    for _ in 0..MAX_ADD_ATTEMPTS {
        let base_idx = match anatomy.random_cell_index(rng) {
            Some(i) => i,
            None => return false,
        };
        let (bx, by) = (anatomy.cells[base_idx].dx, anatomy.cells[base_idx].dy);
        let n = NEIGHBORS_8[rng.int(8)];
        let dx = bx + n.0;
        let dy = by + n.1;
        if anatomy.cell_at(dx, dy).is_some() {
            continue; // 这个方向被占了，换个方向再试
        }
        let type_id = LIVING_CELL_TYPES[rng.int(LIVING_CELL_TYPES.len())];
        let dir = if is_directional(type_id) {
            rng.int(4) as u8
        } else {
            0
        };
        return anatomy.add_cell(type_id, dx, dy, dir);
    }
    false
}

fn change_random_cell(anatomy: &mut Anatomy, rng: &mut Rng) -> bool {
    let idx = match anatomy.random_cell_index(rng) {
        Some(i) => i,
        None => return false,
    };
    let type_id = LIVING_CELL_TYPES[rng.int(LIVING_CELL_TYPES.len())];
    if type_id == anatomy.cells[idx].type_id && !is_directional(type_id) {
        return false;
    }
    anatomy.cells[idx].type_id = type_id;
    anatomy.cells[idx].dir = if is_directional(type_id) {
        rng.int(4) as u8
    } else {
        0
    };
    anatomy.recompute();
    true
}

/// 权重变异（不改变物种指纹）
pub fn mutate_brain(brain: &mut Brain, rng: &mut Rng, m: &MutationCfg) {
    let rate = m.weight_mut_rate * 100.0;
    let reset_rate = m.weight_reset_rate * 100.0;
    for arr_idx in 0..4 {
        let len = match arr_idx {
            0 => brain.w1.len(),
            1 => brain.b1.len(),
            2 => brain.w2.len(),
            _ => brain.b2.len(),
        };
        for i in 0..len {
            if rng.chance(rate) {
                let reset = rng.chance(reset_rate);
                let w = match arr_idx {
                    0 => &mut brain.w1[i],
                    1 => &mut brain.b1[i],
                    2 => &mut brain.w2[i],
                    _ => &mut brain.b2[i],
                };
                if reset {
                    *w = rng.range(-1.0, 1.0) as f32;
                } else {
                    *w += rng.gaussian(0.0, m.weight_mut_sigma) as f32;
                    if *w > 4.0 {
                        *w = 4.0;
                    } else if *w < -4.0 {
                        *w = -4.0;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell_type::ct;

    /// 只留"加细胞"的变异配置：别的算子关掉，才能把上限的作用单独隔离出来
    fn grow_only() -> MutationCfg {
        MutationCfg {
            add_prob: 100.0,
            change_prob: 0.0,
            remove_prob: 0.0,
            ..MutationCfg::default()
        }
    }

    /// 到达体型上限后，"加细胞"必须彻底失效——否则上限形同虚设。
    #[test]
    fn body_cap_stops_growth() {
        let m = grow_only();
        let mut a = Anatomy::default();
        a.add_cell(ct::MOUTH, 0, 0, 0);
        a.recompute();

        let mut rng = Rng::new(7);
        for _ in 0..40 {
            mutate_anatomy(&mut a, &mut rng, &m, 3);
        }
        assert_eq!(a.cells.len(), 3, "上限 3，加细胞算子应该刚好停在 3 格");
    }

    /// cap = 0 是"不限"，不是"上限 0"。这两个语义搞混的话，默认配置会把所有个体
    /// 锁死在 1 格——而黄金快照测的正是默认配置。
    #[test]
    fn zero_cap_means_unlimited_not_zero() {
        let m = grow_only();
        let mut a = Anatomy::default();
        a.add_cell(ct::MOUTH, 0, 0, 0);
        a.recompute();

        let mut rng = Rng::new(7);
        for _ in 0..40 {
            mutate_anatomy(&mut a, &mut rng, &m, 0);
        }
        assert!(
            a.cells.len() > 3,
            "cap=0 应该不设限（实际只有 {} 格）",
            a.cells.len()
        );
    }

    /// 到上限后缩小体型的算子必须还能用，否则个体一旦长满就再也回不来了。
    #[test]
    fn body_cap_still_allows_shrinking() {
        let m = MutationCfg {
            add_prob: 0.0,
            change_prob: 0.0,
            remove_prob: 100.0,
            ..MutationCfg::default()
        };
        let mut a = Anatomy::default();
        a.add_cell(ct::MOUTH, 0, 0, 0);
        a.add_cell(ct::PRODUCER, 1, 0, 0);
        a.add_cell(ct::PRODUCER, -1, 0, 0);
        a.recompute();

        let mut rng = Rng::new(11);
        // 上限设成 1（当前已经 3 格，处在"超限"状态）
        mutate_anatomy(&mut a, &mut rng, &m, 1);
        assert!(a.cells.len() < 3, "到了上限也不能冻住，缩小体型的变异要照常生效");
    }
}
