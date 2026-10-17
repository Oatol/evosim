//! 无界面 CLI —— 第一阶段用来做黄金快照对照。
//!
//! 用法：
//!   evosim --dump-rng 200           打印 200 个 u32（十六进制，每行一个）
//!   evosim --dump-gauss 200         打印 200 个高斯样本（f64 位模式，十六进制）
//!   evosim --sim --seed 1001 --ticks 500
//!   evosim --digest --seed 1001 --ticks 500

use std::time::Instant;

use evosim_core::config::Config;
use evosim_core::factory::{seed_origin_of_life, spawn_population};
use evosim_core::rng::Rng;
use evosim_core::world::World;
use evosim_core::worldgen::generate_demo_world;

fn arg_value(args: &[String], key: &str) -> Option<String> {
    args.iter()
        .position(|a| a == key)
        .and_then(|i| args.get(i + 1).cloned())
}

fn arg_flag(args: &[String], key: &str) -> bool {
    args.iter().any(|a| a == key)
}

fn main() {
    evosim_core::init_threads();
    let args: Vec<String> = std::env::args().skip(1).collect();

    if arg_flag(&args, "--dump-rng") {
        let n: usize = arg_value(&args, "--dump-rng")
            .and_then(|v| v.parse().ok())
            .unwrap_or(200);
        let seed: u32 = arg_value(&args, "--seed")
            .and_then(|v| v.parse().ok())
            .unwrap_or(12345);
        let mut rng = Rng::new(seed);
        for _ in 0..n {
            println!("{:08x}", rng.next_u32());
        }
        return;
    }

    if arg_flag(&args, "--dump-gauss") {
        let n: usize = arg_value(&args, "--dump-gauss")
            .and_then(|v| v.parse().ok())
            .unwrap_or(200);
        let seed: u32 = arg_value(&args, "--seed")
            .and_then(|v| v.parse().ok())
            .unwrap_or(12345);
        let mut rng = Rng::new(seed);
        for _ in 0..n {
            let v = rng.gaussian(0.0, 1.0);
            println!("{:016x}", v.to_bits());
        }
        return;
    }

    let seed: u32 = arg_value(&args, "--seed")
        .and_then(|v| v.parse().ok())
        .unwrap_or(20_261_015);
    let ticks: u64 = arg_value(&args, "--ticks")
        .and_then(|v| v.parse().ok())
        .unwrap_or(500);
    let pop: usize = arg_value(&args, "--pop")
        .and_then(|v| v.parse().ok())
        .unwrap_or(8);
    let every = arg_flag(&args, "--every");
    let dump_orgs: Option<u64> = arg_value(&args, "--dump-orgs").and_then(|v| v.parse().ok());

    run_sim(seed, ticks, pop, every, dump_orgs);
}

fn run_sim(seed: u32, ticks: u64, pop: usize, every: bool, dump_orgs: Option<u64>) {
    let cfg = Config::default();
    let mut world = World::new(cfg, Rng::new(seed));

    generate_demo_world(&mut world, 1.0, 1.0);
    seed_origin_of_life(&mut world);
    spawn_population(&mut world, pop, 3);

    let checkpoints: [u64; 6] = [1, 10, 50, 100, 200, 500];

    if let Some(t) = dump_orgs {
        while world.tick < t {
            world.step();
        }
        dump_organism_list(&world);
        return;
    }

    let t0 = Instant::now();
    let mut next_cp = 0usize;
    for _ in 0..ticks {
        world.step();
        if every {
            print_state(&world);
            continue;
        }
        if next_cp < checkpoints.len() && world.tick == checkpoints[next_cp] {
            print_state(&world);
            next_cp += 1;
        }
    }
    let elapsed = t0.elapsed().as_secs_f64();

    // 最终一行（脚本对照用）
    print_state(&world);
    if ticks > 0 {
        println!(
            "perf {:.3} ms/tick",
            elapsed * 1000.0 / ticks as f64
        );
    }
}

fn print_state(world: &World) {
    let stats = world.snapshot_stats();
    let orgs = world.organism_stats();
    println!(
        "tick {} orgs {} food {} species {} body {} digest {}",
        world.tick,
        stats.organisms,
        stats.food,
        world.species.size(),
        stats.body,
        world.state_digest()
    );
    if world.tick % 500 == 0 && world.tick > 0 {
        println!(
            "  avg_cells {:.2} movers {} eyes {} killers {} avg_mut {:.2}",
            orgs.avg_cells, orgs.movers, orgs.eyes, orgs.killers, orgs.avg_mutability
        );
    }
}

/// 逐个体导出（定位偏差用）
fn dump_organism_list(world: &World) {
    println!("tick {} count {}", world.tick, world.organisms.len());
    for org in &world.organisms {
        let mut cells = String::new();
        for c in &org.anatomy.cells {
            cells.push_str(&format!("{}:{},{}:{},", c.type_id, c.dx, c.dy, c.dir));
        }
        println!(
            "org {} col {} row {} rot {} dir {} age {} energy {:.6} damage {:.6} mut {:.4} cells {}",
            org.id, org.col, org.row, org.rotation, org.direction, org.age,
            org.energy, org.damage, org.mutability, cells
        );
    }
}
