import io

p = 'crates/core/src/genome.rs'
s = io.open(p, encoding='utf-8').read()

old = '''    /// 段重复与镜像（阶段 1 会用到，这里先把解释器钉死）。
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

new = '''    /// 重复：同一段沿主轴堆叠成体节。
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
        assert!(
            plan.cells.iter().all(|c| c.dx > 0),
            "活下来的应该只有原件那一侧"
        );
    }'''

assert old in s
s = s.replace(old, new, 1)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('ok')
