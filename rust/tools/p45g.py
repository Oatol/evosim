import io

p = 'crates/core/src/world.rs'
s = io.open(p, encoding='utf-8').read()
anchor = '''    /// 没被挡住时，身体必须**按 BFS 顺序**由内向外长：先长靠近胚的格子。'''
test = '''    /// `mutate_genome` 必须真的能往基因组里加格子。
    ///
    /// 这条是"体型塌到 2 格"那次事故的守门：基因组加不了格子，
    /// 蓝图就永远停在出生时那么大，整个演化被冻住（实测成体从 14 格掉到 2.5 格）。
    #[test]
    fn mutate_genome_can_add_cells() {
        let (mut w, id) = tiny_world(&[(ct::PRODUCER, 0, 0)]);
        let i = w.organism_index(id).unwrap();
        w.config.mutation.add_prob = 100.0;
        w.config.mutation.change_prob = 0.0;
        w.config.mutation.remove_prob = 0.0;
        w.organisms[i].mutability = 100.0;

        let mut genome = w.organisms[i].effective_genome();
        assert_eq!(genome.expand(0).len(), 1, "起始应该只有 1 格");
        let mut rng = Rng::new(3);
        let mut changed_any = false;
        for _ in 0..40 {
            if mutate_genome(&mut genome, &w.organisms[i], &mut rng, &w.config) {
                changed_any = true;
            }
        }
        assert!(changed_any, "mutate_genome 一次都没生效");
        assert!(
            genome.expand(0).len() > 1,
            "加了 40 次细胞之后基因组还是 1 格——加细胞这条路径断了"
        );
    }

''' + anchor
assert anchor in s
s = s.replace(anchor, test, 1)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('ok')
