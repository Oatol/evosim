//! 变异算子。与 JS 版 `src/life/Mutation.js` 对应。
//!
//! 顺序固定：元变异 → 结构变异 → 权重变异。
//! **调用顺序决定 RNG 消耗顺序**，任何一处提前/延后都会让黄金快照错位。

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

/// 对**基因组**应用全部变异（批次 45）。
///
/// 和旧版 `mutate_organism` 的区别：变异作用的对象从"身体"变成"基因组"。
/// 这一步是整个发育层的入口——没有它，"变异"永远只能改当下这一具身体，
/// 改不了"这具身体要长成什么样"。
///
/// 阶段 0 的基因组是**退化单段**（一个段、repeat=1、mirror=false），
/// 所以这里直接借道 `mutate_anatomy` 的成熟算子：把段里的格子临时装成
/// 一份 `Anatomy`，跑完三个算子再装回去。等阶段 1 有了 repeat/mirror，
/// 再换成真正按段操作的算子。
///
/// 返回 `true` = 结构真的变了（调用方据此决定要不要扰动 `mutability`）。
pub fn mutate_genome(
    genome: &mut crate::genome::Genome,
    parent: &Organism,
    rng: &mut Rng,
    cfg: &Config,
) -> bool {
    let m = &cfg.mutation;

    // 元变异：变异率从母体继承（这一条本来就在，位置不变）
    let base = if m.use_global_mutability {
        m.global_mutability
    } else {
        parent.mutability
    };
    // 辐射带加成：站在辐射里的个体，**后代**更容易突变（临时乘，不写回 mutability）
    let rad_boost = if parent.in_radiation {
        cfg.terrain.rad_mutation_boost.max(1.0)
    } else {
        1.0
    };
    let rate = base * rad_boost;
    if rate <= 0.0 || !rng.chance(rate.min(100.0)) {
        return false;
    }

    // ── 段级操作（批次 46）──
    //
    // 这是阶段 1 的核心：**一个变异可以跨 N 格**。
    // 细胞级算子永远只能微调（加一格 / 改一格 / 删一格），
    // 而 `repeat +1` 让体型直接翻倍、`mirror` 让身体长出对称的另一半。
    // 这正是"协调的表型变化"——旧代码里完全缺失的东西。
    if rng.chance(m.segment_op_prob) {
        return segment_op(genome, rng);
    }

    // ── 细胞级操作：随机挑一个段，借道 `mutate_anatomy` 的成熟算子 ──
    if genome.segments.is_empty() {
        return false;
    }
    let si = rng.int(genome.segments.len());
    let seg = &mut genome.segments[si];
    let mut a = Anatomy::default();
    for c in &seg.cells {
        a.add_cell(c.type_id, c.dx, c.dy, c.dir);
    }
    let changed = mutate_anatomy(&mut a, rng, m, cfg.life.max_body_cells);
    if changed {
        seg.cells = a
            .cells
            .iter()
            .map(|c| crate::genome::GeneCell::new(c.type_id, c.dx, c.dy, c.dir))
            .collect();
    }
    changed
}

/// 基因组展开后的总格数（"细胞预算"用）。
fn plan_size(g: &crate::genome::Genome) -> usize {
    g.segments
        .iter()
        .map(|s| {
            let copies = s.repeat.max(1) as usize * if s.mirror { 2 } else { 1 };
            s.cells.len() * copies
        })
        .sum()
}

/// 整个基因组在 y 方向的最低点（插段时用来"叠在下方"）。
fn bbox_max_dy(g: &crate::genome::Genome) -> i32 {
    let mut m = 0;
    for s in &g.segments {
        let min_dy = s.cells.iter().map(|c| c.dy).min().unwrap_or(0);
        let max_dy = s.cells.iter().map(|c| c.dy).max().unwrap_or(0);
        let span = (max_dy - min_dy + 1).max(1);
        m = m.max(s.anchor.1 + min_dy + span * s.repeat.max(1) as i32);
    }
    m
}

/// 段级操作：一次改动影响整块身体。
///
/// 五个算子等概率挑一个：
///
/// | 算子 | 效果 | 生态意义 |
/// |---|---|---|
/// | `repeat ±1` | 体型成倍增减 | 体节 / 体型 |
/// | `mirror` | 长出对称的另一半 | 成对器官 |
/// | 插段 | 复制一段叠在下方 | 新器官（**重尾增长**的主要来源） |
/// | 删段 | 丢掉一段 | 简化 |
/// | 平移 | 整段挪一格 | 只在多段时有效；单段会被"胚归一化"抹掉 → 中性变异 |
///
/// 每一步都检查 `MAX_PLAN_CELLS`：超了就回滚。没有这道闸，一次 repeat+1
/// 就能把 40 格变 80 格，而那种身体在任何世界里都长不出来。
fn segment_op(g: &mut crate::genome::Genome, rng: &mut Rng) -> bool {
    use crate::genome::MAX_PLAN_CELLS;
    if g.segments.is_empty() {
        return false;
    }
    match rng.int(5) {
        // ① 重复次数 ±1（偏向上调：基因/体节重复是真实演化的主要创新来源）
        0 => {
            let i = rng.int(g.segments.len());
            let up = rng.chance(65.0);
            if up {
                let old = g.segments[i].repeat;
                if old >= 8 {
                    return false;
                }
                g.segments[i].repeat = old + 1;
                if plan_size(g) <= MAX_PLAN_CELLS {
                    true
                } else {
                    g.segments[i].repeat = old;
                    false
                }
            } else if g.segments[i].repeat > 1 {
                g.segments[i].repeat -= 1;
                true
            } else {
                false
            }
        }
        // ② 镜像开关
        1 => {
            let i = rng.int(g.segments.len());
            g.segments[i].mirror = !g.segments[i].mirror;
            if plan_size(g) > MAX_PLAN_CELLS {
                g.segments[i].mirror = !g.segments[i].mirror;
                return false;
            }
            true
        }
        // ③ 插一段：复制现有的一段，叠在身体最低处下方
        2 => {
            let i = rng.int(g.segments.len());
            let mut copy = g.segments[i].clone();
            // `bbox_max_dy` 返回的已经是「最后一行之后的那一行」，直接当锚点用
            // 就正好贴着身体下方。再 +1 会留一条缝，整段变成孤岛被丢掉。
            copy.anchor = (0, bbox_max_dy(g));
            g.segments.push(copy);
            if plan_size(g) > MAX_PLAN_CELLS {
                g.segments.pop();
                return false;
            }
            true
        }
        // ④ 删一段
        3 => {
            if g.segments.len() <= 1 {
                return false;
            }
            let i = rng.int(g.segments.len());
            g.segments.remove(i);
            true
        }
        // ⑤ 整段平移一格
        _ => {
            let i = rng.int(g.segments.len());
            let d = [(1, 0), (-1, 0), (0, 1), (0, -1)][rng.int(4)];
            g.segments[i].anchor.0 += d.0;
            g.segments[i].anchor.1 += d.1;
            true
        }
    }
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

/// 从"**表面**"的细胞里随机挑一个（有至少一个空邻居的）。
///
/// 为什么不能纯随机挑：身体是**紧贴**着长的，内部细胞的 8 个邻居全被自己占着，
/// 抽到它就必然失败。所以"加细胞"这条路径在旧代码里**成功率极低**，而
/// "删细胞"永远成功——净效果是基因组只会缩、不会长。
///
/// 实测（批次 45）：纯随机挑的时候，加 40% / 删 10% 的偏置都推不动体型，
/// 基因组永远停在 3 格；改成只从表面挑之后，体型才真正开始长。
fn peripheral_cell_index(anatomy: &Anatomy, rng: &mut Rng) -> Option<usize> {
    let mut cand: Vec<usize> = Vec::new();
    for (i, c) in anatomy.cells.iter().enumerate() {
        for n in NEIGHBORS_8 {
            if anatomy.cell_at(c.dx + n.0, c.dy + n.1).is_none() {
                cand.push(i);
                break;
            }
        }
    }
    if cand.is_empty() {
        return None;
    }
    Some(cand[rng.int(cand.len())])
}

fn add_random_cell(anatomy: &mut Anatomy, rng: &mut Rng) -> bool {
    for _ in 0..MAX_ADD_ATTEMPTS {
        // 优先从表面长（生长发生在表面），退化成纯随机只在"全身都堵死"时发生。
        let base_idx = match peripheral_cell_index(anatomy, rng)
            .or_else(|| anatomy.random_cell_index(rng))
        {
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
    // 5 组：w1 / b1 / w2 / b2 / w_rec（循环关掉时 w_rec 是空的，循环体不跑）。
    for arr_idx in 0..5 {
        let len = match arr_idx {
            0 => brain.w1.len(),
            1 => brain.b1.len(),
            2 => brain.w2.len(),
            3 => brain.b2.len(),
            _ => brain.w_rec.len(),
        };
        for i in 0..len {
            if rng.chance(rate) {
                let reset = rng.chance(reset_rate);
                let w = match arr_idx {
                    0 => &mut brain.w1[i],
                    1 => &mut brain.b1[i],
                    2 => &mut brain.w2[i],
                    3 => &mut brain.b2[i],
                    _ => &mut brain.w_rec[i],
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

    // ── 批次 46：段级算子 ──

    use crate::genome::{GeneCell, Genome, Segment, MAX_PLAN_CELLS};

    fn one_segment() -> Genome {
        Genome {
            segments: vec![Segment::plain(vec![
                GeneCell::new(ct::PRODUCER, 0, 0, 0),
                GeneCell::new(ct::MOUTH, 1, 0, 0),
            ])],
        }
    }

    /// 段级算子必须守住两条不变量：**不突破细胞预算**、**不把段删光**。
    #[test]
    fn segment_ops_respect_the_budget_and_never_empty_the_genome() {
        let mut g = one_segment();
        let mut rng = Rng::new(9);
        for i in 0..800 {
            segment_op(&mut g, &mut rng);
            assert!(
                plan_size(&g) <= MAX_PLAN_CELLS,
                "第 {i} 次段级操作把体型推到 {} 格（上限 {MAX_PLAN_CELLS}）",
                plan_size(&g)
            );
            assert!(!g.segments.is_empty(), "第 {i} 次把段删光了");
            assert!(
                g.segments.iter().all(|s| s.repeat >= 1),
                "第 {i} 次把 repeat 归零了（会展开成空身体）"
            );
        }
    }

    /// `repeat` 与 `mirror` 必须真的让身体变大——这是阶段 1 的全部意义。
    #[test]
    fn repeat_and_mirror_grow_the_body() {
        let mut g = one_segment();
        let base = g.expand(0).len();
        assert_eq!(base, 2);

        g.segments[0].repeat = 2;
        let doubled = g.expand(0).len();
        assert_eq!(doubled, 4, "repeat 2 应该让体型翻倍");

        g.segments[0].mirror = true;
        let mirrored = g.expand(0).len();
        assert!(
            mirrored > doubled,
            "镜像应该再加一份（轴上的格子会去重），实测 {mirrored}"
        );
    }

    /// 插段 = 复制一段叠在下方，身体变长。
    #[test]
    fn insert_segment_stacks_below_the_body() {
        let mut g = one_segment();
        let before = g.expand(0).len();
        let mut copy = g.segments[0].clone();
        copy.anchor = (0, bbox_max_dy(&g));
        g.segments.push(copy);
        let after = g.expand(0).len();
        assert_eq!(after, before * 2, "复制一段应该让身体长一倍");
        // 两段必须在 y 上错开
        let ys: std::collections::BTreeSet<i32> = g.expand(0).cells.iter().map(|c| c.dy).collect();
        assert_eq!(ys.len(), 2, "两段应该占两行");
    }

    /// 段级算子跑很多次之后，体型应该能明显长起来（不是原地打转）。
    #[test]
    fn segment_ops_can_grow_the_plan_over_time() {
        let mut g = one_segment();
        let mut rng = Rng::new(17);
        let mut max_seen = g.expand(0).len();
        for _ in 0..600 {
            segment_op(&mut g, &mut rng);
            max_seen = max_seen.max(g.expand(0).len());
        }
        assert!(
            max_seen >= 8,
            "600 次段级操作都没让身体长到 8 格（最大 {max_seen}）——算子没在起作用"
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
