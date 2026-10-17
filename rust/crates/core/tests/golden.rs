//! 黄金快照回归 —— Rust 版必须与 JS 版**逐位一致**。
//!
//! `golden/world.txt` 是 JS 版导出的基线（`node tools/export-golden.mjs`），
//! 每个种子在 1/10/50/100/200/500 tick 上记录一份状态摘要。
//! 任何一次改动让摘要变了，就说明行为漂移了 —— 这个测试会立刻抓住。

use std::fs;
use std::path::PathBuf;

use evosim_core::config::Config;
use evosim_core::factory::{seed_origin_of_life, spawn_population};
use evosim_core::rng::Rng;
use evosim_core::world::World;
use evosim_core::worldgen::generate_demo_world;

fn golden_path() -> PathBuf {
    // CARGO_MANIFEST_DIR = rust/crates/core
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../golden/world.txt")
        .canonicalize()
        .expect("找不到 golden/world.txt（先跑 node tools/export-golden.mjs）")
}

struct Expect {
    seed: u32,
    tick: u64,
    orgs: usize,
    food: usize,
    species: usize,
    body: usize,
    digest: String,
}

fn load_expectations() -> Vec<Expect> {
    let text = fs::read_to_string(golden_path()).expect("读取 golden/world.txt 失败");
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // seed 1001 tick 1 orgs 9 food 900 species 9 body 37 digest 8ff7dc10
        let mut it = line.split_whitespace();
        let _ = it.next();
        let seed: u32 = it.next().unwrap().parse().unwrap();
        let _ = it.next();
        let tick: u64 = it.next().unwrap().parse().unwrap();
        let _ = it.next();
        let orgs: usize = it.next().unwrap().parse().unwrap();
        let _ = it.next();
        let food: usize = it.next().unwrap().parse().unwrap();
        let _ = it.next();
        let species: usize = it.next().unwrap().parse().unwrap();
        let _ = it.next();
        let body: usize = it.next().unwrap().parse().unwrap();
        let _ = it.next();
        let digest = it.next().unwrap().to_string();
        out.push(Expect {
            seed,
            tick,
            orgs,
            food,
            species,
            body,
            digest,
        });
    }
    out
}

#[test]
fn matches_js_golden_snapshots() {
    let expects = load_expectations();
    assert!(!expects.is_empty(), "golden/world.txt 是空的");

    // 按种子分组，每个种子只跑一遍
    let mut seeds: Vec<u32> = expects.iter().map(|e| e.seed).collect();
    seeds.sort_unstable();
    seeds.dedup();

    for seed in seeds {
        let mut world = World::new(Config::default(), Rng::new(seed));
        generate_demo_world(&mut world, 1.0, 1.0);
        seed_origin_of_life(&mut world);
        spawn_population(&mut world, 8, 3);

        let mut items: Vec<&Expect> = expects.iter().filter(|e| e.seed == seed).collect();
        items.sort_by_key(|e| e.tick);

        for item in items {
            while world.tick < item.tick {
                world.step();
            }
            let stats = world.snapshot_stats();
            assert_eq!(stats.organisms, item.orgs, "seed {} tick {} 个体数", seed, item.tick);
            assert_eq!(stats.food, item.food, "seed {} tick {} 食物数", seed, item.tick);
            assert_eq!(
                world.species.size(),
                item.species,
                "seed {} tick {} 物种数",
                seed,
                item.tick
            );
            assert_eq!(stats.body, item.body, "seed {} tick {} 身体格数", seed, item.tick);
            assert_eq!(
                world.state_digest(),
                item.digest,
                "seed {} tick {} 状态摘要（行为已漂移）",
                seed,
                item.tick
            );
        }
    }
}

#[test]
fn rng_matches_js_sequence() {
    // 与 golden/rng-u32.txt 前 8 个值一致（完整 200 个由 tools/parity-check.mjs 覆盖）
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../golden/rng-u32.txt")
        .canonicalize()
        .expect("找不到 golden/rng-u32.txt");
    let text = fs::read_to_string(path).unwrap();
    let expected: Vec<&str> = text.lines().take(8).collect();

    let mut rng = Rng::new(12345);
    for (i, exp) in expected.iter().enumerate() {
        let got = format!("{:08x}", rng.next_u32());
        assert_eq!(&got, exp, "第 {} 个 u32 不一致", i + 1);
    }
}
