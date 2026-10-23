import io

# ── 1) genome.rs：把胚平移到 (0,0)，保证出生时放的那一格就是胚 ──
p = 'crates/core/src/genome.rs'
s = io.open(p, encoding='utf-8').read()

old = '''        let out_cells: Vec<BodyCell> = order[..keep].iter().map(|&i| cells[i]).collect();'''
new = '''        // ── 5. 归一化：把胚平移到 (0, 0) ──
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
            .collect();'''
assert old in s
s = s.replace(old, new, 1)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('ok genome')

# ── 2) world.rs：先找能长的地方，只有全都长不了才凋亡 ──
p = 'crates/core/src/world.rs'
s = io.open(p, encoding='utf-8').read()

old2 = '''                if !self.cell_is_free_for_birth(idx) {
                    // 按 BFS 顺序，第一个受阻的格子 = "最早长不出来的那一支"
                    blocked = Some(k);
                    break;
                }'''
new2 = '''                if !self.cell_is_free_for_birth(idx) {
                    // **不要立刻凋亡**：先记下"最早受阻的那一格"，然后继续找
                    // 别的能长的地方。拥挤的芽常常是"这一格被母体/邻居占着，
                    // 但另一侧还有空间"——过早砍掉这一支，身体就只剩 1~2 格了
                    // （实测：一受阻就 break 的话，成体体型从 14 格掉到 2.5 格）。
                    if blocked.is_none() {
                        blocked = Some(k);
                    }
                    continue;
                }'''
assert old2 in s
s = s.replace(old2, new2, 1)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('ok world')
