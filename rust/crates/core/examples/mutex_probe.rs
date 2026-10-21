//! **只读诊断**：「生产者 + 嘴」互斥到底会损失多少（批次 30 前置调研）。
//!
//! 背景：纯异养卡在 1~5%，批次 29 的判断是"混养体（生产者 + 嘴）永远更优——
//! 它有底薪又吃红利"。于是有人想：**让一个身体不能同时有生产者和嘴**，
//! 逼它们选边，纯捕食者就多了。
//!
//! 这个想法在动手前必须先量三件事：
//!
//! 1. 混养体到底占多少？
//! 2. 它们的收入是"一边压倒另一边"，还是"真两栖"？
//! 3. **它们的身体构成支持哪条路**？一个 9 生产者 + 1 嘴的身体，
//!    就算把生产者全删掉也只剩 1 格嘴——捕食能力根本养不活自己。
//!    所以"强制互斥"对绝大多数混养体只会把它们变成**纯光合**，
//!    纯异养占比不但不会涨，可能还会跌。
//!
//! 用法：`cargo run --release --example mutex_probe -- 6000`
//! 第一个参数是 tick 数（默认 6000）。不写文件、不改世界，纯读。

use std::collections::HashMap;

use evosim_core::cell_type::ct;
use evosim_core::config::{AttrField, Config};
use evosim_core::factory::seed_origin_of_life;
use evosim_core::rng::Rng;
use evosim_core::world::World;
use evosim_core::worldgen::generate_demo_world;

#[derive(Default)]
struct Acc {
    total: usize,
    pure_photo: usize,
    pure_pred: usize,
    mixed: usize,
    neither: usize,
    /// 混养体的收入偏向：photo_share 分五档
    share_bins: [usize; 5],
    /// 混养体的身体构成：生产者格 / 嘴格（含杀手）
    photo_cells: Vec<usize>,
    mouth_cells: Vec<usize>,
    /// 走光合路线 / 走捕食路线之后的净收入保留率（1.0 = 不掉）
    keep_photo: Vec<f64>,
    keep_pred: Vec<f64>,
    /// 纯捕食者的状态
    pred_cells: Vec<usize>,
    pred_energy: Vec<f64>,
    pred_age: Vec<u64>,
}

fn median(v: &mut Vec<f64>) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn median_usize(v: &mut Vec<usize>) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_unstable();
    v[v.len() / 2] as f64
}

fn median_u64(v: &mut Vec<u64>) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_unstable();
    v[v.len() / 2] as f64
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let ticks: u64 = args.first().and_then(|s| s.parse().ok()).unwrap_or(6000);
    let seeds: Vec<u32> = if args.len() > 1 {
        args[1..].iter().filter_map(|s| s.parse().ok()).collect()
    } else {
        vec![1001, 777, 4242]
    };

    let mut acc = Acc::default();
    for &seed in &seeds {
        let mut cfg = Config::default();
        cfg.world.cols = 200;
        cfg.world.rows = 150;
        let mut w = World::new(cfg, Rng::new(seed));
        generate_demo_world(&mut w, 1.0, 1.0);
        seed_origin_of_life(&mut w);
        for _ in 0..ticks {
            w.step();
        }
        collect(&w, &mut acc);
    }

    let n = acc.total.max(1);
    println!(
        "── 「生产者 + 嘴」互斥诊断 · {ticks} tick × {} 种子 · 平均 {} 个体/种子 ──\n",
        seeds.len(),
        acc.total / seeds.len()
    );
    println!("【1】结构分类（成体）");
    println!("    纯光合（有生产者、无嘴）  {:>7}  {:>5.1}%", acc.pure_photo, pct(acc.pure_photo, n));
    println!("    纯捕食（有嘴、无生产者）  {:>7}  {:>5.1}%", acc.pure_pred, pct(acc.pure_pred, n));
    println!("    混养（两者都有）          {:>7}  {:>5.1}%", acc.mixed, pct(acc.mixed, n));
    println!("    都没有（纯储食/移动）     {:>7}  {:>5.1}%", acc.neither, pct(acc.neither, n));

    println!("\n【2】混养体的收入偏向（光合收入占比）");
    let labels = ["0–20%（实际是捕食者）", "20–40%", "40–60%（真两栖）", "60–80%", "80–100%（实际是光合）"];
    for (i, l) in labels.iter().enumerate() {
        println!(
            "    {:<26} {:>7}  {:>5.1}%  (占混养体)",
            l,
            acc.share_bins[i],
            pct(acc.share_bins[i], acc.mixed.max(1))
        );
    }

    println!("\n【3】混养体的身体构成（中位数）");
    println!(
        "    生产者格 {} / 嘴格（含杀手） {}",
        median_usize(&mut acc.photo_cells),
        median_usize(&mut acc.mouth_cells)
    );

    println!("\n【4】强制互斥后各走一条路，净收入保留率（中位数，1.0 = 完全不掉）");
    println!(
        "    只留生产者（去掉嘴）      {:.2}",
        median(&mut acc.keep_photo)
    );
    println!(
        "    只留嘴（去掉生产者）      {:.2}",
        median(&mut acc.keep_pred)
    );
    let better = acc.keep_photo.iter().zip(&acc.keep_pred).filter(|(a, b)| a > b).count();
    println!(
        "    其中「走光合更划算」的    {} / {}  ({:.1}%)",
        better,
        acc.keep_photo.len(),
        pct(better, acc.keep_photo.len().max(1))
    );

    println!("\n【5】纯捕食者的状态（中位数）");
    println!(
        "    格数 {} / 能量 {:.2} / 年龄 {} tick",
        median_usize(&mut acc.pred_cells),
        median(&mut acc.pred_energy),
        median_u64(&mut acc.pred_age)
    );

    println!("\n判读：如果【4】里「只留生产者」明显高于「只留嘴」，");
    println!("那强制互斥的结果是**世界变得更纯光合**，纯异养占比不会涨。");
}

fn pct(a: usize, b: usize) -> f64 {
    a as f64 * 100.0 / b as f64
}

fn collect(w: &World, acc: &mut Acc) {
    let cfg = &w.config;
    // 每格的维护成本（去掉一种细胞能省多少 upkeep）
    let cost_of = |id: u8| cfg.attr(id, AttrField::Cost);

    for org in &w.organisms {
        if !org.alive {
            continue;
        }
        // **芽不算**：它们的 anatomy 只有 1 格，蓝图在 growth_plan 里，
        // 拿它统计"身体构成"会全是 1:0。
        if org.growth_plan.is_some() {
            continue;
        }
        acc.total += 1;

        let mut producers = 0usize;
        let mut mouths = 0usize;
        for c in &org.anatomy.cells {
            if c.type_id == ct::PRODUCER {
                producers += 1;
            } else if c.type_id == ct::MOUTH || c.type_id == ct::KILLER {
                mouths += 1;
            }
        }

        let has_p = producers > 0;
        let has_m = mouths > 0;
        match (has_p, has_m) {
            (true, false) => acc.pure_photo += 1,
            (false, true) => {
                acc.pure_pred += 1;
                acc.pred_cells.push(org.cell_count());
                acc.pred_energy.push(org.energy);
                acc.pred_age.push(org.age);
            }
            (true, true) => {
                acc.mixed += 1;
                acc.photo_cells.push(producers);
                acc.mouth_cells.push(mouths);

                let photo = org.gain_photo.max(0.0);
                let bite = org.gain_bite.max(0.0);
                let share = if photo + bite > 1e-9 {
                    photo / (photo + bite)
                } else {
                    0.5
                };
                let bin = ((share * 5.0) as usize).min(4);
                acc.share_bins[bin] += 1;

                // 净收入保留率 = (新收入 − 新 upkeep) / (原收入 − 原 upkeep)
                // 用**累计值**算，比单 tick 稳。
                let age = org.age.max(1) as f64;
                let upkeep = org.upkeep_cost(cfg);
                let old_net = (photo + bite) - upkeep * age;
                if old_net > 1e-9 {
                    let save_m = mouths as f64 * cost_of(ct::MOUTH);
                    let save_p = producers as f64 * cost_of(ct::PRODUCER);
                    let new_photo = photo - (upkeep - save_m) * age;
                    let new_pred = bite - (upkeep - save_p) * age;
                    acc.keep_photo.push((new_photo / old_net).clamp(-1.0, 2.0));
                    acc.keep_pred.push((new_pred / old_net).clamp(-1.0, 2.0));
                }
            }
            (false, false) => acc.neither += 1,
        }
    }
}

// 让编译器别抱怨未使用的导入（HashMap 留给后续扩展）
#[allow(dead_code)]
fn _unused(_: HashMap<u8, u8>) {}
