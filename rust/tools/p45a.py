import io, re

def rd(p): return io.open(p, encoding='utf-8').read()
def wr(p, s): io.open(p, 'w', encoding='utf-8', newline='').write(s)

# ══════════════════════════ mutation.rs ══════════════════════════
p = 'crates/core/src/mutation.rs'
s = rd(p)

# 1) 用 mutate_genome 取代 mutate_organism（阶段 0：借道 Anatomy 的算子）
start = s.index('/// 对子代应用全部变异。就地修改 child。')
end = s.index('/// 结构变异：三个算子各自独立判定')
new_block = '''/// 对**基因组**应用全部变异（批次 45）。
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

    let Some(seg) = genome.segments.first_mut() else {
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
}

'''
s = s[:start] + new_block + s[end:]
wr(p, s)
print('ok mutation.rs')

# ══════════════════════════ world.rs ══════════════════════════
p = 'crates/core/src/world.rs'
s = rd(p)

# 2) kin_penalty / species_clusters 改用 expressed_composition
s = s.replace('''            let d = self.organisms[pred_i]
                .genome()
                .composition_distance(self.organisms[vi].genome());''',
'''            let a = self.organisms[pred_i].expressed_composition();
            let b = self.organisms[vi].expressed_composition();
            let d = crate::genome::composition_distance(&a, &b);''', 1)

s = s.replace('            let comp = org.genome().composition();',
              '            let comp = org.expressed_composition();', 1)

# 3) grow_organism 整体重写
g0 = s.index('    fn grow_organism(&mut self, i: usize) {')
g1 = s.index('    /// 环境温度造成的伤害：**表面积/体积比**决定谁扛得住。')
new_grow = '''    fn grow_organism(&mut self, i: usize) {
        let Some(plan) = self.organisms[i].growth_plan.clone() else {
            return;
        };
        // 年龄只在"还是芽"的时候涨。它是「最短成长期」的计时器：
        // 1 格目标的芽出生时就已经"长满"了，没有它就会当 tick 成年、立刻繁殖，
        // 于是「小」变成免费策略，体型在**任何**环境里都涨不上去（见批次 13 实测）。
        self.organisms[i].grow_age = self.organisms[i].grow_age.saturating_add(1);
        // 出芽：先向母体取一份能量（内部转移，不进热池）。
        if let Some(pid) = self.organisms[i].parent_id {
            self.feed_from_parent(i, pid);
        }
        let min_age = self.config.life.growth_min_ticks;
        let old_enough = self.organisms[i].grow_age >= min_age;
        let cost = self.config.life.growth_cost.max(0.0);
        // 能量不够就长不动 —— 但**不能直接 return**：要走下面的停滞计数，
        // 否则"饿着的芽"永远不会被判定为"该提前成熟"。
        let can_pay = self.organisms[i].energy >= cost;

        let mut grew = false;
        let mut blocked: Option<usize> = None;
        if can_pay {
            // **按发育顺序（BFS）**逐格尝试：离胚近的先长，身体由内向外展开。
            //
            // 旧版是"遍历蓝图，谁相邻就长谁"——顺序由 `Vec` 的存储顺序决定，
            // 于是芽可能先长出边缘的一根手指，而核心（生产者）迟迟不长，
            // 结果是幼体期没有收入、活活饿死。BFS 顺序把这个隐患从根上去掉了。
            for k in 0..plan.cells.len() {
                if plan.dropped[k] {
                    continue;
                }
                let cell = plan.cells[k];
                // 已经长出来了？
                if self.organisms[i].anatomy.cell_at(cell.dx, cell.dy).is_some() {
                    continue;
                }
                // 必须和现有身体四邻相邻（否则身体会裂成互不相连的两块）
                let adjacent = self.organisms[i]
                    .anatomy
                    .cells
                    .iter()
                    .any(|c| (c.dx - cell.dx).abs() + (c.dy - cell.dy).abs() == 1);
                if !adjacent {
                    continue;
                }
                let (c, r) = (
                    self.organisms[i].real_col(&cell),
                    self.organisms[i].real_row(&cell),
                );
                let Some(idx) = self.grid.index(c, r) else {
                    continue;
                };
                if !self.cell_is_free_for_birth(idx) {
                    // 按 BFS 顺序，第一个受阻的格子 = "最早长不出来的那一支"
                    blocked = Some(k);
                    break;
                }
                self.organisms[i]
                    .anatomy
                    .add_cell(cell.type_id, cell.dx, cell.dy, cell.dir);
                self.write_grid(i);
                grew = true;
                break;
            }
        }

        if grew && cost > 0.0 {
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

        if grew {
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
                || self.organisms[i].grow_stall >= GROW_STALL_BACKSTOP;
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
s = s[:g0] + new_grow + s[g1:]

# 4) 常量：停滞兜底
s = s.replace('''    /// 芽长一格。''',
'''    /// 停滞兜底的硬上限（tick）。
    ///
    /// `growth_stall_mature` 默认是 0（关闭），而凋亡虽然能保证"有进展"，
    /// 却挡不住"能量长期不够"这种停滞。留一个硬上限，避免出现
    /// **永远长不大、也永远不能繁殖的僵尸芽**。
    const GROW_STALL_BACKSTOP: u32 = 300;

    /// 芽长一格。''', 1)

# 5) 出芽：基因组 → 变异 → 展开
old_bud = '''        child.parent_id = Some(id);
        child.growth_plan = Some(self.organisms[i].anatomy.clone());

        // 变异作用在**目标蓝图**上（同成长模式，否则体型永远变不了）
        child.anatomy = child.growth_plan.take().unwrap();
        {
            let parent = &self.organisms[i];
            let cfg = &self.config;
            mutate_organism(&mut child, parent, &mut self.rng, cfg);
        }
        let plan = child.anatomy.clone();
        if plan.cells.is_empty() {
            return false;
        }
        child.anatomy = Anatomy::default();
        let seed = plan.cell_at(0, 0).copied().unwrap_or(plan.cells[0]);
        child.anatomy.add_cell(seed.type_id, 0, 0, seed.dir);
        child.growth_plan = Some(plan);'''
new_bud = '''        child.parent_id = Some(id);

        // 变异作用在**基因组**上，再展开成蓝图（批次 45）。
        // 这样"变异"和"发育"彻底分开：基因组决定**长成什么样**，
        // 蓝图决定**按什么顺序长**。旧版把变异作用在身体上、再反推蓝图，
        // 顺序信息在这一步就丢了。
        {
            let World {
                organisms,
                rng,
                config,
                ..
            } = self;
            let parent = &organisms[i];
            let mut genome = parent.effective_genome();
            let changed = mutate_genome(&mut genome, parent, rng, config);
            mutate_brain(&mut child.brain, rng, &config.mutation);
            child.mutability = if changed {
                (parent.mutability + rng.gaussian(0.0, config.mutation.mutability_sigma))
                    .clamp(1.0, 100.0)
            } else {
                parent.mutability
            };
            child.parent_fingerprint = parent.species_fingerprint.clone();
            child.species_fingerprint = Some(std::sync::Arc::from(genome.hash().as_str()));
            child.genome = genome;
        }
        let plan = child.genome.expand(self.config.life.max_body_cells);
        let Some(seed) = plan.seed() else {
            return false;
        };
        child.anatomy = crate::genome::seed_anatomy(seed);
        child.growth_plan = Some(plan);'''
assert old_bud in s
s = s.replace(old_bud, new_bud, 1)

# 6) 普通繁殖：同一套（基因组 → 变异 → 展开）
old_rep = '''        let growth = self.config.life.growth_enabled;
        if growth {
            // 芽：只有 1 格，外加一份「要长成什么样」的蓝图。
            child.growth_plan = Some(self.organisms[i].anatomy.clone());
        } else {
            child.anatomy.copy_from(&self.organisms[i].anatomy);
        }

        {
            let parent = &self.organisms[i];
            let cfg = &self.config;
            if growth {
                // **变异必须作用在目标蓝图上**，不能只变异芽那 1 格——
                // 否则体型永远变不了，演化直接停摆。
                // 做法：临时把蓝图塞进 `anatomy` 让 `mutate_organism` 去改，
                // 改完再把蓝图拆出来、重新取 1 格当种子。
                child.anatomy = child.growth_plan.take().unwrap();
                mutate_organism(&mut child, parent, &mut self.rng, cfg);
                let plan = child.anatomy.clone();
                if plan.cells.is_empty() {
                    // 变异把身体删空了：这个后代生不下来
                    if strict {
                        self.charge_repro(i, 0.0);
                    }
                    return false;
                }
                child.anatomy = Anatomy::default();
                let seed = plan.cell_at(0, 0).copied().unwrap_or(plan.cells[0]);
                child.anatomy.add_cell(seed.type_id, 0, 0, seed.dir);
                child.growth_plan = Some(plan);
            } else {
                mutate_organism(&mut child, parent, &mut self.rng, cfg);
            }
        }'''
new_rep = '''        let growth = self.config.life.growth_enabled;

        // 变异作用在**基因组**上（批次 45），和出芽同一条路径。
        {
            let World {
                organisms,
                rng,
                config,
                ..
            } = self;
            let parent = &organisms[i];
            let mut genome = parent.effective_genome();
            let changed = mutate_genome(&mut genome, parent, rng, config);
            mutate_brain(&mut child.brain, rng, &config.mutation);
            child.mutability = if changed {
                (parent.mutability + rng.gaussian(0.0, config.mutation.mutability_sigma))
                    .clamp(1.0, 100.0)
            } else {
                parent.mutability
            };
            child.parent_fingerprint = parent.species_fingerprint.clone();
            child.species_fingerprint = Some(std::sync::Arc::from(genome.hash().as_str()));
            child.genome = genome;
        }
        let plan = child.genome.expand(self.config.life.max_body_cells);
        let Some(seed) = plan.seed() else {
            // 变异把基因组删空了：这个后代生不下来
            if strict {
                self.charge_repro(i, 0.0);
            }
            return false;
        };
        if growth {
            // 芽：只有 1 格（胚），外加一份带**发育顺序**的蓝图。
            child.anatomy = crate::genome::seed_anatomy(seed);
            child.growth_plan = Some(plan);
        } else {
            // 不启用生长：直接展开成成体（基因组仍然生效，只是没有渐进发育）
            child.anatomy = plan.to_anatomy();
        }'''
assert old_rep in s
s = s.replace(old_rep, new_rep, 1)

wr(p, s)
print('ok world.rs')
