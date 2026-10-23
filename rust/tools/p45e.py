import io

# ── genome.rs：BFS 与模块统计改用 8 邻域 ──
p = 'crates/core/src/genome.rs'
s = io.open(p, encoding='utf-8').read()

old_bfs = '''        while let Some(i) = queue.pop_front() {
            let (dx, dy) = (cells[i].dx, cells[i].dy);
            for (ox, oy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let Some(&j) = occupied.get(&(dx + ox, dy + oy)) else {
                    continue;
                };'''
new_bfs = '''        while let Some(i) = queue.pop_front() {
            let (dx, dy) = (cells[i].dx, cells[i].dy);
            // **8 邻域**：身体是靠 `add_random_cell` 用 8 邻域长出来的，
            // 所以"连在一起"必须按 8 邻域算。用 4 邻域会把对角相接的格子
            // 当成孤岛丢掉——实测会把体型从 13 格砍到 2 格。
            for (ox, oy) in NEIGHBORS8 {
                let Some(&j) = occupied.get(&(dx + ox, dy + oy)) else {
                    continue;
                };'''
assert old_bfs in s
s = s.replace(old_bfs, new_bfs, 1)

old_mod = '''                for (ox, oy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    if let Some(&j) = at.get(&(dx + ox, dy + oy)) {'''
new_mod = '''                for (ox, oy) in NEIGHBORS8 {
                    if let Some(&j) = at.get(&(dx + ox, dy + oy)) {'''
assert old_mod in s
s = s.replace(old_mod, new_mod, 1)

# 常量
old_c = '''/// 基因组里的一个格子。'''
new_c = '''/// 八邻域（和 `mutation.rs` 的 `NEIGHBORS_8` 同序）。
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

/// 基因组里的一个格子。'''
assert old_c in s
s = s.replace(old_c, new_c, 1)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('ok genome')

# ── world.rs：生长相邻判定改 8 邻域 ──
p = 'crates/core/src/world.rs'
s = io.open(p, encoding='utf-8').read()
old_adj = '''                // 必须和现有身体四邻相邻（否则身体会裂成互不相连的两块）
                let adjacent = self.organisms[i]
                    .anatomy
                    .cells
                    .iter()
                    .any(|c| (c.dx - cell.dx).abs() + (c.dy - cell.dy).abs() == 1);'''
new_adj = '''                // 必须和现有身体**相邻**（否则身体会裂成互不相连的两块）。
                // 用 8 邻域而不是 4 邻域：身体本来就是按 8 邻域拼出来的
                // （`add_random_cell` 从 8 个方向挑），对角相接也算连着。
                let adjacent = self.organisms[i]
                    .anatomy
                    .cells
                    .iter()
                    .any(|c| (c.dx - cell.dx).abs() <= 1 && (c.dy - cell.dy).abs() <= 1);'''
assert old_adj in s
s = s.replace(old_adj, new_adj, 1)

# 测试：BFS 顺序断言放宽到切比雪夫距离
old_t = '''        // 第二格必须是**距离 1** 的相邻格，不能是距离 3 的尾巴
        let has_near = w.organisms[0]
            .anatomy
            .cells
            .iter()
            .any(|c| c.dx.abs() + c.dy.abs() == 1);'''
new_t = '''        // 第二格必须是**紧邻**的格子（切比雪夫距离 1），不能是距离 3 的尾巴
        let has_near = w.organisms[0]
            .anatomy
            .cells
            .iter()
            .any(|c| c.dx.abs() <= 1 && c.dy.abs() <= 1 && (c.dx != 0 || c.dy != 0));'''
assert old_t in s
s = s.replace(old_t, new_t, 1)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('ok world')
