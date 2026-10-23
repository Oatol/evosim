import io

p = 'crates/core/src/genome.rs'
s = io.open(p, encoding='utf-8').read()

old = '''        // ── 4. BFS 定发育顺序（4 邻域），同时建父子关系 ──
        let mut order: Vec<usize> = Vec::with_capacity(cells.len());
        let mut parent: Vec<i16> = Vec::with_capacity(cells.len());
        let mut seen = vec![false; cells.len()];
        let mut queue = std::collections::VecDeque::new();
        seen[seed] = true;
        order.push(seed);
        parent.push(-1);
        queue.push_back(seed);
        while let Some(i) = queue.pop_front() {
            let (dx, dy) = (cells[i].dx, cells[i].dy);
            for (ox, oy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let Some(&j) = occupied.get(&(dx + ox, dy + oy)) else {
                    continue;
                };
                if seen[j] {
                    continue;
                }
                seen[j] = true;
                order.push(j);
                parent.push(order.len() as i16 - 2); // 父格在 order 里的下标
                queue.push_back(j);
            }
        }'''

new = '''        // ── 4. BFS 定发育顺序（4 邻域），同时建父子关系 ──
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
            for (ox, oy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
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
        }'''

assert old in s
s = s.replace(old, new, 1)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('ok')
