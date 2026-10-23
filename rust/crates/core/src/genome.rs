//! 基因组与形态解释器（批次 45 · 阶段 0）。
//!
//! 这一层把「逐格细胞列表」升级成 **基因组 → 形态解释器 → 身体**。
//! 阶段 0 只用到**退化形态**：一个段、`repeat = 1`、`mirror = false`、`rot = 0`，
//! 也就是"基因组就是那份逐格列表"，所以展开是恒等映射，可以和旧世界逐项对照。
//!
//! 但结构已经按最终形态建好了（`repeat` / `mirror` / `rot` / `children` 都能用），
//! 阶段 1 只需要加变异算子，不用再动这一层。
//!
//! ## 为什么要有 `BodyPlan` 而不是直接用 `Anatomy`
//!
//! 因为发育需要**顺序**和**父子关系**：
//!
//! - **顺序**：每 tick 长一格，得知道"下一格是哪一格"。按 BFS 从胚开始长，
//!   身体才会由内向外长，而不是先长出边缘的一根手指。
//! - **父子关系**：某一格被挡住长不出来时，**它下游的格子也长不出来**
//!   （没有它就没有"相邻"这条路）。所以要能一次砍掉一整棵子树——
//!   这是"受阻即凋亡"，也是替代旧代码里"随机类型填任意相邻空位"的那个补丁。
//!
//! ## 三条硬约束
//!
//! 1. **确定性**：`expand` 不消耗 RNG。同一个基因组永远展开成同一个身体。
//! 2. **胚在 0 号位**：`cells[0]` 永远是胚（出生时的那一格），BFS 也从它出发。
//! 3. **`parent[i] < i`**：BFS 顺序保证父格一定在子格之前，
//!    所以 `prune_subtree` 只要正向扫一遍就能标完所有后代。

use serde::{Deserialize, Serialize};

use crate::anatomy::{Anatomy, BodyCell};
use crate::cell_type::ct;

/// 一份蓝图最多几格（硬上限，批次 46）。
///
/// 段级操作会让体型**成倍**增长（repeat × mirror × 插段），没有上限的话
/// 一次"repeat +1"就能让 40 格的身体变 80 格，而那样的身体在任何世界里都
/// 长不出来——纯粹浪费算力和随机数。64 是"够大但不失控"的经验值。
pub const MAX_PLAN_CELLS: usize = 64;

/// 八邻域（和 `mutation.rs` 的 `NEIGHBORS_8` 同序）。
///
/// **身体是按 8 邻域拼出来的**（`add_random_cell` 从 8 个方向挑），
/// 所以"连在一起"必须按 8 邻域判。用 4 邻域会把对角相接的格子当成孤岛。
pub const NEIGHBORS8: [(i32, i32); 8] = [
    (0, -1),
    (1, 0),
    (0, 1),
    (-1, 0),
    (-1, -1),
    (1, 1),
    (-1, 1),
    (1, -1),
];

/// 基因组里的一个格子。
///
/// 和 `BodyCell` 的差别只有一个：**它描述的是"要长成什么"，不是"现在是什么"**。
/// 阶段 3 会把 `type_id` 换成细胞器向量，那时这个结构才会真正分开。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneCell {
    pub type_id: u8,
    pub dx: i32,
    pub dy: i32,
    pub dir: u8,
}

impl GeneCell {
    pub fn new(type_id: u8, dx: i32, dy: i32, dir: u8) -> Self {
        GeneCell {
            type_id,
            dx,
            dy,
            dir,
        }
    }
}

/// 一个"部件"。
///
/// 阶段 0 只用一个段（`repeat = 1`、`mirror = false`、`rot = 0`），
/// 但字段全部按最终形态留着：
///
/// - `repeat`：沿主轴重复 N 次 → 体节。
/// - `mirror`：镜像出一份 → 成对器官。
/// - `rot`：整段旋转 90° 的倍数。
/// - `children`：从这一段末尾长出去的子段 → 附肢（阶段 2）。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Segment {
    pub cells: Vec<GeneCell>,
    /// 相对父段末端的锚点（顶层段相对世界原点）。
    pub anchor: (i32, i32),
    pub repeat: u8,
    pub mirror: bool,
    pub rot: u8,
    pub children: Vec<Segment>,
}

impl Segment {
    /// 一份普通的、不重复不镜像的段。
    pub fn plain(cells: Vec<GeneCell>) -> Self {
        Segment {
            cells,
            anchor: (0, 0),
            repeat: 1,
            mirror: false,
            rot: 0,
            children: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Genome {
    pub segments: Vec<Segment>,
}

/// 展开后的身体蓝图。
///
/// `cells` **按发育顺序（BFS）排列**，`cells[0]` 是胚。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BodyPlan {
    pub cells: Vec<BodyCell>,
    /// 每格的父格下标（`-1` = 胚）。保证 `parent[i] < i`。
    pub parent: Vec<i16>,
    /// 段嵌套深度，0 = 顶层段。用于"模块数 / 压缩率"这类诊断。
    pub depth: Vec<u8>,
    /// 受阻凋亡标记。被标 `true` 的格子永远不会长出来。
    pub dropped: Vec<bool>,
    /// 每一格连续被挡了多少 tick。超过阈值就凋亡（见 `World::grow_organism`）。
    ///
    /// 为什么需要"等几 tick"而不是一受阻就砍：拥挤的芽常常是"这一格被母体占着，
    /// 下一 tick 母体挪开就好了"。一刀切会把整支身体砍掉（实测成体从 14 格掉到 2 格）。
    #[serde(default)]
    pub blocked: Vec<u8>,
}

impl BodyPlan {
    /// 胚：出生时的那一格。
    pub fn seed(&self) -> Option<BodyCell> {
        self.cells.first().copied()
    }

    pub fn len(&self) -> usize {
        self.cells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// 还有几格没长出来（含已凋亡的不算）。
    pub fn pending(&self, anatomy: &Anatomy) -> usize {
        self.cells
            .iter()
            .enumerate()
            .filter(|(i, c)| !self.dropped[*i] && anatomy.cell_at(c.dx, c.dy).is_none())
            .count()
    }

    /// 砍掉第 `k` 格和它的全部后代。
    ///
    /// 因为 `parent[i] < i`，正向扫一遍就够：只要父格被标了，子格一定在后面。
    pub fn prune_subtree(&mut self, k: usize) {
        if k >= self.cells.len() {
            return;
        }
        self.dropped[k] = true;
        for i in (k + 1)..self.cells.len() {
            let p = self.parent[i];
            if p >= 0 && self.dropped[p as usize] {
                self.dropped[i] = true;
            }
        }
    }

    /// 构成（每种细胞几格），口径和 `Anatomy::composition` 完全一致。
    pub fn composition(&self) -> [u16; 17] {
        let mut c = [0u16; 17];
        for cell in &self.cells {
            c[(cell.type_id as usize).min(16)] += 1;
        }
        c
    }

    /// 转回 `Anatomy`（给不启用生长的路径 / 工坊预览 / 存档兼容用）。
    pub fn to_anatomy(&self) -> Anatomy {
        let mut a = Anatomy::default();
        for (i, cell) in self.cells.iter().enumerate() {
            if self.dropped[i] {
                continue;
            }
            a.add_cell(cell.type_id, cell.dx, cell.dy, cell.dir);
        }
        a
    }

    /// **模块数**：同类型、四邻连通的块有几个。
    ///
    /// 这是"身体有没有结构"的最便宜度量：一团同类型细胞 = 1，
    /// "中间生产者 + 外层嘴" = 2，"分段 + 附肢" 会更高。
    pub fn modules(&self) -> usize {
        let n = self.cells.len();
        if n == 0 {
            return 0;
        }
        let mut seen = vec![false; n];
        let mut count = 0;
        // 位置 → 下标（同位置只会有一格，展开时已去重）
        let mut at = std::collections::HashMap::with_capacity(n);
        for (i, c) in self.cells.iter().enumerate() {
            at.insert((c.dx, c.dy), i);
        }
        for start in 0..n {
            if seen[start] {
                continue;
            }
            count += 1;
            let ty = self.cells[start].type_id;
            let mut stack = vec![start];
            seen[start] = true;
            while let Some(i) = stack.pop() {
                let (dx, dy) = (self.cells[i].dx, self.cells[i].dy);
                for (ox, oy) in NEIGHBORS8 {
                    if let Some(&j) = at.get(&(dx + ox, dy + oy)) {
                        if !seen[j] && self.cells[j].type_id == ty {
                            seen[j] = true;
                            stack.push(j);
                        }
                    }
                }
            }
        }
        count
    }
}

impl Genome {
    pub fn is_empty(&self) -> bool {
        self.segments.iter().all(|s| s.cells.is_empty())
    }

    /// 退化形态：把一份身体原样包成"单段基因组"。
    ///
    /// 这是阶段 0 的主力构造函数：所有旧路径（工厂投放、工坊蓝图、测试）
    /// 都能靠它一步进入新世界，而展开是恒等映射。
    pub fn from_anatomy(a: &Anatomy) -> Self {
        let cells = a
            .cells
            .iter()
            .map(|c| GeneCell::new(c.type_id, c.dx, c.dy, c.dir))
            .collect();
        Genome {
            segments: vec![Segment::plain(cells)],
        }
    }

    /// 展开成身体蓝图。`max_cells = 0` 表示不限。
    pub fn expand(&self, max_cells: usize) -> BodyPlan {
        // ── 1. 递归展开所有段 ──
        let mut raw: Vec<(i32, i32, u8, u8)> = Vec::new(); // (dx, dy, type, dir) + depth 另存
        let mut raw_depth: Vec<u8> = Vec::new();
        for seg in &self.segments {
            emit(seg, 0, (0, 0), &mut raw, &mut raw_depth);
        }

        // ── 2. 去重：先到先得（段按声明顺序，重复按 r，镜像按 m）──
        let mut occupied: std::collections::HashMap<(i32, i32), usize> =
            std::collections::HashMap::with_capacity(raw.len());
        let mut cells: Vec<BodyCell> = Vec::with_capacity(raw.len());
        let mut depth: Vec<u8> = Vec::with_capacity(raw.len());
        for (i, &(dx, dy, ty, dir)) in raw.iter().enumerate() {
            if occupied.contains_key(&(dx, dy)) {
                continue;
            }
            occupied.insert((dx, dy), cells.len());
            cells.push(BodyCell::new(ty, dx, dy, dir));
            depth.push(raw_depth[i]);
        }
        if cells.is_empty() {
            return BodyPlan::default();
        }

        // ── 3. 选胚：坐标原点那一格；没有就用第一格 ──
        //
        // 用**位置**而不是"第一格"是有理由的：段可能从别处开始画，
        // 而原点通常是身体的中心。阶段 1 会让 `Genome::seed` 显式指定。
        let seed_pos = (0, 0);
        let seed = occupied.get(&seed_pos).copied().unwrap_or(0);

        // ── 4. BFS 定发育顺序（4 邻域），同时建父子关系 ──
        //
        // `parent` 存的是**父格在 `order` 里的下标**（不是 `cells` 里的下标）。
        // 这两套下标必须分清：BFS 的父子关系是"谁先把它发现的"，
        // 而 `prune_subtree` 只认 `order` 这一套。
        let mut order: Vec<usize> = Vec::with_capacity(cells.len());
        let mut parent: Vec<i16> = Vec::with_capacity(cells.len());
        // cells 下标 → order 下标
        let mut pos_in_order = vec![usize::MAX; cells.len()];
        let mut seen = vec![false; cells.len()];
        let mut queue = std::collections::VecDeque::new();
        seen[seed] = true;
        pos_in_order[seed] = 0;
        order.push(seed);
        parent.push(-1);
        queue.push_back(seed);
        while let Some(i) = queue.pop_front() {
            let (dx, dy) = (cells[i].dx, cells[i].dy);
            // **8 邻域**：身体是靠 `add_random_cell` 用 8 邻域长出来的，
            // 所以"连在一起"必须按 8 邻域算。用 4 邻域会把对角相接的格子
            // 当成孤岛丢掉——实测会把体型从 13 格砍到 2 格。
            for (ox, oy) in NEIGHBORS8 {
                let Some(&j) = occupied.get(&(dx + ox, dy + oy)) else {
                    continue;
                };
                if seen[j] {
                    continue;
                }
                seen[j] = true;
                let ord = order.len();
                order.push(j);
                parent.push(pos_in_order[i] as i16); // 发现它的那一格就是父格
                pos_in_order[j] = ord;
                queue.push_back(j);
            }
        }
        // 与胚不连通的孤岛直接丢弃（相当于凋亡），而不是强行补桥。
        // 上限取"调用方给的"和"硬上限"里更小的那个（0 = 只用硬上限）。
        let cap = if max_cells > 0 {
            max_cells.min(MAX_PLAN_CELLS)
        } else {
            MAX_PLAN_CELLS
        };
        let keep = order.len().min(cap);
        // ── 5. 归一化：把胚平移到 (0, 0) ──
        //
        // 出生时放下的那一格在 (0,0)，所以蓝图里的胚也必须在 (0,0)，
        // 否则 `pending()` 永远认为"胚还没长出来"，芽会卡死。
        let (sx, sy) = (cells[seed].dx, cells[seed].dy);
        let out_cells: Vec<BodyCell> = order[..keep]
            .iter()
            .map(|&i| {
                let c = cells[i];
                BodyCell::new(c.type_id, c.dx - sx, c.dy - sy, c.dir)
            })
            .collect();
        let out_depth: Vec<u8> = order[..keep].iter().map(|&i| depth[i]).collect();
        let out_parent: Vec<i16> = parent[..keep].to_vec();
        let dropped = vec![false; keep];
        let blocked = vec![0u8; keep];

        BodyPlan {
            cells: out_cells,
            parent: out_parent,
            depth: out_depth,
            dropped,
            blocked,
        }
    }

    /// 谱系指纹：结构哈希（确定性，跨平台一致）。
    ///
    /// 和 `Anatomy::fingerprint()` 的分工：
    /// - 这个哈希的是**基因组**（"谁从谁变来"）；
    /// - `Anatomy::composition` 量的是**形态**（"现在长什么样"）。
    /// 批次 32 已经把这双轨建好了，这里接上。
    pub fn hash(&self) -> String {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut eat = |v: u64| {
            h ^= v;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        };
        for seg in &self.segments {
            eat(seg.cells.len() as u64);
            eat(seg.repeat as u64);
            eat(seg.mirror as u64);
            eat(seg.rot as u64);
            eat(((seg.anchor.0 as u32 as u64) << 32) | (seg.anchor.1 as u32 as u64));
            for c in &seg.cells {
                eat(c.type_id as u64);
                eat(((c.dx as u32 as u64) << 32) | (c.dy as u32 as u64));
                eat(c.dir as u64);
            }
            eat(seg.children.len() as u64);
        }
        format!("g{h:016x}")
    }
}

/// 递归展开一个段：`base` 是这个段的世界锚点，`level` 是嵌套深度。
fn emit(
    seg: &Segment,
    level: u8,
    base: (i32, i32),
    out: &mut Vec<(i32, i32, u8, u8)>,
    depth: &mut Vec<u8>,
) {
    if seg.cells.is_empty() {
        return;
    }
    let repeat = seg.repeat.max(1) as i32;
    // 主轴：段的纵向跨度（+1）。重复份沿主轴堆叠，互不重叠。
    let min_dy = seg.cells.iter().map(|c| c.dy).min().unwrap_or(0);
    let max_dy = seg.cells.iter().map(|c| c.dy).max().unwrap_or(0);
    let span = (max_dy - min_dy + 1).max(1);
    let copies = if seg.mirror { 2 } else { 1 };

    for r in 0..repeat {
        for m in 0..copies {
            // 镜像：dx 取反（绕 y 轴翻）
            let flip = if m == 1 { -1 } else { 1 };
            let mut last = base;
            for c in &seg.cells {
                let (mut dx, mut dy) = (c.dx * flip, c.dy);
                // 旋转（90° 的倍数，顺时针）
                for _ in 0..(seg.rot & 3) {
                    let t = dx;
                    dx = -dy;
                    dy = t;
                }
                let x = base.0 + seg.anchor.0 + dx;
                let y = base.1 + seg.anchor.1 + dy + r * span;
                out.push((x, y, c.type_id, c.dir));
                depth.push(level);
                last = (x, y);
            }
            // 子段：从这一份的最后一格长出去
            for child in &seg.children {
                emit(child, level.saturating_add(1), last, out, depth);
            }
        }
    }
}

/// 两个构成之间的 Jaccard 距离（和 `Anatomy::composition_distance` 同一口径）。
pub fn composition_distance(a: &[u16; 17], b: &[u16; 17]) -> f32 {
    let mut inter = 0u32;
    let mut union = 0u32;
    for i in 0..17 {
        let (x, y) = (a[i] as u32, b[i] as u32);
        inter += x.min(y);
        union += x.max(y);
    }
    if union == 0 {
        return 0.0;
    }
    1.0 - inter as f32 / union as f32
}

/// 一格身体（胚）。
pub fn seed_anatomy(cell: BodyCell) -> Anatomy {
    let mut a = Anatomy::default();
    a.add_cell(cell.type_id, 0, 0, cell.dir);
    a
}

/// 胚是不是"能活"的：有生产者，或者至少不是纯嘴。
///
/// 阶段 0 不强制，只在 `describe()` 和诊断里用——真正的兜底是 `growth_seed_fraction`
/// 那份"胚乳"能量。
pub fn seed_is_viable(cell: BodyCell) -> bool {
    cell.type_id == ct::PRODUCER
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anatomy_of(cells: &[(u8, i32, i32)]) -> Anatomy {
        let mut a = Anatomy::default();
        for (t, dx, dy) in cells {
            a.add_cell(*t, *dx, *dy, 0);
        }
        a
    }

    /// 退化基因组展开必须是**恒等映射**（只是顺序被重排成 BFS）。
    #[test]
    fn degenerate_genome_expands_to_the_same_body() {
        let a = anatomy_of(&[
            (ct::PRODUCER, 0, 0),
            (ct::PRODUCER, 1, 0),
            (ct::MOUTH, 0, 1),
            (ct::MOVER, -1, 0),
        ]);
        let g = Genome::from_anatomy(&a);
        let plan = g.expand(0);
        assert_eq!(plan.len(), a.cells.len());
        for c in &a.cells {
            assert!(
                plan.cells.iter().any(|p| p.dx == c.dx && p.dy == c.dy && p.type_id == c.type_id),
                "展开后丢了格子 ({}, {})",
                c.dx,
                c.dy
            );
        }
        assert_eq!(plan.composition(), a.composition());
    }

    /// 胚在 0 号位，而且父格下标一定小于子格。
    #[test]
    fn plan_is_bfs_ordered_from_the_seed() {
        let a = anatomy_of(&[
            (ct::PRODUCER, 0, 0),
            (ct::PRODUCER, 1, 0),
            (ct::PRODUCER, 2, 0),
            (ct::MOUTH, 2, 1),
        ]);
        let plan = Genome::from_anatomy(&a).expand(0);
        assert_eq!((plan.cells[0].dx, plan.cells[0].dy), (0, 0), "0 号必须是胚");
        assert_eq!(plan.parent[0], -1, "胚没有父格");
        for i in 1..plan.len() {
            assert!(
                plan.parent[i] >= 0 && (plan.parent[i] as usize) < i,
                "第 {i} 格的父格下标必须更小"
            );
        }
        // BFS：距离 1 的格子必须排在距离 2 之前
        let d = |c: &BodyCell| c.dx.abs() + c.dy.abs();
        assert!(d(&plan.cells[1]) <= 1, "第二个应该是相邻格");
        assert_eq!(d(&plan.cells[3]), 3, "最远的那个排在最后");
    }

    /// 砍子树：被砍的那格和它下游都不再待长。
    #[test]
    fn prune_subtree_drops_the_whole_branch() {
        // 一条直线：0-1-2-3，砍掉 2 → 3 也没了
        let a = anatomy_of(&[
            (ct::PRODUCER, 0, 0),
            (ct::PRODUCER, 1, 0),
            (ct::PRODUCER, 2, 0),
            (ct::PRODUCER, 3, 0),
        ]);
        let mut plan = Genome::from_anatomy(&a).expand(0);
        assert_eq!(plan.pending(&anatomy_of(&[(ct::PRODUCER, 0, 0)])), 3);
        plan.prune_subtree(2);
        assert!(plan.dropped[2] && plan.dropped[3], "2 和 3 都该被砍");
        assert_eq!(
            plan.pending(&anatomy_of(&[(ct::PRODUCER, 0, 0)])),
            1,
            "只剩 1 号待长"
        );
    }

    /// 重复：同一段沿主轴堆叠成体节。
    #[test]
    fn repeat_stacks_copies_along_the_main_axis() {
        let mut seg = Segment::plain(vec![
            GeneCell::new(ct::PRODUCER, 0, 0, 0),
            GeneCell::new(ct::MOUTH, 1, 0, 0),
        ]);
        seg.repeat = 2;
        let plan = Genome { segments: vec![seg] }.expand(0);
        assert_eq!(plan.len(), 4, "2 格 × 重复 2");
        let ys: std::collections::BTreeSet<i32> = plan.cells.iter().map(|c| c.dy).collect();
        assert_eq!(ys.len(), 2, "两份应该占两行");
        // 每行都要有嘴（重复份是完整的）
        for y in ys {
            assert_eq!(
                plan.cells
                    .iter()
                    .filter(|c| c.dy == y && c.type_id == ct::MOUTH)
                    .count(),
                1,
                "第 {y} 行应该有一只嘴"
            );
        }
    }

    /// 镜像：绕 y 轴翻一份，**碰到轴上的格子会去重**（先到先得）。
    #[test]
    fn mirror_reflects_and_dedups_on_the_axis() {
        let mut seg = Segment::plain(vec![
            GeneCell::new(ct::PRODUCER, 0, 0, 0),
            GeneCell::new(ct::MOUTH, 1, 0, 0),
        ]);
        seg.mirror = true;
        let plan = Genome { segments: vec![seg] }.expand(0);
        // (0,0) 的镜像还是 (0,0) → 去重；(-1,0) 是新的 → 共 3 格
        assert_eq!(plan.len(), 3, "轴上的格子不该被算两次");
        assert!(
            plan.cells.iter().any(|c| c.dx == -1 && c.type_id == ct::MOUTH),
            "镜像份应该在负侧"
        );
    }

    /// 与胚不连通的孤岛**整块丢弃**（相当于凋亡），不强行补桥。
    ///
    /// 这条是"镜像必须贴着轴"的原因：段要是从 dx=1 开始，镜像份会落在
    /// dx<0 一侧、和原件之间隔着一条空隙，于是整个镜像份都是孤岛。
    #[test]
    fn disconnected_copies_are_dropped() {
        let mut seg = Segment::plain(vec![
            GeneCell::new(ct::PRODUCER, 1, 0, 0),
            GeneCell::new(ct::MOUTH, 2, 0, 0),
        ]);
        seg.mirror = true;
        let plan = Genome { segments: vec![seg] }.expand(0);
        assert_eq!(plan.len(), 2, "镜像份和原件之间有空隙，应该被丢掉");
        // 归一化之后胚在 (0,0)，所以原件那一侧占 dx ∈ {0,1}，
        // 镜像那一侧会在 dx ≤ -2 —— 一格都不该剩下。
        assert!(
            plan.cells.iter().all(|c| c.dx >= 0),
            "活下来的应该只有原件那一侧，实测 {:?}",
            plan.cells.iter().map(|c| c.dx).collect::<Vec<_>>()
        );
    }

    /// 超出上限时按 BFS 顺序截断（离胚近的留下）。
    #[test]
    fn max_cells_truncates_from_the_far_end() {
        let a = anatomy_of(&[
            (ct::PRODUCER, 0, 0),
            (ct::PRODUCER, 1, 0),
            (ct::PRODUCER, 2, 0),
            (ct::PRODUCER, 3, 0),
        ]);
        let plan = Genome::from_anatomy(&a).expand(2);
        assert_eq!(plan.len(), 2);
        assert_eq!((plan.cells[0].dx, plan.cells[1].dx), (0, 1), "留下的必须是近的");
    }

    /// 模块数：一团同类型 = 1，两种颜色分开 = 2。
    #[test]
    fn modules_counts_same_type_blocks() {
        let one = anatomy_of(&[(ct::PRODUCER, 0, 0), (ct::PRODUCER, 1, 0)]);
        assert_eq!(Genome::from_anatomy(&one).expand(0).modules(), 1);

        let two = anatomy_of(&[(ct::PRODUCER, 0, 0), (ct::MOUTH, 1, 0)]);
        assert_eq!(Genome::from_anatomy(&two).expand(0).modules(), 2);
    }

    /// 同基因组 → 同哈希；差一格 → 不同哈希。
    #[test]
    fn hash_is_stable_and_sensitive() {
        let a = anatomy_of(&[(ct::PRODUCER, 0, 0)]);
        let b = anatomy_of(&[(ct::PRODUCER, 0, 0), (ct::MOUTH, 1, 0)]);
        assert_eq!(Genome::from_anatomy(&a).hash(), Genome::from_anatomy(&a).hash());
        assert_ne!(Genome::from_anatomy(&a).hash(), Genome::from_anatomy(&b).hash());
    }
}
