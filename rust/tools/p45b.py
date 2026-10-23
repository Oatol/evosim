import io

# ── 1) genome 测试：镜像与轴重合会去重，改用不碰轴的段 ──
p = 'crates/core/src/genome.rs'
s = io.open(p, encoding='utf-8').read()
old = '''    /// 段重复与镜像（阶段 1 会用到，这里先把解释器钉死）。
    #[test]
    fn repeat_and_mirror_expand_as_expected() {
        let mut seg = Segment::plain(vec![
            GeneCell::new(ct::PRODUCER, 0, 0, 0),
            GeneCell::new(ct::MOUTH, 1, 0, 0),
        ]);
        seg.repeat = 2;
        seg.mirror = true;
        let g = Genome { segments: vec![seg] };
        let plan = g.expand(0);
        // 2 份 × 2 镜像 × 2 格 = 8
        assert_eq!(plan.len(), 8, "重复 2 × 镜像 2 × 2 格");
        // 镜像应该出现在 dx = -1
        assert!(plan.cells.iter().any(|c| c.dx == -1 && c.type_id == ct::MOUTH));
    }'''
new = '''    /// 段重复与镜像（阶段 1 会用到，这里先把解释器钉死）。
    ///
    /// 段故意**不碰镜像轴**（dx 从 1 开始）：碰轴的话镜像份会和原件重叠，
    /// 而重叠的规则是"先到先得"，格数就对不上了。
    #[test]
    fn repeat_and_mirror_expand_as_expected() {
        let mut seg = Segment::plain(vec![
            GeneCell::new(ct::PRODUCER, 1, 0, 0),
            GeneCell::new(ct::MOUTH, 2, 0, 0),
        ]);
        seg.repeat = 2;
        seg.mirror = true;
        let g = Genome { segments: vec![seg] };
        let plan = g.expand(0);
        // 2 份 × 2 镜像 × 2 格 = 8，互不重叠
        assert_eq!(plan.len(), 8, "重复 2 × 镜像 2 × 2 格");
        // 镜像出现在负侧
        assert!(
            plan.cells.iter().any(|c| c.dx == -2 && c.type_id == ct::MOUTH),
            "镜像份应该在 dx = -2"
        );
        // 重复份沿主轴平移，所以应该有两组不同的 y
        let ys: std::collections::BTreeSet<i32> = plan.cells.iter().map(|c| c.dy).collect();
        assert_eq!(ys.len(), 2, "重复 2 份应该占两行");
    }

    /// 镜像份与原件重叠时，**先到先得**（不报错、不叠加）。
    #[test]
    fn overlapping_copies_keep_the_first() {
        let mut seg = Segment::plain(vec![
            GeneCell::new(ct::PRODUCER, 0, 0, 0),
            GeneCell::new(ct::MOUTH, 1, 0, 0),
        ]);
        seg.mirror = true;
        let plan = Genome { segments: vec![seg] }.expand(0);
        // (0,0) 的镜像还是 (0,0) → 去重；共 3 格
        assert_eq!(plan.len(), 3);
    }'''
assert old in s
s = s.replace(old, new, 1)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('ok genome test')

# ── 2) world 测试：兜底 → 凋亡 ──
p = 'crates/core/src/world.rs'
s = io.open(p, encoding='utf-8').read()
old2 = '''    /// 目标格全被占时，芽要能**退化成长到相邻空位**，而不是永远卡在 1 格。
    ///
    /// 这条是实测救回来的：严格按蓝图长的话，芽生在母体旁边那片拥挤区域，
    /// 目标格大概率被邻居占着，于是永远长不大、永远不能繁殖——1002/1003 直接灭绝。
    #[test]
    fn growth_falls_back_to_any_free_neighbour() {
        let (mut w, _) = tiny_world(&[(ct::MOUTH, 0, 0)]);
        w.config.life.growth_enabled = true;
        // 这几条量的是细胞层面的机制（长一格 / 血量 / 兜底），不是年龄门槛，
        // 所以显式把最短成长期关掉，否则"出生即成年"的前提不成立。
        w.config.life.growth_min_ticks = 0;
        w.config.life.growth_cost = 1.0;
        let mut plan = w.organisms[0].anatomy.clone();
        plan.add_cell(ct::PRODUCER, 1, 0, 0);
        w.organisms[0].growth_plan = Some(crate::genome::Genome::from_anatomy(&plan).expand(0));
        w.organisms[0].energy = 5.0;
        // 把蓝图指定的那一格 (1,0) 用墙堵死
        let org = &w.organisms[0];
        let (c, r) = (org.col + 1, org.row);
        let idx = w.grid.index(c, r).unwrap();
        w.set_cell_at(idx, ct::WALL, -1);

        w.grow_organism(0);
        assert_eq!(
            w.organisms[0].cell_count(),
            2,
            "目标格被堵死时应该退化长到别的相邻空位"
        );
    }'''
new2 = '''    /// 目标格被占时**不再瞎猜**：砍掉那一支（凋亡），以当前体型成年。
    ///
    /// 旧行为是"随机挑个相邻空位、类型从蓝图里随机取"——那是当年为了不灭绝
    /// 打的补丁（严格按蓝图长会让 1002/1003 两个种子直接灭绝），代价是形态被
    /// 彻底打乱，等于发育这件事不存在。
    ///
    /// 新行为保留了"不灭绝"，但换成了有序版本：长不出来的部分被放弃，
    /// 剩下的仍然是一个合法的、可辨识的身体。这条测试钉的就是
    /// **不许再往随机空位塞格子**。
    #[test]
    fn blocked_growth_prunes_the_branch_instead_of_guessing() {
        let (mut w, _) = tiny_world(&[(ct::MOUTH, 0, 0)]);
        w.config.life.growth_enabled = true;
        w.config.life.growth_min_ticks = 0;
        w.config.life.growth_cost = 1.0;
        let mut plan = w.organisms[0].anatomy.clone();
        plan.add_cell(ct::PRODUCER, 1, 0, 0);
        w.organisms[0].growth_plan = Some(crate::genome::Genome::from_anatomy(&plan).expand(0));
        w.organisms[0].energy = 5.0;
        // 把蓝图指定的那一格 (1,0) 用墙堵死
        let org = &w.organisms[0];
        let (c, r) = (org.col + 1, org.row);
        let idx = w.grid.index(c, r).unwrap();
        w.set_cell_at(idx, ct::WALL, -1);

        w.grow_organism(0);
        assert_eq!(
            w.organisms[0].cell_count(),
            1,
            "长不出来的那一支应该被放弃，而不是长到随机空位"
        );
        assert!(
            w.organisms[0].growth_plan.is_none(),
            "蓝图删空 = 成年（发育失败但活着，不是死亡）"
        );
        assert!(w.organisms[0].alive, "凋亡不该杀死个体");
    }

    /// 没被挡住时，身体必须**按 BFS 顺序**由内向外长：先长靠近胚的格子。
    #[test]
    fn growth_follows_the_developmental_order() {
        let (mut w, _) = tiny_world(&[(ct::PRODUCER, 0, 0)]);
        w.config.life.growth_enabled = true;
        w.config.life.growth_min_ticks = 0;
        w.config.life.growth_cost = 0.0;
        // 蓝图：0-(1,0)-(2,0)-(3,0) 一条线，外加 (0,1)
        let mut plan = w.organisms[0].anatomy.clone();
        plan.add_cell(ct::PRODUCER, 1, 0, 0);
        plan.add_cell(ct::PRODUCER, 2, 0, 0);
        plan.add_cell(ct::PRODUCER, 3, 0, 0);
        plan.add_cell(ct::PRODUCER, 0, 1, 0);
        w.organisms[0].growth_plan = Some(crate::genome::Genome::from_anatomy(&plan).expand(0));
        w.organisms[0].energy = 10.0;

        w.grow_organism(0);
        assert_eq!(w.organisms[0].cell_count(), 2, "一 tick 只长一格");
        // 第二格必须是**距离 1** 的相邻格，不能是距离 3 的尾巴
        let has_near = w.organisms[0]
            .anatomy
            .cells
            .iter()
            .any(|c| c.dx.abs() + c.dy.abs() == 1);
        assert!(has_near, "先长出来的必须是离胚最近的格子");
    }'''
assert old2 in s
s = s.replace(old2, new2, 1)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('ok world tests')
