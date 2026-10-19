//! 回归基线 —— 取代原来的黄金快照。
//!
//! ## 为什么废弃黄金快照
//!
//! `tests/golden.rs` 要求 Rust 版与 JS 版**逐位一致**，那是 1.x → 2.0 迁移期的
//! 质量保证：改内核时只要有一个 tick 漂了，测试立刻红。
//!
//! 但 2.0 现在要**改模拟本身**（光场、能量守恒、卵生），逐位对齐只会挡路——
//! 每次有意的行为改动都得去改基线，"测试红了"和"测试过时了"分不清。
//! 用户已确认：黄金快照不再维护。
//!
//! ## 那靠什么防回归
//!
//! 换成**特征化测试**（characterization test）：跑固定 tick 数，断言几个宏观量
//! 落在合理区间。它抓不到"第 37 tick 多算了一点"，但能抓到"改完生态崩了"
//! 或"体型突然变成 1 或 50"这类真正要命的问题。
//!
//! **基线数值变了不一定是 bug**——改了模拟就该变。更新区间时先跑
//! `cargo run --release -p evosim-core --example light_check -- 2000`，
//! 确认变化方向是想要的那个。
//!
//! ## 保留的部分
//!
//! `rng_matches_js_sequence` 留着：RNG 是**确定性基础**，不是模拟行为。
//! 它的序列一旦漂了，所有存档重放和实验复现都会失效，和"要不要对齐 1.x"无关。

use std::fs;
use std::path::PathBuf;

use evosim_core::config::Config;
use evosim_core::factory::{seed_origin_of_life, spawn_population};
use evosim_core::rng::Rng;
use evosim_core::world::World;
use evosim_core::worldgen::generate_demo_world;

const SEED: u32 = 1001;

/// 建一个和 `examples/light_check` 完全一样的初始世界，跑 `ticks` 步。
fn run(ticks: u64, metabolism: f64) -> World {
    run_with_seed(SEED, ticks, metabolism)
}

/// 同上，但可以换种子（用来复核「结论是不是只对某一个种子成立」）。
fn run_with_seed(seed: u32, ticks: u64, metabolism: f64) -> World {
    run_with(seed, ticks, metabolism, Config::default().life.shade_strength)
}

/// 全参数版：`shade = 0` 关掉遮挡，用来单独观察某个机制。
fn run_with(seed: u32, ticks: u64, metabolism: f64, shade: f64) -> World {
    let mut cfg = Config::default();
    cfg.life.metabolism_scale = metabolism;
    cfg.life.shade_strength = shade;
    let mut w = World::new(cfg, Rng::new(seed));
    generate_demo_world(&mut w, 1.0, 1.0);
    seed_origin_of_life(&mut w);
    spawn_population(&mut w, 8, 3);
    for _ in 0..ticks {
        w.step();
    }
    w
}

fn alive(w: &World) -> Vec<&evosim_core::organism::Organism> {
    w.organisms.iter().filter(|o| o.alive).collect()
}

/// RNG 必须和 JS 版逐位一致 —— 这条和"要不要对齐模拟行为"无关。
///
/// 序列一漂，所有存档重放、实验复现、A/B 对照全部失效。
#[test]
fn rng_matches_js_sequence() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../golden/rng-u32.txt");
    let text = fs::read_to_string(&path).expect("找不到 golden/rng-u32.txt");
    let expected: Vec<u32> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(8)
        .map(|l| u32::from_str_radix(l.trim(), 16).expect("rng-u32.txt 里有非十六进制行"))
        .collect();
    assert_eq!(expected.len(), 8, "rng-u32.txt 至少要有 8 个样本");

    let mut rng = Rng::new(12345);
    for (i, e) in expected.iter().enumerate() {
        assert_eq!(rng.next_u32(), *e, "第 {} 个随机数和 JS 版对不上", i);
    }
}

/// 光场必须有垂直梯度：顶部满照、底部全黑。
///
/// 这条是后面所有"趋光行为"的前提。光场一旦变成均匀的（比如有人把
/// `absorb_for` 改成返回 0），生态会悄悄退回"到处一样富饶"的旧状态，
/// 而其他测试都不会红。
#[test]
fn light_field_has_vertical_gradient() {
    let w = run(0, 0.0);
    assert_eq!(w.light.at(&w.grid, 0, 0), 255, "顶行应该满照");
    assert!(w.light.absorb >= 1, "absorb 至少是 1，否则光传不下去");

    // 底部不再保证**精确为 0**：光是多源 BFS 扩散的，会从侧面渗进深处
    // （这正是软阴影，比第一版的硬切更真实）。所以只断言"深处明显更暗"。
    let mid = w.grid.rows as i32 / 2;
    let surface = w.light.at(&w.grid, 0, 0) as i32;
    let deep = w.light.at(&w.grid, 0, mid) as i32;
    assert!(
        deep < surface / 2,
        "世界中线（{}）应该比地表（{}）暗一半以上",
        deep,
        surface
    );
}

/// 2000 tick 后生态还活着，体型落在合理区间。
#[test]
fn ecosystem_survives_2000_ticks() {
    let w = run(2000, 0.0);
    let orgs = alive(&w);
    assert!(
        orgs.len() > 100,
        "2000 tick 后只剩 {} 个个体——生态崩了",
        orgs.len()
    );
    assert!(orgs.len() < 5000, "个体数 {} 异常膨胀", orgs.len());

    let cells: usize = orgs.iter().map(|o| o.cell_count()).sum();
    let avg = cells as f64 / orgs.len() as f64;
    assert!(
        (2.0..=12.0).contains(&avg),
        "平均体型 {:.2} 超出合理区间 [2, 12]",
        avg
    );
}

/// 种群应该待在有光的上半部分 —— 光合是唯一能量来源。
///
/// 这条是「光场真的影响了行为」最直接的证据。如果种群均匀分布在世界里，
/// 说明光合没有按位置结算（或者光场是均匀的）。
#[test]
fn population_prefers_the_lit_half() {
    let w = run(2000, 0.0);
    let orgs = alive(&w);
    let avg_row: f64 = orgs.iter().map(|o| o.row as f64).sum::<f64>() / orgs.len() as f64;
    let mid = w.grid.rows as f64 * 0.6;
    assert!(
        avg_row < mid,
        "种群平均行 {:.1} 太靠下（世界中线 {:.1}）——光合没把生物拉向顶部",
        avg_row,
        mid
    );
}

/// 维持成本必须真的把体型压下去（**关掉遮挡**之后）。
///
/// 这条钉的是「多一格要不要付账」：`metabolism_scale = 0` 时多一格是免费的，
/// 所以体型会漂上去（多一个生产者纯赚）。打开成本之后体型必须变小，
/// 否则说明 upkeep 根本没被扣——那能量守恒就只是个说法。
///
/// **为什么要关掉遮挡**：批次 9 加了遮挡之后，「大体型」第一次有了收益
/// （长得高才抢得到光），于是成本上涨不一定压小体型。实测（3000 tick、metab 0.05）：
///
/// | 种子 | 体型 无成本→有成本 | 细胞总数 无成本→有成本 |
/// |---|---|---|
/// | 1001 | 4.21 → **5.03**（反而更大） | 3471 → 3237 |
/// | 1002 | 3.24 → 3.02 | 3318 → 3301 |
/// | 1003 | 2.22 → **2.74**（反而更大） | 3574 → 3124 |
///
/// 所以这条测试**只验"扣费有宏观效果"这件事本身**，方法是把遮挡关掉、
/// 让体型回到"纯成本"的语境里。两个机制纠缠时，任何单一趋势断言都会变成噪声。
/// 精确的每 tick 扣费由 `world::tests::metabolism_drains_energy_every_tick` 守。
///
/// **只断言方向，不断言幅度。** 幅度对种子的依赖极大（这个生态有多个吸引子），
/// 在单个种子上写死百分比阈值等于赌它落在哪个吸引子里——批次 8 之前
/// 那条 `taxed < free * 0.95` 就是这么挂掉的。
#[test]
fn metabolism_presses_body_size_down() {
    let avg = |seed: u32, metab: f64| -> f64 {
        let w = run_with(seed, 3000, metab, 0.0);
        let orgs = alive(&w);
        let cells: usize = orgs.iter().map(|o| o.cell_count()).sum();
        cells as f64 / orgs.len() as f64
    };
    for seed in [1001, 1003] {
        let free = avg(seed, 0.0);
        let taxed = avg(seed, 0.05);
        assert!(
            taxed < free,
            "种子 {}：打开维持成本后体型反而从 {:.2} 涨到 {:.2}——upkeep 没生效",
            seed,
            free,
            taxed
        );
    }
}
