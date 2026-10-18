//! 体型扫描：繁殖成本指数 p 与体型上限对演化结果的影响。
//!
//! 用法：
//! ```text
//! cargo run --release -p evosim-core --example body_scan -- <ticks> [seeds...]
//! ```
//!
//! 为什么要有这个：`breed_cost_exponent` 的默认值必须是 1.0（黄金快照锁着），
//! 所以"到底该给多少"只能靠扫描得出证据，不能凭感觉改默认值。
//!
//! **必须多种子取平均**：这个模拟处在临界状态，同一个 p 换个种子，种群能从 256
//! 跳到 1157（4.5 倍）。单种子单点的结论不可信——第一次扫就是这么被坑的。
//!
//! p 的**方向**容易搞反：`food_needed = n^p`，p > 1 让大体型更贵（体型变小），
//! p < 1 让大体型更便宜（体型变大）。想看到"大体型"就得往 1 以下扫。

use evosim_core::config::Config;
use evosim_core::factory::{seed_origin_of_life, spawn_population};
use evosim_core::rng::Rng;
use evosim_core::world::World;
use evosim_core::worldgen::generate_demo_world;

/// 一次运行的结果
struct Row {
    pop: f64,
    body_avg: f64,
    body_max: usize,
    species: f64,
    /// 体型分布：1 格 / 2 格 / … 的个体占比（只统计前 10 档，够看形状）
    hist: [f64; 10],
}

fn run(p: f64, cap: usize, ticks: u64, seed: u32) -> Row {
    let mut cfg = Config::default();
    cfg.life.breed_cost_exponent = p;
    cfg.life.max_body_cells = cap;

    let mut w = World::new(cfg, Rng::new(seed));
    generate_demo_world(&mut w, 1.0, 1.0);
    seed_origin_of_life(&mut w);
    spawn_population(&mut w, 8, 3);
    for _ in 0..ticks {
        w.step();
    }

    let st = w.organism_stats();
    let n = w.organisms.len().max(1);

    let mut hist = [0f64; 10];
    for o in &w.organisms {
        let c = o.cell_count();
        if (1..=10).contains(&c) {
            hist[c - 1] += 100.0 / n as f64;
        }
    }

    Row {
        pop: w.organisms.len() as f64,
        body_avg: st.avg_cells,
        body_max: st.max_cells,
        species: w.species.alive_count as f64,
        hist,
    }
}

/// 同参数跑多个种子，取平均（`body_max` 取最大，因为要看"最极端的那只"）
fn run_seeds(p: f64, cap: usize, ticks: u64, seeds: &[u32]) -> Row {
    let mut acc = Row {
        pop: 0.0,
        body_avg: 0.0,
        body_max: 0,
        species: 0.0,
        hist: [0f64; 10],
    };
    for &s in seeds {
        let r = run(p, cap, ticks, s);
        acc.pop += r.pop;
        acc.body_avg += r.body_avg;
        acc.body_max = acc.body_max.max(r.body_max);
        acc.species += r.species;
        for k in 0..10 {
            acc.hist[k] += r.hist[k];
        }
    }
    let n = seeds.len() as f64;
    acc.pop /= n;
    acc.body_avg /= n;
    acc.species /= n;
    for k in 0..10 {
        acc.hist[k] /= n;
    }
    acc
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let ticks: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(15000);
    let seeds: Vec<u32> = if args.len() > 2 {
        args[2..].iter().filter_map(|s| s.parse().ok()).collect()
    } else {
        vec![1001, 777, 4242]
    };
    assert!(!seeds.is_empty(), "至少要一个种子");

    println!("── 体型扫描 · {ticks} tick · 种子 {:?} ──", seeds);
    println!("（同参数多跑几个种子取平均：这模拟在临界点上，单点差 4 倍很常见）\n");

    // ── 第一轮：只动指数 p，不限体型 ──
    println!("【1】繁殖成本指数 p（max_body_cells = 0，不限）");
    println!(
        "{:>5}  {:>7}  {:>9}  {:>9}  {:>7}",
        "p", "个体", "平均体型", "最大体型", "存活种"
    );
    // 往 1 以下扫是重点：p < 1 让大体型便宜，才有向上的选择压力。
    // 只留 4 个点是为了控功耗——这台机器长时间满载会过热（clock_watchdog_timeout），
    // 要更细的曲线就手动加回来，然后盯着跑。
    let ps = [0.6, 0.8, 1.0, 1.2];
    let mut rows = Vec::new();
    for &p in &ps {
        let r = run_seeds(p, 0, ticks, &seeds);
        println!(
            "{:>5.2}  {:>7.0}  {:>9.2}  {:>9}  {:>7.1}",
            p, r.pop, r.body_avg, r.body_max, r.species
        );
        rows.push((p, r));
    }

    // 体型分布：看 p 是不是真的把分布推走
    println!("\n【2】体型分布（占个体百分比）");
    print!("{:>5} ", "p");
    for k in 1..=10 {
        print!(" {:>4}格", k);
    }
    println!();
    for (p, r) in &rows {
        print!("{:>5.2} ", p);
        for v in r.hist.iter() {
            print!(" {:>5.1}", v);
        }
        println!();
    }

    // ── 第二轮：扫体型上限 ──
    //
    // 上限的意义取决于"有没有巨无霸"。p = 1.0 时最大才 5 格，上限给多少都一样；
    // 只有在体型真的能膨胀起来时，上限才是个真问题。
    println!("\n【3】体型上限（p = 1.0）");
    println!(
        "  {:>6}  {:>7}  {:>9}  {:>9}  {:>7}",
        "上限", "个体", "平均体型", "最大体型", "存活种"
    );
    for cap in [0usize, 8, 16] {
        let r = run_seeds(1.0, cap, ticks, &seeds);
        println!(
            "  {:>6}  {:>7.0}  {:>9.2}  {:>9}  {:>7.1}",
            if cap == 0 {
                "不限".to_string()
            } else {
                cap.to_string()
            },
            r.pop,
            r.body_avg,
            r.body_max,
            r.species
        );
    }

    println!("\n判读要点：");
    println!("- 平均体型随 p 下降而上升 → p < 1 才能造出大体型，这就是想要的杠杆");
    println!("- 个体数骤降 → p 给太大，繁殖成本把种群压死了");
    println!("- 最大体型 ≈ 平均体型 → 没有巨无霸，体型上限没必要设");
}
