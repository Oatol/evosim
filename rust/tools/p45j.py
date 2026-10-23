import io

p = 'crates/core/src/world.rs'
s = io.open(p, encoding='utf-8').read()

start = s.index('        if grew && cost > 0.0 {')
end = s.index('    /// 环境温度造成的伤害：**表面积/体积比**决定谁扛得住。')

new_tail = '''        if grew && cost > 0.0 {
            let paid: f64;
            {
                let org = &mut self.organisms[i];
                paid = cost.min(org.energy.max(0.0));
                org.energy -= cost;
                if org.energy < 0.0 {
                    org.energy = 0.0;
                }
            }
            // 生长耗能也是"离开生物圈"，进热池——否则账上会多出一块。
            self.heat_pool += paid;
        }

        // 蓝图每 tick 都要写回去：这一 tick 可能给若干格记了"受阻账"。
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

        // 长满了**而且**够年龄 → 成年。
        // "长满"的口径是 `pending == 0`：既包含"真的长完了"，也包含"剩下的都凋亡了"。
        let done = self
            .organisms[i]
            .growth_plan
            .as_ref()
            .map(|p| p.pending(&self.organisms[i].anatomy) == 0)
            .unwrap_or(true);
        if done && old_enough {
            self.organisms[i].growth_plan = None;
            self.organisms[i].grow_stall = 0;
            self.organisms[i].parent_id = None;
        }
    }

'''

s = s[:start] + new_tail + s[end:]
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('ok')
