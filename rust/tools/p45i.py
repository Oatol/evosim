import io

p = 'crates/core/src/world.rs'
s = io.open(p, encoding='utf-8').read()

old = '''        if grew {
            self.organisms[i].grow_stall = 0;
        } else if let Some(k) = blocked {
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
        } else {
            // 长不动（能量不够 / 还没轮到）→ 停滞计数
            self.organisms[i].grow_stall = self.organisms[i].grow_stall.saturating_add(1);
            let limit = self.config.life.growth_stall_mature;
            let reached = (limit > 0 && self.organisms[i].grow_stall >= limit)
                || self.organisms[i].grow_stall >= Self::GROW_STALL_BACKSTOP;
            // **提前成熟也要过年龄门槛**：否则「最短成长期」会被这条路径绕过去——
            // 那正好是最需要它的场景（贫斑块里芽长不动，本来会立刻变小体型）。
            if reached && old_enough {
                self.organisms[i].growth_plan = None;
                self.organisms[i].grow_stall = 0;
                // 成年即脱离母体：之后不再取能量，也不再占母体的芽位。
                self.organisms[i].parent_id = None;
                return;
            }
        }

        // 长满了**而且**够年龄 → 成年。'''

new = '''        // 蓝图每 tick 都要写回去：这一 tick 可能给若干格记了"受阻账"。
        let mut plan = plan;
        if grew {
            self.organisms[i].grow_stall = 0;
        } else if blocked.is_some() {
            // ── 受阻即凋亡（批次 45）──
            //
            // 旧版在这里"长到任意相邻空位、类型从蓝图里随机取"——那是当年为了
            // 不灭绝打的补丁（实测 1002/1003 两个种子会直接灭绝）。它的代价是
            // 双重的：形态被彻底打乱（发育这件事等于不存在），而且**体型变成
            // "芽周围有多少空地"的函数**——不可遗传。见 DESIGN.md §17.12。
            //
            // 凋亡是同一件事的有序版本：卡够 `GROW_BLOCK_TICKS` 的那一支
            // （连同它下游）被放弃，剩下的部分仍然是一个合法的、可辨识的身体。
            let mut to_prune: Vec<usize> = Vec::new();
            for k in 0..plan.cells.len() {
                if !plan.dropped[k] && plan.blocked[k] >= Self::GROW_BLOCK_TICKS {
                    to_prune.push(k);
                }
            }
            for k in to_prune {
                plan.prune_subtree(k);
            }
            self.organisms[i].grow_stall = 0;
        } else {
            // 长不动（能量不够 / 还没轮到）→ 停滞计数
            self.organisms[i].grow_stall = self.organisms[i].grow_stall.saturating_add(1);
            let limit = self.config.life.growth_stall_mature;
            let reached = (limit > 0 && self.organisms[i].grow_stall >= limit)
                || self.organisms[i].grow_stall >= Self::GROW_STALL_BACKSTOP;
            // **提前成熟也要过年龄门槛**：否则「最短成长期」会被这条路径绕过去——
            // 那正好是最需要它的场景（贫斑块里芽长不动，本来会立刻变小体型）。
            if reached && old_enough {
                self.organisms[i].growth_plan = None;
                self.organisms[i].grow_stall = 0;
                // 成年即脱离母体：之后不再取能量，也不再占母体的芽位。
                self.organisms[i].parent_id = None;
                return;
            }
        }
        self.organisms[i].growth_plan = Some(plan);

        // 长满了**而且**够年龄 → 成年。'''

assert old in s
s = s.replace(old, new, 1)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('ok')
