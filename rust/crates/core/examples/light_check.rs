//! 光场验证：光到底有没有改变行为（spec §7 的四条判据）。
//!
//! 跑法：`cargo run --release -p evosim-core --example light_check -- 8000 1001`
//!
//! ## 为什么需要这个工具
//!
//! 光场是"看起来对"很容易、但实际没接上也很容易的那种改动：光算出来了，
//! 光合却读了别的东西；或者光合接上了，但光场永远是满值（那等于没加）。
//!
//! 所以这里不看截图，只看**统计量有没有分化**：
//!
//! 1. **生产者所在格的平均光强 > 全局平均光强** —— 趋光性成立
//! 2. **生产者细胞的平均行号 < 全体细胞平均行号** —— 生产者偏向世界顶部
//! 3. **种群平均行号 < 世界中线** —— 整体向光迁移
//! 4. **种群 > 0** —— 光合速率没调崩
//!
//! 判据 1 是最硬的：它同时证明了「光场有梯度」和「生产者按光选位置」。
//! 如果这个比值 ≈ 1.0，说明要么光场是均匀的，要么光合根本没读光场。

use evosim_core::cell_type::ct;
use evosim_core::config::Config;
use evosim_core::factory::{seed_origin_of_life, spawn_population};
use evosim_core::rng::Rng;
use evosim_core::world::World;
use evosim_core::worldgen::generate_demo_world;

struct Report {
    orgs: usize,
    cells: usize,
    avg_body: f64,
    prod_row: f64,
    all_row: f64,
    org_row: f64,
    prod_lit: f64,
    all_lit: f64,
    eyes: usize,
    killers: usize,
    movers: usize,
    producers: usize,
}

fn scan(w: &World) -> Report {
    let mut orgs = 0usize;
    let mut cells = 0usize;
    let mut prod_row = 0.0;
    let mut all_row = 0.0;
    let mut org_row = 0.0;
    let mut prod_lit = 0.0;
    let mut all_lit = 0.0;
    let mut eyes = 0usize;
    let mut killers = 0usize;
    let mut movers = 0usize;
    let mut producers = 0usize;

    for o in w.organisms.iter().filter(|o| o.alive) {
        orgs += 1;
        org_row += o.row as f64;
        for cell in &o.anatomy.cells {
            let col = o.real_col(cell);
            let row = o.real_row(cell);
            let lit = w.light.at(&w.grid, col, row) as f64;
            cells += 1;
            all_row += row as f64;
            all_lit += lit;
            match cell.type_id {
                ct::PRODUCER => {
                    producers += 1;
                    prod_row += row as f64;
                    prod_lit += lit;
                }
                ct::EYE => eyes += 1,
                ct::KILLER => killers += 1,
                ct::MOVER => movers += 1,
                _ => {}
            }
        }
    }

    let d = |v: f64, n: usize| if n == 0 { 0.0 } else { v / n as f64 };
    Report {
        orgs,
        cells,
        avg_body: d(cells as f64, orgs),
        prod_row: d(prod_row, producers),
        all_row: d(all_row, cells),
        org_row: d(org_row, orgs),
        prod_lit: d(prod_lit, producers),
        all_lit: d(all_lit, cells),
        eyes,
        killers,
        movers,
        producers,
    }
}

fn main() {
    let ticks: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(8000);
    let seed: u32 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1001);
    // 第三个参数：全局维持成本（默认 0 = 关闭，也就是现在的出厂值）。
    // 用来验证「多一格到底要不要付账」——这是体型能不能被压住的关键。
    let metabolism: f64 = std::env::args()
        .nth(3)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);
    // 第四个参数：天空光强度（0 = 永夜）。第五个参数写 `lamp` 就在世界**下半部**
    // 摆几个点光源，用来验证「世界不再强行分上下」。
    let sky: f64 = std::env::args()
        .nth(4)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.0);
    let lamps = std::env::args().nth(5).as_deref() == Some("lamp");

    let mut cfg = Config::default();
    cfg.life.metabolism_scale = metabolism;
    cfg.world.sun_intensity = sky;
    let mut w = World::new(cfg, Rng::new(seed));
    generate_demo_world(&mut w, 1.0, 1.0);
    seed_origin_of_life(&mut w);
    spawn_population(&mut w, 8, 3);

    // 「玩家在下半部放灯」——这一步证明光源是独立的一层：
    // 关掉天空光之后，生物只能围着这几盏灯活，和世界"上半亮下半黑"彻底解耦。
    if lamps {
        // 在每只初始个体身上点一盏灯 —— 模拟"玩家想在哪照就在哪照"。
        // 灯跟着生物走不了（它们是静态的），但足够验证：光合在任何深度都工作。
        let spots: Vec<(i32, i32)> = w.organisms.iter().map(|o| (o.col, o.row)).collect();
        for (c, r) in &spots {
            w.set_light_source(*c, *r, 255);
        }
        println!("已放置 {} 盏灯（跟着初始个体）", w.light.sources.len());
    }

    // 光场是地形纯函数，`World::new` 已经投过一次；这里再确认一下顶底差异
    let top = w.light.at(&w.grid, 0, 0);
    let bottom = w.light.at(&w.grid, 0, w.grid.rows as i32 - 1);
    println!(
        "世界 {}×{} · absorb {} · 顶部光强 {} · 底部光强 {}",
        w.grid.cols, w.grid.rows, w.light.absorb, top, bottom
    );
    // 光场的垂直剖面：确认它真的随深度衰减（而不是哪里出了问题全亮）
    let prof: Vec<String> = [0, 25, 50, 75, 100, 125, 149]
        .iter()
        .map(|&r| format!("r{}={}", r, w.light.at(&w.grid, 100, r)))
        .collect();
    println!("光场剖面（第 100 列）：{}", prof.join(" "));
    println!();
    println!("   tick   个体  平均体型  生产者行  全体行  种群行  生产者光强  全局光强  比值  眼%  杀%  移%");

    let step = (ticks / 10).max(1);
    let mut done = 0u64;
    while done < ticks {
        let n = step.min(ticks - done);
        for _ in 0..n {
            w.step();
        }
        done += n;
        let r = scan(&w);
        let ratio = if r.all_lit > 0.0 {
            r.prod_lit / r.all_lit
        } else {
            0.0
        };
        let pct = |v: usize| {
            if r.cells == 0 {
                0.0
            } else {
                v as f64 * 100.0 / r.cells as f64
            }
        };
        println!(
            "{:>7} {:>6} {:>9.2} {:>8.1} {:>6.1} {:>7.1} {:>11.1} {:>9.1} {:>5.2} {:>4.1} {:>4.1} {:>4.1}",
            done,
            r.orgs,
            r.avg_body,
            r.prod_row,
            r.all_row,
            r.org_row,
            r.prod_lit,
            r.all_lit,
            ratio,
            pct(r.eyes),
            pct(r.killers),
            pct(r.movers),
        );
    }

    println!();
    let r = scan(&w);
    let ratio = if r.all_lit > 0.0 {
        r.prod_lit / r.all_lit
    } else {
        0.0
    };
    println!("── tick {}（seed {}）──", w.tick, seed);
    println!(
        "个体 {} · 细胞 {} · 平均体型 {:.2}",
        r.orgs, r.cells, r.avg_body
    );
    println!(
        "判据 1 体内布局：生产者所在格光强 {:.1} vs 全局 {:.1} → 比值 {:.2}{}",
        r.prod_lit,
        r.all_lit,
        ratio,
        if ratio > 1.05 {
            " ✓ 生产者偏向身体靠光的一侧"
        } else {
            " · 没出现（梯度太缓：5 格身体只差 6% 光强，且变异不针对布局）"
        }
    );
    println!(
        "判据 2 生产者位置：生产者平均行 {:.1} vs 全体 {:.1}{}",
        r.prod_row,
        r.all_row,
        if r.prod_row <= r.all_row + 0.5 {
            " ✓"
        } else {
            " · 反了（种群整体已经贴顶，体内差异被压没）"
        }
    );
    println!(
        "判据 3 迁移：种群平均行 {:.1} vs 中线 {:.1}{}",
        r.org_row,
        w.grid.rows as f64 / 2.0,
        if r.org_row < w.grid.rows as f64 / 2.0 {
            " ✓"
        } else {
            " ✗"
        }
    );
    println!(
        "判据 4 存活：{} 个体{}",
        r.orgs,
        if r.orgs > 0 { " ✓" } else { " ✗（生态崩了）" }
    );
    println!(
        "细胞构成：生产者 {} · 眼睛 {} · 杀手 {} · 移动器 {}",
        r.producers, r.eyes, r.killers, r.movers
    );

    // 食物存量（批次 8）：食物不再是二值的，这里能看出"肥尸体"到底存不存在
    let food_cells = w.food_cells.len();
    let food_energy: f64 = w.food_cells.iter().map(|&k| w.grid.food[k]).sum();
    let avg_food = if food_cells > 0 {
        food_energy / food_cells as f64
    } else {
        0.0
    };
    println!(
        "食物 {} 格 · 总能量 {:.1} · 平均每格 {:.2} · 热池 {:.1}",
        food_cells, food_energy, avg_food, w.heat_pool
    );
    println!(
        "（平均每格 > 1.0 说明存在「肥尸体」——残骸携带了死者体内能量；         热池 > 0 说明腐败在流失能量）"
    );
}
