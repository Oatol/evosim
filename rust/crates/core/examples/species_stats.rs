//! 诊断：物种判定粒度到底有多细。
//!
//! 跑法：`cargo run --release -p evosim-core --example species_stats -- 20000`
//!
//! 输出的是决定"要不要改指纹、怎么改"的那几个数：
//! 存活个体数 vs 存活物种数、物种的个体数分布、祖先链深度、典型描述。

use std::collections::HashMap;

use evosim_core::config::Config;
use evosim_core::factory::{seed_origin_of_life, spawn_population};
use evosim_core::rng::Rng;
use evosim_core::world::World;
use evosim_core::worldgen::generate_demo_world;

fn main() {
    let ticks: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(20_000);
    let seed: u32 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1001);

    let mut w = World::new(Config::default(), Rng::new(seed));
    generate_demo_world(&mut w, 1.0, 1.0);
    seed_origin_of_life(&mut w);
    spawn_population(&mut w, 8, 3);

    // 先打一条时间序列：单点快照噪声太大（种群本来就在震荡）
    println!("   tick    个体  存活物种  累计物种  一次性占比");
    let step = (ticks / 10).max(1);
    let mut done = 0;
    while done < ticks {
        let n = step.min(ticks - done);
        for _ in 0..n {
            w.step();
        }
        done += n;
        let singles = w
            .species
            .list
            .iter()
            .filter(|s| s.cumulative_pop <= 1)
            .count();
        println!(
            "  {:>6}  {:>5}  {:>8}  {:>8}  {:>7.1}%",
            w.tick,
            w.organisms.len(),
            w.species.alive_count,
            w.species.size(),
            100.0 * singles as f64 / w.species.size().max(1) as f64
        );
    }

    let reg = &w.species;
    println!("── tick {}（seed {}）──", w.tick, seed);
    println!(
        "个体 {} · 物种累计 {} · 物种存活 {} · 峰值存活 {}",
        w.organisms.len(),
        reg.size(),
        reg.alive_count,
        reg.peak_alive_count
    );
    if w.organisms.is_empty() || reg.size() == 0 {
        println!("世界已经灭绝，换个大点的 tick 数或别的种子");
        return;
    }
    println!(
        "  每个存活物种平均摊到 {:.1} 个个体（1.0 = 一个体一物种，完全退化）",
        w.organisms.len() as f64 / reg.alive_count.max(1) as f64
    );

    // ── 存活物种的个体数分布 ──
    let mut buckets = [0usize; 5];
    for sp in reg.list.iter().filter(|s| s.population > 0) {
        let b = match sp.population {
            1 => 0,
            2..=3 => 1,
            4..=10 => 2,
            11..=100 => 3,
            _ => 4,
        };
        buckets[b] += 1;
    }
    println!(
        "  存活物种的个体数分布：1只 {} · 2-3只 {} · 4-10只 {} · 11-100只 {} · >100只 {}",
        buckets[0], buckets[1], buckets[2], buckets[3], buckets[4]
    );

    // ── 累计个体数（= 这个物种一共出现过几个个体）──
    let singles = reg.list.iter().filter(|s| s.cumulative_pop <= 1).count();
    println!(
        "  一次性物种（累计 ≤1 个个体）{} / {} = {:.1}%",
        singles,
        reg.size(),
        100.0 * singles as f64 / reg.size() as f64
    );

    // ── 祖先链深度 ──
    let by_id: HashMap<u32, usize> = reg
        .list
        .iter()
        .enumerate()
        .map(|(i, s)| (s.id, i))
        .collect();
    let depth_of = |start: usize| -> usize {
        let mut d = 0;
        let mut cur = reg.list[start].ancestor_id;
        let mut guard = 0;
        while let Some(a) = cur {
            guard += 1;
            if guard > reg.size() {
                break;
            }
            let Some(&i) = by_id.get(&a) else { break };
            d += 1;
            cur = reg.list[i].ancestor_id;
        }
        d
    };
    let mut depths: Vec<usize> = reg
        .list
        .iter()
        .enumerate()
        .filter(|(_, s)| s.population > 0)
        .map(|(i, _)| depth_of(i))
        .collect();
    depths.sort_unstable();
    if !depths.is_empty() {
        println!(
            "  存活物种的祖先链深度：中位 {} · 最大 {}",
            depths[depths.len() / 2],
            depths[depths.len() - 1]
        );
    }

    // ── 结构描述：看"不同物种"到底差在哪 ──
    let mut descs: Vec<(u32, &str)> = reg
        .list
        .iter()
        .filter(|s| s.population > 0)
        .map(|s| (s.population, s.description.as_str()))
        .collect();
    descs.sort_by(|a, b| b.0.cmp(&a.0));
    println!("  个体数最多的几个存活物种：");
    for (pop, desc) in descs.iter().take(8) {
        println!("    {:>4} 只  {}", pop, desc);
    }

    // ── 同一个"描述"下聚了多少个指纹 ──
    //
    // 这是最关键的一个数：如果同一种细胞组成（比如"嘴×1 生产者×2"）被拆成了
    // 几十上百个物种，说明差异只在**布局**上，粗化指纹就能立刻收敛。
    let mut by_desc: HashMap<&str, usize> = HashMap::new();
    for sp in reg.list.iter() {
        *by_desc.entry(sp.description.as_str()).or_default() += 1;
    }
    let mut desc_groups: Vec<(&str, usize)> = by_desc.into_iter().collect();
    desc_groups.sort_by(|a, b| b.1.cmp(&a.1));
    println!(
        "  按「细胞组成」聚合：{} 个物种只对应 {} 种不同组成",
        reg.size(),
        desc_groups.len()
    );
    for (desc, n) in desc_groups.iter().take(5) {
        println!("    {:>4} 个物种共享  {}", n, desc);
    }

    // ── 存活层面：当前判据 vs 只按组成 ──
    //
    // 这是"要不要改判据"的关键对比：如果存活层面两者差不多，说明粗化只是
    // 砍掉了已灭绝的中间态，对"现在有哪些物种"几乎没影响 —— 那就不值得改。
    let alive_descs: std::collections::HashSet<&str> = reg
        .list
        .iter()
        .filter(|s| s.population > 0)
        .map(|s| s.description.as_str())
        .collect();
    println!(
        "  存活层面：{} 个物种（布局敏感）vs {} 种组成（布局不敏感）",
        reg.alive_count,
        alive_descs.len()
    );

    // ── 出生率：估算"每次出生做一次聚类"的成本 ──
    //
    // `total_created` 只数"新开了几个物种"，不是出生次数。出生次数看现存物种的
    // `cumulative_pop` 之和（被剪掉的物种已丢失，所以是下界）。
    let observes: u64 = reg.list.iter().map(|s| s.cumulative_pop as u64).sum();
    println!(
        "  新物种 {} 个（{:.4}/tick）· 登记个体数 ≥{}（≥{:.2}/tick）",
        reg.total_created,
        reg.total_created as f64 / w.tick.max(1) as f64,
        observes,
        observes as f64 / w.tick.max(1) as f64
    );

    // ── 噪声尾巴：累计个体数分布 ──
    //
    // `minDiscard` 只在上限被顶到时才剪。没顶到时这些"昙花一现"会一直留着，
    // 把谱系树撑得很大 —— 这决定了"该不该靠剪枝/过滤解决，而不是改判据"。
    let mut cum_buckets = [0usize; 5];
    for sp in reg.list.iter() {
        let b = match sp.cumulative_pop {
            0..=1 => 0,
            2..=4 => 1,
            5..=9 => 2,
            10..=99 => 3,
            _ => 4,
        };
        cum_buckets[b] += 1;
    }
    println!(
        "  累计个体数分布：≤1 {} · 2-4 {} · 5-9 {} · 10-99 {} · ≥100 {}（后两档才算真有存续）",
        cum_buckets[0], cum_buckets[1], cum_buckets[2], cum_buckets[3], cum_buckets[4]
    );
}
