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
        changed = mutate_anatomy(&mut child.anatomy, rng, m);
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
pub fn mutate_anatomy(anatomy: &mut Anatomy, rng: &mut Rng, m: &MutationCfg) -> bool {
    let mut changed = false;
    if rng.chance(m.add_prob) && add_random_cell(anatomy, rng) {
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
