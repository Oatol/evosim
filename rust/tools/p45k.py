import io

p = 'crates/core/src/mutation.rs'
s = io.open(p, encoding='utf-8').read()

old = '''fn add_random_cell(anatomy: &mut Anatomy, rng: &mut Rng) -> bool {
    for _ in 0..MAX_ADD_ATTEMPTS {
        let base_idx = match anatomy.random_cell_index(rng) {
            Some(i) => i,
            None => return false,
        };'''
new = '''/// 从"**表面**"的细胞里随机挑一个（有至少一个空邻居的）。
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
        };'''
assert old in s
s = s.replace(old, new, 1)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('ok')
