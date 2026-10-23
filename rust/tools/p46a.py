import io

def patch(p, pairs):
    s = io.open(p, encoding='utf-8').read()
    for old, new in pairs:
        assert old in s, (p, old[:70])
        s = s.replace(old, new, 1)
    io.open(p, 'w', encoding='utf-8', newline='').write(s)
    print('ok', p)

# ══════════ config：段级操作概率 ══════════
patch('crates/core/src/config.rs', [
(
"""    pub add_prob: f64,
    pub change_prob: f64,
    pub remove_prob: f64,""",
"""    pub add_prob: f64,
    pub change_prob: f64,
    pub remove_prob: f64,
    /// **段级操作的概率**（%，批次 46，默认 45）。
    ///
    /// 变异分两层：
    /// - **细胞级**（加 / 改 / 删一格）：只能微调，永远长不出"新器官"；
    /// - **段级**（重复 / 镜像 / 插段 / 删段 / 平移）：**一次跨 N 格**。
    ///
    /// 这是"演化太随机"的第一味解药：段级操作让变异产生**协调的**表型变化
    /// （repeat +1 = 体型翻倍、mirror = 长出对称的另一半），而不是逐格乱撞。
    pub segment_op_prob: f64,"""),
(
"""            add_prob: 25.0,
            change_prob: 25.0,
            remove_prob: 25.0,""",
"""            add_prob: 25.0,
            change_prob: 25.0,
            remove_prob: 25.0,
            segment_op_prob: 45.0,"""),
])

# ══════════ genome：细胞预算上限 ══════════
patch('crates/core/src/genome.rs', [
(
"""/// 八邻域（和 `mutation.rs` 的 `NEIGHBORS_8` 同序）。""",
"""/// 一份蓝图最多几格（硬上限，批次 46）。
///
/// 段级操作会让体型**成倍**增长（repeat × mirror × 插段），没有上限的话
/// 一次"repeat +1"就能让 40 格的身体变 80 格，而那样的身体在任何世界里都
/// 长不出来——纯粹浪费算力和随机数。64 是"够大但不失控"的经验值。
pub const MAX_PLAN_CELLS: usize = 64;

/// 八邻域（和 `mutation.rs` 的 `NEIGHBORS_8` 同序）。"""),
(
"""        let keep = if max_cells > 0 {
            order.len().min(max_cells)
        } else {
            order.len()
        };""",
"""        // 上限取"调用方给的"和"硬上限"里更小的那个（0 = 只用硬上限）。
        let cap = if max_cells > 0 {
            max_cells.min(MAX_PLAN_CELLS)
        } else {
            MAX_PLAN_CELLS
        };
        let keep = order.len().min(cap);"""),
])

# ══════════ mutation：段级算子 ══════════
patch('crates/core/src/mutation.rs', [
(
"""    let Some(seg) = genome.segments.first_mut() else {
        return false;
    };
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
}""",
"""    // ── 段级操作（批次 46）──
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
            copy.anchor = (0, bbox_max_dy(g) + 1);
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
}"""),
])
