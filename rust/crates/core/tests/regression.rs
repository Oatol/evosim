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

/// 维持成本必须真的在全生态尺度上"收税"（**关掉遮挡**之后）。
///
/// **判据是热池，不是人口。**
///
/// 这条测试原来断言"打开维持成本后人口变少"。批次 47 之后它开始翻车：
/// 4 个种子 3000 tick 实测（`metabolism_scale` 0 → 1.0）
///
/// | 种子 | 无成本 | 有成本 |
/// |---|---|---|
/// | 1001 | 2196 | 2290 ↑ |
/// | 1002 | 2758 | 2311 ↓ |
/// | 1003 | 2313 | 1985 ↓ |
/// | 777 | 2304 | 2673 ↑ |
///
/// 两个反向、聚合只差 -3.3%，远在这个系统的种子噪声（±20%）里。
/// 原因不是 upkeep 失效了，而是**人口从来就不是成本的单调函数**：
/// 它同时被空间、繁殖成功率、捕食、地力推着走，而这个世界已经接近饱和。
/// 这和注释里早就记下的"体型不能当成本生效的判据"是同一类错误。
///
/// 换成**直接量**：代谢付出去的能量全部记进 `heat_pool`。
/// 打开成本 → 热池必然暴涨，这是单调的、与生态吸引子无关的。
/// "某个细胞到底扣了多少钱"由 `upkeep_is_paid_into_the_heat_pool` 单测覆盖；
/// 这里只管"整局游戏里税真的收了"。
#[test]
fn upkeep_drains_energy_into_the_heat_pool() {
    let run = |seed: u32, metab: f64| -> (f64, usize) {
        let w = run_with(seed, 3000, metab, 0.0);
        (w.heat_pool, alive(&w).len())
    };
    for seed in [1001, 1003] {
        let (free_heat, free_pop) = run(seed, 0.0);
        let (taxed_heat, taxed_pop) = run(seed, 1.0);
        assert!(
            taxed_heat > free_heat * 1.5 + 100.0,
            "种子 {}：打开维持成本后热池只从 {:.1} 涨到 {:.1}——upkeep 没在收税（人口 {} → {}）",
            seed,
            free_heat,
            taxed_heat,
            free_pop,
            taxed_pop
        );
    }
}

/// 能量账必须平：**注入总量 == 生物 + 食物 + 热池 + 丢弃**。
///
/// 这是批次 10 守恒审计的核心断言。它抓的不是"某个数变了"，而是
/// **有某条路径在凭空造能量或吞能量**——这种 bug 在数值上完全看不出来
/// （种群照样长、体型照样正常），只有对账才会暴露。
///
/// 这条等式成立的前提是每一个"改能量"的地方都记了账：
///
/// | 路径 | 记到哪 |
/// |---|---|
/// | 光合产出 | `energy_injected`（全额，含装不下的部分） |
/// | 腐殖质冒食物 / 手工放置 | `energy_injected`（`set_cell_at` 里统一记） |
/// | 维持成本 / 沼泽耗能 / 繁殖成本 | `heat_pool` |
/// | 食物腐败 / 压碎 / 塞不下的溢出 | `heat_pool` |
/// | 超容量丢弃 / 消化损耗 / 摄取倍率 < 1 | `energy_wasted` |
/// | 摄取倍率 > 1 | `energy_injected`（它是凭空放大，得认） |
///
/// 漏掉任何一条，残差就会随时间线性增长。
#[test]
fn energy_ledger_balances() {
    // 代谢开 / 关两种都要平：关掉代谢时 upkeep 那条路径根本不执行，
    // 只测一种会漏掉另一半。
    for (seed, metab) in [(1001, 1.0), (1001, 0.0), (1003, 1.0)] {
        let w = run_with(seed, 2000, metab, Config::default().life.shade_strength);
        let a = w.energy_audit();
        let tol = 1e-9 * a.stored().max(1.0);
        assert!(
            a.drift().abs() < tol,
            "种子 {} / 代谢 {}：能量账对不上。注入 {:.9}，存量 {:.9}             （生物 {:.3} + 食物 {:.3} + 热池 {:.3} + 丢弃 {:.3}），残差 {:.3e}",
            seed,
            metab,
            a.injected,
            a.stored(),
            a.organisms,
            a.food,
            a.heat,
            a.wasted,
            a.drift()
        );
    }
}

/// 云影（批次 10.5）：开着也要活得下去，而且**能量账照样要平**。
///
/// 云影只改 `sky_level` 的数值，不动任何能量流向，所以守恒等式必须仍然成立——
/// 这条顺带守住"以后给光场加任何滤镜，都不能悄悄改能量"。
#[test]
fn ecosystem_survives_clouds_and_the_ledger_still_balances() {
    let mut cfg = Config::default();
    cfg.world.sky_patchiness = 1.0;
    let mut w = World::new(cfg, Rng::new(1002));
    generate_demo_world(&mut w, 1.0, 1.0);
    seed_origin_of_life(&mut w);
    spawn_population(&mut w, 8, 3);

    // 始祖必须落在**真的有光**的地方：批次 10.5 的第一次实测就是栽在这上面，
    // 固定坐标正好压在暗斑里，1002 全灭。
    let origin = &w.organisms[0];
    let lit = w.light.sky_at(&w.grid, origin.col, origin.row);
    assert!(
        lit > 100,
        "始祖所在格光强只有 {}，应该落在光岛里",
        lit
    );

    for _ in 0..2000 {
        w.step();
    }
    let n = alive(&w).len();
    assert!(n > 50, "云影 1.0 下 2000 tick 只剩 {} 个个体——生态崩了", n);

    let a = w.energy_audit();
    assert!(
        a.drift().abs() < 1e-9 * a.stored().max(1.0),
        "云影下能量账对不上：注入 {:.6}，存量 {:.6}，残差 {:.3e}",
        a.injected,
        a.stored(),
        a.drift()
    );
}

/// 同样的**平均**光照下，碎片化的光比整体变暗能养活更多生命。
///
/// 这是云影存在的正面理由：`cloud = 1` 把平均透射率压到 0.5，
/// 但保住了"总有一块地方是满照"；`sun = 0.5` 则把每一处都压暗一半。
/// 实测（10000 tick）前者 743 只、后者 184 只，**差 4 倍**。
#[test]
fn fragmented_light_feeds_more_than_uniform_dimness() {
    let run = |cloud: f64, sun: f64| -> usize {
        let mut cfg = Config::default();
        cfg.world.sky_patchiness = cloud;
        cfg.world.sun_intensity = sun;
        let mut w = World::new(cfg, Rng::new(1001));
        generate_demo_world(&mut w, 1.0, 1.0);
        seed_origin_of_life(&mut w);
        spawn_population(&mut w, 8, 3);
        for _ in 0..3000 {
            w.step();
        }
        alive(&w).len()
    };
    let patchy = run(1.0, 1.0);
    let dim = run(0.0, 0.5);
    assert!(
        patchy > dim,
        "碎片化的光（{} 只）应该比同样平均亮度的均匀暗（{} 只）养活更多",
        patchy,
        dim
    );
}

/// 富贫斑块（批次 13）：开着也要活得下去，而且**能量账照样要平**。
///
/// 它只改天空光的数值和食物落点，不动任何能量流向，所以守恒等式必须仍然成立。
#[test]
fn ecosystem_survives_rich_patches_and_the_ledger_still_balances() {
    let mut cfg = Config::default();
    cfg.world.patch_richness = 0.7;
    let mut w = World::new(cfg, Rng::new(1001));
    generate_demo_world(&mut w, 1.0, 1.0);
    seed_origin_of_life(&mut w);
    spawn_population(&mut w, 8, 3);

    // 世界必须**同时**有富区和贫区，否则这一批就白做了。
    //
    // 只看**顶行有光**的列：顶行是墙的那一列本来就不发光（光不该从墙里冒出来），
    // 那是另一回事，混进来会把"最暗"算成 0。
    let top: Vec<u8> = (0..w.grid.cols)
        .map(|c| w.light.sky_at(&w.grid, c as i32, 0))
        .filter(|&v| v > 0)
        .collect();
    let hi = *top.iter().max().unwrap();
    let lo = *top.iter().min().unwrap();
    assert!(hi > 200, "应该存在接近满照的富斑块，实测最亮 {}", hi);
    assert!(
        lo > 40 && lo < 140,
        "贫斑块应该「有光但很穷」（40~140），实测 {}",
        lo
    );

    for _ in 0..2000 {
        w.step();
    }
    let n = alive(&w).len();
    assert!(n > 50, "富贫斑块下 2000 tick 只剩 {} 个个体——生态崩了", n);

    let a = w.energy_audit();
    assert!(
        a.drift().abs() < 1e-9 * a.stored().max(1.0),
        "富贫斑块下能量账对不上：注入 {:.6}，存量 {:.6}，残差 {:.3e}",
        a.injected,
        a.stored(),
        a.drift()
    );
}

/// 出芽（批次 15）：开着也要活得下去，而且**能量账照样要平**。
///
/// 这条尤其重要：出芽的「母体供养」是**内部转移**（母体的能量挪给芽），
/// 不进热池也不进注入。一旦写错（比如两边都加），账上会凭空多出能量，
/// 而这种 bug 在种群曲线上完全看不出来。
#[test]
fn ecosystem_survives_budding_and_the_ledger_still_balances() {
    let mut cfg = Config::default();
    cfg.life.growth_enabled = true;
    cfg.life.budding_enabled = true;
    let mut w = World::new(cfg, Rng::new(1001));
    generate_demo_world(&mut w, 1.0, 1.0);
    seed_origin_of_life(&mut w);
    spawn_population(&mut w, 8, 3);

    for _ in 0..2000 {
        w.step();
    }
    let n = alive(&w).len();
    assert!(n > 50, "出芽下 2000 tick 只剩 {} 个个体——生态崩了", n);

    let a = w.energy_audit();
    assert!(
        a.drift().abs() < 1e-9 * a.stored().max(1.0),
        "出芽下能量账对不上：注入 {:.6}，存量 {:.6}，残差 {:.3e}",
        a.injected,
        a.stored(),
        a.drift()
    );
}
