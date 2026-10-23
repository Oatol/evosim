//! 发育探针（批次 45 调试用，只读）。
//!
//! 打印「蓝图多大 / 长出多少 / 还剩几格没长」的分布，用来定位
//! "体型塌到 2 格" 这类问题到底是**展开时被截断**还是**生长时被凋亡**。

use evosim_core::config::Config;
use evosim_core::factory::{seed_origin_of_life, spawn_population};
use evosim_core::rng::Rng;
use evosim_core::world::World;
use evosim_core::worldgen::generate_demo_world;

fn main() {
    let ticks: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1200);
    let mut cfg = Config::default();
    cfg.world.cols = 120;
    cfg.world.rows = 80;
    // `MUT=N`：把变异率钉死，用来判断"基因组不长大"是不是变异门控的问题
    // `ADD=100 REMOVE=0`：只加不减，用来确认"基因组能不能长大"这条路是通的
    if let Ok(v) = std::env::var("ADD") {
        if let Ok(r) = v.parse::<f64>() { cfg.mutation.add_prob = r; }
    }
    if let Ok(v) = std::env::var("REMOVE") {
        if let Ok(r) = v.parse::<f64>() { cfg.mutation.remove_prob = r; }
    }
    // `GCOST=N`：每长一格的能量成本（默认 1.0）
    if let Ok(v) = std::env::var("GCOST") {
        if let Ok(r) = v.parse::<f64>() { cfg.life.growth_cost = r; }
    }
    if let Ok(v) = std::env::var("MUT") {
        if let Ok(r) = v.parse::<f64>() {
            cfg.mutation.use_global_mutability = true;
            cfg.mutation.global_mutability = r;
        }
    }
    let mut w = World::new(cfg, Rng::new(1001));
    generate_demo_world(&mut w, 1.0, 1.0);
    seed_origin_of_life(&mut w);
    spawn_population(&mut w, 8, 3);

    for t in 0..=ticks {
        if t % 200 == 0 {
            let n = w.organisms.len().max(1);
            let buds = w.organisms.iter().filter(|o| o.growth_plan.is_some()).count();
            let cells: usize = w.organisms.iter().map(|o| o.cell_count()).sum();
            let mut plans: Vec<usize> = w
                .organisms
                .iter()
                .filter_map(|o| o.growth_plan.as_ref().map(|p| p.len()))
                .collect();
            plans.sort_unstable();
            let pending: usize = w
                .organisms
                .iter()
                .filter_map(|o| {
                    o.growth_plan
                        .as_ref()
                        .map(|p| p.pending(&o.anatomy))
                })
                .sum();
            let dropped: usize = w
                .organisms
                .iter()
                .filter_map(|o| {
                    o.growth_plan
                        .as_ref()
                        .map(|p| p.dropped.iter().filter(|d| **d).count())
                })
                .sum();
            let adult_genomes: Vec<usize> = w
                .organisms
                .iter()
                .filter(|o| o.growth_plan.is_none())
                .map(|o| o.effective_genome().expand(0).len())
                .collect();
            let avg_g = if adult_genomes.is_empty() {
                0.0
            } else {
                adult_genomes.iter().sum::<usize>() as f64 / adult_genomes.len() as f64
            };
            println!(
                "t={:<5} 个体 {:>4} · 芽 {:>4} · 平均体型 {:.2} · 成体基因组 {:.2} · 蓝图(中位/最大) {:?}/{:?} · 待长 {} · 已凋亡 {}",
                t,
                w.organisms.len(),
                buds,
                cells as f64 / n as f64,
                avg_g,
                plans.get(plans.len() / 2),
                plans.last(),
                pending,
                dropped
            );
        }
        w.step();
    }
}
