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

use evosim_core::anatomy::Flags;
use evosim_core::cell_type::ct;
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
    /// **纯异养**：有嘴、但**没有**生产者细胞 —— 它必须靠吃别人活着。
    /// 这是"捕食生态位到底站没站住"最硬的读数；光看"嘴%"会把
    /// "顺手长了一张嘴的生产者"也算进去。
    hetero: f64,
    /// 带嘴的占比（含混养）
    mouth: f64,
    /// 带杀手的占比
    killer: f64,
    /// 繁殖尝试 / 成功 / 各种受阻（诊断：体型为什么涨不上去）
    tries: u64,
    ok: u64,
    blk_space: u64,
    blk_path: u64,
    blk_cap: u64,
    blk_small: u64,
    /// 有光格占用率（%）：`被生物占据的格子 / 有光格子`。
    /// 判断"世界到底挤不挤"的唯一硬指标——温度生态位需要"别处太挤"才有驱动力。
    occ: f64,
    /// 按温度分带：每带的（个体数, 平均体型）。5 带，每带 52 点温度。
    ///
    /// **必须分带看**：生物只占 5% 的格子，热区又只是一小块，
    /// 全局平均体型会把"大个体挤进热区"这件事完全稀释掉。
    bands: [(usize, f64); 5],
    /// 体型分布：1 格 / 2 格 / … 的个体占比（只统计前 10 档，够看形状）
    hist: [f64; 10],
}

fn run(
    p: f64,
    cap: usize,
    ticks: u64,
    seed: u32,
    free: bool,
    tries: usize,
    min_cells: usize,
    cost_mul: f64,
    upkeep_p: f64,
    heat: usize,
    temp_dmg: f64,
    photo: f64,
    ambient: f64,
    comfort: f64,
    growth: bool,
    gcost: f64,
) -> Row {
    let mut cfg = Config::default();
    cfg.life.breed_cost_exponent = p;
    cfg.life.max_body_cells = cap;
    // `free = true`：繁殖失败**不扣能量**（「找不到地方就继续攒着」）。
    // 旧行为是失败也白付一次 `food_needed`，而实测 86.5% 的尝试都会失败。
    cfg.life.repro_fail_costs_energy = !free;
    // `tries`：每次繁殖尝试最多试几个候选位置（1 = 旧行为）
    cfg.life.repro_search_tries = tries;
    // `min_cells`：最小可繁殖体型（1 = 旧行为）
    cfg.life.min_breed_cells = min_cells;
    // `costmul`：所有细胞的 cost 同乘一个数（用来调「成本占收入的比例」）
    if (cost_mul - 1.0).abs() > 1e-12 {
        for c in cfg.cells.iter_mut() {
            c.cost *= cost_mul;
        }
    }
    // `upkeep`：维持成本随体型的指数（1 = 线性；0.75 = Kleiber）
    cfg.life.upkeep_exponent = upkeep_p;
    // `heatdmg`：环境温度伤害（0 = 关闭）
    cfg.life.temp_damage = temp_dmg;
    // `photo`：光合速率（默认 0.12）。调高 = 世界更拥挤
    cfg.life.photosynthesis_rate = photo;
    // `ambient` / `comfort`：环境温度与舒适温度（摄氏度）
    cfg.world.ambient_temp = ambient;
    cfg.life.comfort_temp = comfort;
    // `growth` / `gcost`：成长过程与每格生长成本
    cfg.life.growth_enabled = growth;
    cfg.life.growth_cost = gcost;

    let mut w = World::new(cfg, Rng::new(seed));
    generate_demo_world(&mut w, 1.0, 1.0);

    // `heat=N`：在世界上半部分铺 N 块岩浆当热源。
    // 温度场靠 `update_fields` 扩散，所以下面每 tick 推一次。
    if heat > 0 {
        for k in 0..heat {
            let cx = (w.grid.cols as i32) * (k as i32 + 1) / (heat as i32 + 1);
            let cy = 20 + (k as i32 % 3) * 12;
            for dr in -4..=4 {
                for dc in -4..=4 {
                    if dc * dc + dr * dr <= 16 {
                        w.set_cell(cx + dc, cy + dr, ct::LAVA, -1);
                    }
                }
            }
        }
    }

    seed_origin_of_life(&mut w);
    spawn_population(&mut w, 8, 3);
    for _ in 0..ticks {
        w.step();
        // 主程序按 60 Hz 推温度场；这里跟 tick 走，保证热区真的热起来
        w.update_fields();
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

    let pct = |f: u16| {
        w.organisms.iter().filter(|o| o.anatomy.has(f)).count() as f64 * 100.0 / n as f64
    };
    Row {
        pop: w.organisms.len() as f64,
        body_avg: st.avg_cells,
        body_max: st.max_cells,
        species: w.species.alive_count as f64,
        hetero: w
            .organisms
            .iter()
            .filter(|o| {
                o.anatomy.has(Flags::MOUTH) && !o.anatomy.has(Flags::PRODUCER)
            })
            .count() as f64
            * 100.0
            / n as f64,
        mouth: pct(Flags::MOUTH),
        killer: pct(Flags::KILLER),
        occ: {
            let lit = w.light.level.iter().filter(|&&v| v > 0).count().max(1);
            let occ = w.grid.owners.iter().filter(|&&o| o >= 0).count();
            occ as f64 * 100.0 / lit as f64
        },
        bands: {
            let mut cnt = [0usize; 5];
            let mut sum = [0usize; 5];
            for o in &w.organisms {
                let t = w.fields.temp_at(&w.grid, o.col, o.row) as usize;
                let b = (t / 52).min(4);
                cnt[b] += 1;
                sum[b] += o.cell_count();
            }
            let mut out = [(0usize, 0.0f64); 5];
            for b in 0..5 {
                out[b] = (
                    cnt[b],
                    if cnt[b] == 0 {
                        0.0
                    } else {
                        sum[b] as f64 / cnt[b] as f64
                    },
                );
            }
            out
        },
        tries: w.repro_tries,
        ok: w.repro_ok,
        blk_space: w.repro_blocked_space,
        blk_path: w.repro_blocked_path,
        blk_cap: w.repro_blocked_cap,
        blk_small: w.repro_blocked_too_small,
        hist,
    }
}

/// 同参数跑多个种子，取平均（`body_max` 取最大，因为要看"最极端的那只"）
fn run_seeds(
    p: f64,
    cap: usize,
    ticks: u64,
    seeds: &[u32],
    free: bool,
    tries: usize,
    min_cells: usize,
    cost_mul: f64,
    upkeep_p: f64,
    heat: usize,
    temp_dmg: f64,
    photo: f64,
    ambient: f64,
    comfort: f64,
    growth: bool,
    gcost: f64,
) -> Row {
    let mut acc = Row {
        pop: 0.0,
        body_avg: 0.0,
        body_max: 0,
        species: 0.0,
        hetero: 0.0,
        mouth: 0.0,
        killer: 0.0,
        occ: 0.0,
        bands: [(0, 0.0); 5],
        tries: 0,
        ok: 0,
        blk_space: 0,
        blk_path: 0,
        blk_cap: 0,
        blk_small: 0,
        hist: [0f64; 10],
    };
    for &s in seeds {
        let r = run(
            p, cap, ticks, s, free, tries, min_cells, cost_mul, upkeep_p, heat, temp_dmg, photo,
            ambient, comfort, growth, gcost,
        );
        acc.pop += r.pop;
        acc.body_avg += r.body_avg;
        acc.body_max = acc.body_max.max(r.body_max);
        acc.species += r.species;
        acc.hetero += r.hetero;
        acc.mouth += r.mouth;
        acc.killer += r.killer;
        acc.occ += r.occ;
        for b in 0..5 {
            acc.bands[b].0 += r.bands[b].0;
            acc.bands[b].1 += r.bands[b].1;
        }
        acc.tries += r.tries;
        acc.ok += r.ok;
        acc.blk_space += r.blk_space;
        acc.blk_path += r.blk_path;
        acc.blk_cap += r.blk_cap;
        acc.blk_small += r.blk_small;
        for k in 0..10 {
            acc.hist[k] += r.hist[k];
        }
    }
    let n = seeds.len() as f64;
    acc.pop /= n;
    acc.body_avg /= n;
    acc.species /= n;
    acc.hetero /= n;
    acc.mouth /= n;
    acc.killer /= n;
    acc.occ /= n;
    for b in 0..5 {
        acc.bands[b].1 /= n;
    }
    for k in 0..10 {
        acc.hist[k] /= n;
    }
    acc
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // `free`：繁殖失败不扣能量。**默认就是 true**（跟 `Config` 默认一致），
    // 要旧行为请显式传 `strict`。
    let free = !args.iter().any(|a| a == "strict");
    // `tries=N`：每次繁殖尝试的候选位置数（1 = 旧行为）
    let tries: usize = args
        .iter()
        .find_map(|a| a.strip_prefix("tries=").and_then(|v| v.parse().ok()))
        .unwrap_or(1);
    // `mincells=N`：最小可繁殖体型（1 = 旧行为）
    let min_cells: usize = args
        .iter()
        .find_map(|a| a.strip_prefix("mincells=").and_then(|v| v.parse().ok()))
        .unwrap_or(1);
    // `costmul=M`：所有细胞 cost 同乘 M（调「成本占收入的比例」）
    let cost_mul: f64 = args
        .iter()
        .find_map(|a| a.strip_prefix("costmul=").and_then(|v| v.parse().ok()))
        .unwrap_or(1.0);
    // `upkeep=P`：维持成本随体型的指数（1 = 线性，0.75 = Kleiber）
    let upkeep_p: f64 = args
        .iter()
        .find_map(|a| a.strip_prefix("upkeep=").and_then(|v| v.parse().ok()))
        .unwrap_or(1.0);
    // `heat=N`：铺 N 块岩浆当热源
    let heat: usize = args
        .iter()
        .find_map(|a| a.strip_prefix("heat=").and_then(|v| v.parse().ok()))
        .unwrap_or(0);
    // `heatdmg=D`：环境温度每 tick 伤害（0 = 关闭）
    let temp_dmg: f64 = args
        .iter()
        .find_map(|a| a.strip_prefix("heatdmg=").and_then(|v| v.parse().ok()))
        .unwrap_or(0.0);
    // `photo=R`：光合速率（默认 0.12）。调高 = 世界更拥挤，用来测"热区避难"
    let photo: f64 = args
        .iter()
        .find_map(|a| a.strip_prefix("photo=").and_then(|v| v.parse().ok()))
        .unwrap_or(0.12);
    // `ambient=C`：环境温度（℃）；`comfort=C`：舒适温度（℃）
    let ambient: f64 = args
        .iter()
        .find_map(|a| a.strip_prefix("ambient=").and_then(|v| v.parse().ok()))
        .unwrap_or(22.0);
    let comfort: f64 = args
        .iter()
        .find_map(|a| a.strip_prefix("comfort=").and_then(|v| v.parse().ok()))
        .unwrap_or(35.0);
    // `growth`：启用成长过程（芽从 1 格长到目标体型）；`gcost=C`：每格生长成本
    let growth = args.iter().any(|a| a == "growth");
    let gcost: f64 = args
        .iter()
        .find_map(|a| a.strip_prefix("gcost=").and_then(|v| v.parse().ok()))
        .unwrap_or(1.0);
    // 位置参数：第一个数字是 ticks，其余是种子。
    // **必须把 ticks 从种子里剔掉**——第一版用 `filter_map(parse)` 全收，
    // 于是 `body_scan 10000 1001 1002 1003` 会把 10000 也当成种子跑（多跑一个）。
    let mut nums: Vec<u32> = args[1..].iter().filter_map(|s| s.parse().ok()).collect();
    let ticks: u64 = if nums.is_empty() {
        15000
    } else {
        nums.remove(0) as u64
    };
    let seeds: Vec<u32> = if nums.is_empty() {
        vec![1001, 777, 4242]
    } else {
        nums
    };
    assert!(!seeds.is_empty(), "至少要一个种子");

    println!(
        "── 体型扫描 · {ticks} tick · 种子 {:?} · 繁殖失败{}扣钱 · 搜索 {} · 最小体型 {} · cost×{:.1} · 维持 {:.2} · 岩浆 {} 块 · 热伤害 {:.2} · 光合 {:.2} · 成长 {} ──",
        seeds,
        if free { "**不**" } else { "" },
        tries,
        min_cells,
        cost_mul,
        upkeep_p,
        heat,
        temp_dmg,
        photo,
        if growth {
            format!("开(每格 {:.1})", gcost)
        } else {
            "关".to_string()
        }
    );
    println!("（同参数多跑几个种子取平均：这模拟在临界点上，单点差 4 倍很常见）\n");

    // ── 第一轮：只动指数 p，不限体型 ──
    println!("【1】繁殖成本指数 p（max_body_cells = 0，不限）");
    println!(
        "{:>5}  {:>7}  {:>9}  {:>9}  {:>7}  {:>8}  {:>7}  {:>6}  {:>7}",
        "p", "个体", "平均体型", "最大体型", "存活种", "有光占用", "异养%", "嘴%", "杀手%"
    );
    // 往 1 以下扫是重点：p < 1 让大体型便宜，才有向上的选择压力。
    // 只留 4 个点是为了控功耗——这台机器长时间满载会过热（clock_watchdog_timeout），
    // 要更细的曲线就手动加回来，然后盯着跑。
    // `p=X`（可多次）只跑指定的指数，省时间。默认跑整条曲线。
    let ps: Vec<f64> = {
        let v: Vec<f64> = args
            .iter()
            .filter_map(|a| a.strip_prefix("p=").and_then(|v| v.parse().ok()))
            .collect();
        if v.is_empty() {
            vec![0.5, 0.6, 0.7, 0.8, 1.0]
        } else {
            v
        }
    };
    let mut rows = Vec::new();
    for &p in &ps {
        let r = run_seeds(
            p, 0, ticks, &seeds, free, tries, min_cells, cost_mul, upkeep_p, heat, temp_dmg, photo,
            ambient, comfort, growth, gcost,
        );
        println!(
            "{:>5.2}  {:>7.0}  {:>9.2}  {:>9}  {:>7.1}  {:>7.1}%  {:>7.1}  {:>6.1}  {:>7.1}",
            p, r.pop, r.body_avg, r.body_max, r.species, r.occ, r.hetero, r.mouth, r.killer
        );
        rows.push((p, r));
    }

    // 繁殖受阻：这是「p 为什么推不动体型」的直接答案。
    //
    // `reproduce_organism` 是**先扣能量再检查空间**的，所以每一次"受阻"都意味着
    // 个体白付了一次全款（`food_needed`）。如果受阻率高，那「降低繁殖门槛（p < 1）」
    // 只会让个体更频繁地撞墙、更亏——规模经济和这个惩罚打架，后者赢。
    println!("\n【1b】繁殖受阻（累计次数 · 括号里是占尝试的百分比）");
    println!(
        "{:>5}  {:>8}  {:>16}  {:>16}  {:>16}  {:>13}  {:>13}",
        "p", "尝试", "成功", "没整块空地", "路径被挡", "到达上限", "体型太小"
    );
    for (p, r) in &rows {
        let pct = |v: u64| {
            if r.tries == 0 {
                0.0
            } else {
                v as f64 * 100.0 / r.tries as f64
            }
        };
        println!(
            "{:>5.2}  {:>8}  {:>8} ({:>5.1}%)  {:>8} ({:>5.1}%)  {:>8} ({:>5.1}%)  {:>7} ({:>4.1}%)  {:>7} ({:>4.1}%)",
            p,
            r.tries,
            r.ok,
            pct(r.ok),
            r.blk_space,
            pct(r.blk_space),
            r.blk_path,
            pct(r.blk_path),
            r.blk_cap,
            pct(r.blk_cap),
            r.blk_small,
            pct(r.blk_small)
        );
    }

    // 温度分带：生态位到底有没有分化的直接读数
    println!("\n【1c】按所在格温度分带（每带 52/255 · 个体数 / 平均体型）");
    print!("{:>5} ", "p");
    for b in 0..5 {
        print!("  {:>3}–{:>3}      ", b * 52, b * 52 + 51);
    }
    println!();
    for (p, r) in &rows {
        print!("{:>5.2} ", p);
        for b in 0..5 {
            let (c, a) = r.bands[b];
            if c == 0 {
                print!("{:>5} {:>5}      ", "—", "—");
            } else {
                print!("{:>5} {:>5.2}      ", c, a);
            }
        }
        println!();
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
        let r = run_seeds(
            1.0, cap, ticks, &seeds, free, tries, min_cells, cost_mul, upkeep_p, heat, temp_dmg, photo,
            ambient, comfort, growth, gcost,
        );
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
    println!("- **异养% 才是判据**：体型涨上去但异养还是 0，说明「猎物变厚 → 捕食可行」这条推论错了");
}
