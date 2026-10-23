import io

p = 'crates/core/src/mutation.rs'
s = io.open(p, encoding='utf-8').read()

anchor = '''    /// 到上限后缩小体型的算子必须还能用，否则个体一旦长满就再也回不来了。'''
tests = '''    // ── 批次 46：段级算子 ──

    use crate::cell_type::ct;
    use crate::genome::{GeneCell, Genome, Segment, MAX_PLAN_CELLS};

    fn one_segment() -> Genome {
        Genome {
            segments: vec![Segment::plain(vec![
                GeneCell::new(ct::PRODUCER, 0, 0, 0),
                GeneCell::new(ct::MOUTH, 1, 0, 0),
            ])],
        }
    }

    /// 段级算子必须守住两条不变量：**不突破细胞预算**、**不把段删光**。
    #[test]
    fn segment_ops_respect_the_budget_and_never_empty_the_genome() {
        let mut g = one_segment();
        let mut rng = Rng::new(9);
        for i in 0..800 {
            segment_op(&mut g, &mut rng);
            assert!(
                plan_size(&g) <= MAX_PLAN_CELLS,
                "第 {i} 次段级操作把体型推到 {} 格（上限 {MAX_PLAN_CELLS}）",
                plan_size(&g)
            );
            assert!(!g.segments.is_empty(), "第 {i} 次把段删光了");
            assert!(
                g.segments.iter().all(|s| s.repeat >= 1),
                "第 {i} 次把 repeat 归零了（会展开成空身体）"
            );
        }
    }

    /// `repeat` 与 `mirror` 必须真的让身体变大——这是阶段 1 的全部意义。
    #[test]
    fn repeat_and_mirror_grow_the_body() {
        let mut g = one_segment();
        let base = g.expand(0).len();
        assert_eq!(base, 2);

        g.segments[0].repeat = 2;
        let doubled = g.expand(0).len();
        assert_eq!(doubled, 4, "repeat 2 应该让体型翻倍");

        g.segments[0].mirror = true;
        let mirrored = g.expand(0).len();
        assert!(
            mirrored > doubled,
            "镜像应该再加一份（轴上的格子会去重），实测 {mirrored}"
        );
    }

    /// 插段 = 复制一段叠在下方，身体变长。
    #[test]
    fn insert_segment_stacks_below_the_body() {
        let mut g = one_segment();
        let before = g.expand(0).len();
        let mut copy = g.segments[0].clone();
        copy.anchor = (0, bbox_max_dy(&g) + 1);
        g.segments.push(copy);
        let after = g.expand(0).len();
        assert_eq!(after, before * 2, "复制一段应该让身体长一倍");
        // 两段必须在 y 上错开
        let ys: std::collections::BTreeSet<i32> = g.expand(0).cells.iter().map(|c| c.dy).collect();
        assert_eq!(ys.len(), 2, "两段应该占两行");
    }

    /// 段级算子跑很多次之后，体型应该能明显长起来（不是原地打转）。
    #[test]
    fn segment_ops_can_grow_the_plan_over_time() {
        let mut g = one_segment();
        let mut rng = Rng::new(17);
        let mut max_seen = g.expand(0).len();
        for _ in 0..600 {
            segment_op(&mut g, &mut rng);
            max_seen = max_seen.max(g.expand(0).len());
        }
        assert!(
            max_seen >= 8,
            "600 次段级操作都没让身体长到 8 格（最大 {max_seen}）——算子没在起作用"
        );
    }

''' + anchor
assert anchor in s
s = s.replace(anchor, tests, 1)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('ok')
