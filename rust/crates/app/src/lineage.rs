//! 谱系树：把物种按祖先关系排成"时间轴上的树"。
//!
//! 物种面板原本是一个按种群数排序的甘特图 —— 一行一个物种、横条是存续区间。
//! 要看出"谁是谁的后代、哪一支活下来了"，光有横条不够：父子可能隔着几百行，
//! 硬画连线就是一团交叉。
//!
//! 做法是**按祖先关系做深度优先排序**：同一支的后代紧挨在父物种下面。
//! 排序对了，连线就退化成"每个非根物种一根竖线"——从父物种那一行、在子物种
//! 出现的那个 tick 上竖下来，接到自己横条的左端。这正是时间标定的系统发生树
//! （chronogram）的样子，而且天然不会交叉。
//!
//! 这个模块只做**排序和过滤**，不碰 egui，方便单测。

use std::collections::HashMap;

use evosim_core::species::SpeciesRegistry;

/// 一行 = 一个可见物种
pub struct Row {
    /// `reg.list` 下标
    pub idx: usize,
    /// 从根起算的代数（缩进用）
    pub depth: u16,
    /// 父物种在 `rows` 里的行号；None = 这棵可见树的根
    pub parent: Option<usize>,
}

pub struct Tree {
    pub rows: Vec<Row>,
    /// 被过滤掉的物种数
    pub hidden: usize,
}

pub struct Opts {
    /// true = 谱系树（DFS 排序 + 连线）；false = 老的按种群数排序的平铺视图
    pub tree: bool,
    /// 只留活着的物种（以及它们的祖先）
    pub alive_only: bool,
    /// 最少累计个体数，滤掉"一次性突变"噪声
    pub min_cum: u32,
}

impl Default for Opts {
    fn default() -> Self {
        Opts {
            tree: true,
            alive_only: false,
            min_cum: 2,
        }
    }
}

pub fn build(reg: &SpeciesRegistry, opts: &Opts) -> Tree {
    let n = reg.list.len();
    if n == 0 {
        return Tree {
            rows: Vec::new(),
            hidden: 0,
        };
    }

    // ancestor_id 存的是 `Species.id`，不是 list 下标 —— 剪枝会重排下标，id 才是稳定的
    let mut by_id: HashMap<u32, usize> = HashMap::with_capacity(n);
    for (i, sp) in reg.list.iter().enumerate() {
        by_id.insert(sp.id, i);
    }

    // ── 1. 可见集 ──
    let mut visible = vec![false; n];
    if !opts.alive_only && opts.min_cum <= 1 {
        visible.fill(true);
    } else {
        for (i, sp) in reg.list.iter().enumerate() {
            if opts.alive_only && !sp.alive {
                continue;
            }
            if sp.cumulative_pop < opts.min_cum {
                continue;
            }
            visible[i] = true;
            // 把整条祖先链补回来。只按 min_cum 过滤会留下一堆"断头分支"：
            // 父物种累计太少被滤掉、子物种还在，树就成了一堆悬空枝条，
            // 看起来像"凭空冒出来的物种"——而它们明明是有来历的。
            let mut cur = sp.ancestor_id;
            let mut guard = 0usize;
            while let Some(a) = cur {
                guard += 1;
                if guard > n {
                    break; // 脏数据里成环时兜底
                }
                let Some(&pi) = by_id.get(&a) else { break };
                if visible[pi] {
                    // 每个被标可见的物种都会立刻往上走一遍，所以这里为真
                    // 就意味着它上面的链也已经补全了
                    break;
                }
                visible[pi] = true;
                cur = reg.list[pi].ancestor_id;
            }
        }
    }
    let vis_count = visible.iter().filter(|v| **v).count();
    let hidden = n - vis_count;

    // ── 2. 平铺视图：老行为，活着的在前、按当前个体数降序 ──
    if !opts.tree {
        let mut idxs: Vec<usize> = (0..n).filter(|&i| visible[i]).collect();
        idxs.sort_by(|&a, &b| {
            let (sa, sb) = (&reg.list[a], &reg.list[b]);
            (sb.population > 0)
                .cmp(&(sa.population > 0))
                .then(sb.population.cmp(&sa.population))
                .then(sa.start_tick.cmp(&sb.start_tick))
        });
        return Tree {
            rows: idxs
                .into_iter()
                .map(|idx| Row {
                    idx,
                    depth: 0,
                    parent: None,
                })
                .collect(),
            hidden,
        };
    }

    // ── 3. 谱系树：建父子索引 ──
    let mut children: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut roots: Vec<usize> = Vec::new();
    for i in 0..n {
        if !visible[i] {
            continue;
        }
        let parent = reg.list[i]
            .ancestor_id
            .and_then(|a| by_id.get(&a).copied())
            // p != i：脏数据里自己当自己的祖先时不要自己接自己
            .filter(|&p| p != i && visible[p]);
        match parent {
            Some(p) => children.entry(p).or_default().push(i),
            None => roots.push(i),
        }
    }
    // 兄弟按出现时间排，从上到下就是"先出现的在上"
    for v in children.values_mut() {
        v.sort_by_key(|&i| reg.list[i].start_tick);
    }
    roots.sort_by_key(|&i| reg.list[i].start_tick);

    // ── 4. 深度优先遍历 ──
    //
    // 用显式栈而不是递归：物种判据目前粒度过细（物种数 ≈ 种群数），
    // 祖先链可能上千层，递归会爆栈。
    let mut rows: Vec<Row> = Vec::with_capacity(vis_count);
    let mut done = vec![false; n];
    let mut stack: Vec<(usize, u16, Option<usize>)> = Vec::new();
    for &root in &roots {
        stack.push((root, 0, None));
        while let Some((idx, depth, parent)) = stack.pop() {
            if done[idx] {
                continue; // 环兜底
            }
            done[idx] = true;
            let my_row = rows.len();
            rows.push(Row { idx, depth, parent });
            if let Some(ch) = children.get(&idx) {
                // 逆序压栈，弹出时才是"先出现的先展开"
                for &c in ch.iter().rev() {
                    stack.push((c, depth.saturating_add(1), Some(my_row)));
                }
            }
        }
    }

    // 兜底：祖先链成环（脏数据）时那些物种不会被任何根到达，但它们是"可见"的。
    // 补成独立的根，保证「显示 N 个」和实际行数一致 —— 否则用户会以为丢数据。
    for i in 0..n {
        if visible[i] && !done[i] {
            done[i] = true;
            rows.push(Row {
                idx: i,
                depth: 0,
                parent: None,
            });
        }
    }

    Tree { rows, hidden }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evosim_core::anatomy::Anatomy;
    use evosim_core::cell_type::ct;
    use std::sync::Arc;

    fn registry() -> SpeciesRegistry {
        SpeciesRegistry::default()
    }

    /// 造一个物种并返回它的指纹；`parent` 用来接祖先
    ///
    /// 注意 `observe` 会把 `name` 覆盖成 `S{id}`，所以测试里定位物种**只能按指纹**。
    fn add(
        reg: &mut SpeciesRegistry,
        key: &str,
        parent: Option<&str>,
        start: u64,
        cum: u32,
        alive: bool,
    ) -> Arc<str> {
        let mut anatomy = Anatomy::default();
        anatomy.add_cell(ct::MOUTH, 0, 0, 0);
        anatomy.recompute();
        let fp = Arc::<str>::from(key);
        let pf = parent.map(Arc::<str>::from);
        reg.observe(&fp, pf.as_ref(), &anatomy, start);
        let i = reg.map.get(key).copied().unwrap();
        reg.list[i].cumulative_pop = cum;
        reg.list[i].alive = alive;
        if !alive {
            reg.list[i].end_tick = start as i64 + 10;
        }
        fp
    }

    /// 某个指纹在树里排第几行
    fn row_of(t: &Tree, reg: &SpeciesRegistry, key: &str) -> usize {
        t.rows
            .iter()
            .position(|r| reg.list[r.idx].fingerprint.as_ref() == key)
            .unwrap_or_else(|| panic!("{} 不在树上", key))
    }

    /// 树的结构性不变量：父行一定在子行**之前**，且子行紧跟在自己那一支里。
    ///
    /// 这条是"连线不交叉"的充要条件：只要父在子之前、且同一支连续，
    /// 竖线就只会跨过自己这一支的行。
    #[test]
    fn parent_row_comes_before_child() {
        let mut reg = registry();
        add(&mut reg, "root", None, 0, 50, false);
        add(&mut reg, "a", Some("root"), 10, 30, false);
        add(&mut reg, "b", Some("root"), 20, 20, false);
        add(&mut reg, "a1", Some("a"), 30, 10, false);

        let t = build(&reg, &Opts::default());
        assert_eq!(t.rows.len(), 4);
        for (i, r) in t.rows.iter().enumerate() {
            if let Some(p) = r.parent {
                assert!(p < i, "第 {} 行的父行是 {}，父必须在子之前", i, p);
                assert_eq!(
                    t.rows[p].depth + 1,
                    r.depth,
                    "第 {} 行的代数应该是父行 +1",
                    i
                );
            }
        }
        // a 的子树（a, a1）必须连续：a1 紧跟在 a 后面
        assert_eq!(
            row_of(&t, &reg, "a1"),
            row_of(&t, &reg, "a") + 1,
            "同一支的后代要紧挨在父物种下面"
        );
        // 兄弟按出现时间排：a（tick 10）在 b（tick 20）之前
        assert!(row_of(&t, &reg, "a") < row_of(&t, &reg, "b"));
    }

    /// 过滤掉低累计的父物种时，子物种不能变成"凭空出现"的孤儿：
    /// 祖先链必须整条补回来，且父子关系保持。
    #[test]
    fn filtered_parent_is_pulled_back_in() {
        let mut reg = registry();
        add(&mut reg, "root", None, 0, 100, false);
        add(&mut reg, "thin", Some("root"), 10, 1, false); // 会被 min_cum 滤掉
        add(&mut reg, "thick", Some("thin"), 20, 40, true); // 但它有后代

        let t = build(
            &reg,
            &Opts {
                tree: true,
                alive_only: false,
                min_cum: 5,
            },
        );
        assert_eq!(t.rows.len(), 3, "祖先链要被补全：root / thin / thick");
        assert_eq!(t.hidden, 0);

        let thin = t
            .rows
            .iter()
            .find(|r| reg.list[r.idx].fingerprint.as_ref() == "thin")
            .expect("累计只有 1 的中间物种也必须出现在树上");
        assert!(thin.parent.is_some(), "补回来的祖先仍然要接到它自己的父物种");
    }

    /// 过滤只影响显示，不能凭空丢物种：行数 + 隐藏数 == 总数
    #[test]
    fn row_count_matches_registry() {
        let mut reg = registry();
        add(&mut reg, "root", None, 0, 100, false);
        add(&mut reg, "x", Some("root"), 10, 1, false);
        add(&mut reg, "y", Some("root"), 20, 40, true);

        for min_cum in [1u32, 2, 5, 999] {
            for alive_only in [false, true] {
                let t = build(
                    &reg,
                    &Opts {
                        tree: true,
                        alive_only,
                        min_cum,
                    },
                );
                assert_eq!(
                    t.rows.len() + t.hidden,
                    reg.size(),
                    "min_cum={} alive_only={} 时行数 + 隐藏数 != 物种总数",
                    min_cum,
                    alive_only
                );
            }
        }
    }

    /// 祖先链成环（存档被改坏 / 早期 bug 留下的脏数据）时不能死循环
    #[test]
    fn ancestor_cycle_terminates() {
        let mut reg = registry();
        add(&mut reg, "p", None, 0, 10, false);
        add(&mut reg, "q", Some("p"), 10, 10, false);
        // 手动制造环：p 的祖先是 q
        let pi = reg.map["p"];
        let qi = reg.map["q"];
        let qid = reg.list[qi].id;
        reg.list[pi].ancestor_id = Some(qid);

        let t = build(&reg, &Opts::default());
        assert_eq!(t.rows.len(), 2, "环里的物种也要出现在图上，不能被静默丢掉");
    }

    /// 平铺视图：活着的在前、按当前个体数降序（老行为，不能退化）
    #[test]
    fn flat_view_sorts_by_population() {
        let mut reg = registry();
        add(&mut reg, "dead", None, 0, 10, false);
        add(&mut reg, "big", Some("dead"), 10, 10, true);
        add(&mut reg, "small", Some("dead"), 20, 10, true);
        let bi = reg.map["big"];
        let si = reg.map["small"];
        reg.list[bi].population = 90;
        reg.list[si].population = 5;

        let t = build(
            &reg,
            &Opts {
                tree: false,
                alive_only: false,
                min_cum: 1,
            },
        );
        assert_eq!(
            reg.list[t.rows[0].idx].fingerprint.as_ref(),
            "big",
            "种群最多的排最前"
        );
        assert_eq!(
            reg.list[t.rows[2].idx].fingerprint.as_ref(),
            "dead",
            "灭绝的排最后"
        );
        assert!(t.rows.iter().all(|r| r.parent.is_none() && r.depth == 0));
    }
}
