import io

# ── genome.rs：BodyPlan 加「受阻计时」 ──
p = 'crates/core/src/genome.rs'
s = io.open(p, encoding='utf-8').read()

s = s.replace('''    /// 受阻凋亡标记。被标 `true` 的格子永远不会长出来。
    pub dropped: Vec<bool>,''',
'''    /// 受阻凋亡标记。被标 `true` 的格子永远不会长出来。
    pub dropped: Vec<bool>,
    /// 每一格连续被挡了多少 tick。超过阈值就凋亡（见 `World::grow_organism`）。
    ///
    /// 为什么需要"等几 tick"而不是一受阻就砍：拥挤的芽常常是"这一格被母体占着，
    /// 下一 tick 母体挪开就好了"。一刀切会把整支身体砍掉（实测成体从 14 格掉到 2 格）。
    #[serde(default)]
    pub blocked: Vec<u8>,''')

s = s.replace('''        let dropped = vec![false; keep];

        BodyPlan {
            cells: out_cells,
            parent: out_parent,
            depth: out_depth,
            dropped,
        }''',
'''        let dropped = vec![false; keep];
        let blocked = vec![0u8; keep];

        BodyPlan {
            cells: out_cells,
            parent: out_parent,
            depth: out_depth,
            dropped,
            blocked,
        }''')
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('ok genome')

# ── world.rs：受阻计时 + 到点凋亡 ──
p = 'crates/core/src/world.rs'
s = io.open(p, encoding='utf-8').read()

# 常量
s = s.replace('''    /// 停滞兜底的硬上限（tick）。''',
'''    /// 一格连续被挡多少 tick 才判定"长不出来"（凋亡）。
    ///
    /// 3 tick 是个折中：短到不会让芽卡死（卡死 = 永远不能繁殖 = 大基因组被选择淘汰），
    /// 长到能等到"母体挪开 / 邻居死掉"这种常见情况。
    const GROW_BLOCK_TICKS: u8 = 3;

    /// 停滞兜底的硬上限（tick）。''', 1)

# 把 blocked 的语义从"立刻凋亡"改成"计时"
old = '''                if !self.cell_is_free_for_birth(idx) {
                    // **不要立刻凋亡**：先记下"最早受阻的那一格"，然后继续找
                    // 别的能长的地方。拥挤的芽常常是"这一格被母体/邻居占着，
                    // 但另一侧还有空间"——过早砍掉这一支，身体就只剩 1~2 格了
                    // （实测：一受阻就 break 的话，成体体型从 14 格掉到 2.5 格）。
                    if blocked.is_none() {
                        blocked = Some(k);
                    }
                    continue;
                }'''
new = '''                if !self.cell_is_free_for_birth(idx) {
                    // **不要立刻凋亡**：先给这一格记一笔"又卡了一 tick"，
                    // 然后继续找别的能长的地方。拥挤的芽常常是"这一格被母体占着，
                    // 下一 tick 母体挪开就好了"——一刀切会把整支身体砍掉
                    // （实测：一受阻就砍，成体体型从 14 格掉到 2 格）。
                    plan.blocked[k] = plan.blocked[k].saturating_add(1);
                    if blocked.is_none() {
                        blocked = Some(k);
                    }
                    continue;
                }'''
assert old in s
s = s.replace(old, new, 1)

# 到点凋亡：不再只砍第一个，而是把所有超时的都砍掉
old2 = '''        } else if let Some(k) = blocked {
            // ── 受阻即凋亡（批次 45）──
            //
            // 旧版在这里"长到任意相邻空位、类型从蓝图里随机取"——那是当年为了
            // 不灭绝打的补丁（实测 1002/1003 两个种子会直接灭绝），但它把形态
            // 彻底打乱，等于**发育这件事不存在**。
            //
            // 凋亡是同一件事的有序版本：长不出来的那一支（连同它下游）被放弃，
            // 剩下的部分仍然是一个合法的、可辨识的身体。蓝图删空 = 成年。
            let mut plan = plan;
            plan.prune_subtree(k);
            self.organisms[i].growth_plan = Some(plan);
            self.organisms[i].grow_stall = 0;
        } else {'''
new2 = '''        } else if blocked.is_some() {
            // ── 受阻即凋亡（批次 45）──
            //
            // 旧版在这里"长到任意相邻空位、类型从蓝图里随机取"——那是当年为了
            // 不灭绝打的补丁（实测 1002/1003 两个种子会直接灭绝），但它把形态
            // 彻底打乱，等于**发育这件事不存在**，而且它让体型变成
            // "芽周围有多少空地"的函数（**不可遗传**，见 DESIGN.md §17.12）。
            //
            // 凋亡是同一件事的有序版本：卡够 `GROW_BLOCK_TICKS` 的那一支
            // （连同它下游）被放弃，剩下的部分仍然是一个合法的、可辨识的身体。
            let mut plan = plan;
            let mut to_prune: Vec<usize> = Vec::new();
            for k in 0..plan.cells.len() {
                if !plan.dropped[k] && plan.blocked[k] >= Self::GROW_BLOCK_TICKS {
                    to_prune.push(k);
                }
            }
            for k in to_prune {
                plan.prune_subtree(k);
            }
            self.organisms[i].growth_plan = Some(plan);
            self.organisms[i].grow_stall = 0;
        } else {'''
assert old2 in s
s = s.replace(old2, new2, 1)

# 其它分支也要把计数写回去（有 blocked 计数变化时才写）
old3 = '''        if grew {
            self.organisms[i].grow_stall = 0;
        } else if blocked.is_some() {'''
new3 = '''        if grew {
            self.organisms[i].grow_stall = 0;
        } else if blocked.is_none() {
            // 没有任何一格是"相邻但被占"的：说明只是能量不够 / 还没轮到，
            // 这种情况下 `plan.blocked` 没变，不需要写回。
        }
        if grew {
            // 长成功了也要把计数写回去（前面几格可能刚记过账）
            self.organisms[i].growth_plan = Some(plan.clone());
        }
        if self.organisms[i].grow_stall == 0 && blocked.is_some() && !grew {'''
assert old3 in s
s = s.replace(old3, new3, 1)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('ok world (draft)')
