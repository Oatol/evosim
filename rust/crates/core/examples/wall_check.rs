//! 诊断：有光区是不是被光合生物挤成一道致密的墙。
//!
//! 用户报的现象：光合生物繁殖太快，很快把有光区占满，组成一道致密的墙，
//! 狩猎生物完全进不去，也吃不到食物。
//!
//! 这个例子只量四件事，不加任何判断：
//!
//! 1. **有光区的占用率** —— 有光格子里有多少被生物身体压着
//! 2. **被困住的个体数** —— 四邻里一个空格都没有的个体（动不了 = 进不去）
//! 3. **食物存量** —— 有光区里还剩多少 FOOD 格（狩猎生物的饭）
//! 4. **捕食者占比** —— 带杀手/嘴的个体有多少
//!
//! 用法：`cargo run --release -p evosim-core --example wall_check -- [ticks] [seed]`

use evosim_core::cell_type::ct;
use evosim_core::config::Config;
use evosim_core::factory::{seed_origin_of_life, spawn_population};
use evosim_core::rng::Rng;
use evosim_core::world::World;
use evosim_core::worldgen::generate_demo_world;

fn main() {
    // 参数解析：先收成一列，**标志位和位置参数分开认**。
    // （第一版按位置硬吃，`shade=0.25` 落进「光合速率」那一格被当成解析失败吞掉了。）
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut flags: Vec<&str> = Vec::new();
    let mut pos: Vec<&str> = Vec::new();
    for a in &argv {
        if a == "lamp" || a == "self" || a == "shade" || a.starts_with("shade=") {
            flags.push(a);
        } else {
            pos.push(a);
        }
    }

    let ticks: u64 = pos.first().and_then(|s| s.parse().ok()).unwrap_or(4000);
    let seed: u32 = pos.get(1).and_then(|s| s.parse().ok()).unwrap_or(1001);

    // `lamp`：天空光关掉，只放 4 盏灯。
    // 这是"墙"最可能出现的场景——光集中在几个点，生物只能围着灯长，
    // 于是局部密度极高（全局平均看不出来）。
    let lamps = flags.contains(&"lamp");

    // `shade` / `shade=0.25`：打开**遮挡**的旁路模拟（不改 core，见 `apply_shading`）。
    // 再加 `self` 表示自己也挡自己（物理正确，但会惩罚大体型）；不加则只数别的个体。
    let mut shade: Option<f64> = None;
    for f in &flags {
        if *f == "shade" {
            shade = Some(0.25);
        } else if let Some(v) = f.strip_prefix("shade=") {
            shade = v.parse().ok();
        }
    }
    let self_shade = flags.contains(&"self");

    let mut cfg = Config::default();
    // 世界尺寸（小世界 = 有光区很快被填满）
    if let Some(n) = pos.get(2).and_then(|s| s.parse::<usize>().ok()) {
        cfg.world.cols = n;
        cfg.world.rows = n * 3 / 4;
    }
    // 光合速率（默认 0.12）。调高 = 收入变多 = 繁殖变快。
    if let Some(v) = pos.get(3).and_then(|s| s.parse::<f64>().ok()) {
        cfg.life.photosynthesis_rate = v;
    }
    if lamps {
        cfg.world.sun_intensity = 0.0;
    }
    let mut w = World::new(cfg, Rng::new(seed));
    generate_demo_world(&mut w, 1.0, 1.0);
    seed_origin_of_life(&mut w);
    spawn_population(&mut w, 8, 3);
    if lamps {
        let (cols, rows) = (w.grid.cols as i32, w.grid.rows as i32);
        for (fx, fy) in [(0.25, 0.3), (0.75, 0.3), (0.25, 0.7), (0.75, 0.7)] {
            w.set_light_source((cols as f64 * fx) as i32, (rows as f64 * fy) as i32, 255);
        }
    }

    println!(
        "世界 {}×{} · seed {} · 光合 {:.3}{}",
        w.grid.cols,
        w.grid.rows,
        seed,
        w.config.life.photosynthesis_rate,
        match shade {
            Some(k) if self_shade => format!(" · 遮挡 k={} （含自身）", k),
            Some(k) => format!(" · 遮挡 k={} （只算别人）", k),
            None => " · 无遮挡（基线）".to_string(),
        }
    );
    println!("tick   个体  有光格  有光占用%  致密%  被困个体  有光食物  杀手%  嘴%  眼%  移动%  平均体型");

    for t in 0..=ticks {
        if t % 500 == 0 {
            report(&w, t);
        }
        w.step();
        if let Some(k) = shade {
            apply_shading(&mut w, k, self_shade);
        }
    }
    profile(&w);
}

/// 遮挡能见度：同列上方最多数几行
const SHADE_DEPTH: i32 = 8;

/// **不落地的遮挡模拟**：每 tick 之后把被身体挡住的那部分光合产出扣掉。
///
/// 真实实现（如果要做 C 方案）应该在 `run_cell_function` 里算，
/// 这里为了不动 core，改成"事后扣账"：晚一个 tick 生效，
/// 而且这一 tick 的能量可能已经够触发繁殖了——所以**遮挡效果被低估**。
/// 如果低估的版本都能把占用率压下来，真做只会更狠。
///
/// 模型：一个生产者细胞的正上方（同列、上面 `SHADE_DEPTH` 行内）每有一个
/// 身体格，光就乘 `1-k`。k=0.25 时 4 层组织只剩 32% 的产出。
fn apply_shading(w: &mut World, k: f64, self_shade: bool) {
    let rate = w.config.life.photosynthesis_rate;
    let mult = w.config.cells[ct::PRODUCER as usize].prod_mult;

    let mut losses: Vec<(usize, f64)> = Vec::new();
    let mut produced = 0.0f64;
    let mut lost = 0.0f64;

    for i in 0..w.organisms.len() {
        if !w.organisms[i].alive {
            continue;
        }
        let id = w.organisms[i].id as i32;
        let n = w.organisms[i].anatomy.cells.len();
        let mut loss = 0.0;
        for kidx in 0..n {
            let cell = w.organisms[i].anatomy.cells[kidx];
            if cell.type_id != ct::PRODUCER {
                continue;
            }
            let c = w.organisms[i].real_col(&cell);
            let r = w.organisms[i].real_row(&cell);
            let lit = w.light.at(&w.grid, c, r) as f64 / 255.0;
            if lit <= 0.0 {
                continue;
            }
            let gain = lit * rate * mult;
            produced += gain;

            let mut occ = 0;
            for d in 1..=SHADE_DEPTH {
                let Some(j) = w.grid.index(c, r - d) else { continue };
                let owner = w.grid.owners[j];
                if owner < 0 || (!self_shade && owner == id) {
                    continue;
                }
                occ += 1;
            }
            let s = (1.0 - k).powi(occ);
            let l = gain * (1.0 - s);
            loss += l;
            lost += l;
        }
        if loss > 0.0 {
            losses.push((i, loss));
        }
    }

    for (i, loss) in losses {
        let e = &mut w.organisms[i].energy;
        *e = (*e - loss).max(0.0);
    }

    // 记账：把这次"本该有但没有"的产出记进热池，和真实实现保持一致
    w.heat_pool += lost;
    let _ = produced;
}

/// 按行分带看占用率：如果"墙"是真的，它应该出现在光最强的那几行。
fn profile(w: &World) {
    let cols = w.grid.cols as i32;
    let rows = w.grid.rows as i32;
    println!();
    println!("按行分带（光强 → 占用率）：");
    let band = (rows / 10).max(1);
    let mut r = 0;
    while r < rows {
        let end = (r + band).min(rows);
        let (mut lit, mut occ, mut lum) = (0usize, 0usize, 0u64);
        for row in r..end {
            for c in 0..cols {
                let Some(i) = w.grid.index(c, row) else { continue };
                let l = w.light.at(&w.grid, c, row);
                lum += l as u64;
                if l >= 16 {
                    lit += 1;
                    if w.grid.owners[i] >= 0 {
                        occ += 1;
                    }
                }
            }
        }
        let n = ((end - r) * cols).max(1) as f64;
        println!(
            "  行 {:>3}–{:>3}  光强 {:>3}  有光 {:>5} 格  占用 {:>5.1}%",
            r,
            end - 1,
            lum as f64 / n,
            lit,
            occ as f64 * 100.0 / lit.max(1) as f64
        );
        r = end;
    }
}

fn report(w: &World, tick: u64) {
    let cols = w.grid.cols as i32;
    let rows = w.grid.rows as i32;

    let mut lit_cells = 0usize;
    let mut lit_occupied = 0usize;
    let mut lit_food = 0usize;
    // 有光区里有多少格是"地形可走但被生物占着"——墙的主体
    for r in 0..rows {
        for c in 0..cols {
            let Some(i) = w.grid.index(c, r) else { continue };
            if w.light.at(&w.grid, c, r) < 16 {
                continue;
            }
            lit_cells += 1;
            if w.grid.owners[i] >= 0 {
                lit_occupied += 1;
            } else if w.grid.types[i] == ct::FOOD {
                lit_food += 1;
            }
        }
    }

    // 「墙」的局部定义：一个被生物占着的格子，八邻里有多少也被占着。
    // 8 个里 >=6 个被占 = 这格在致密组织内部（不是边缘），走不进去也看不见外面。
    let mut mat = 0usize;
    for r in 0..rows {
        for c in 0..cols {
            let Some(i) = w.grid.index(c, r) else { continue };
            if w.grid.owners[i] < 0 || w.light.at(&w.grid, c, r) < 16 {
                continue;
            }
            let mut occ = 0;
            for dr in -1..=1i32 {
                for dc in -1..=1i32 {
                    if dc == 0 && dr == 0 {
                        continue;
                    }
                    if let Some(j) = w.grid.index(c + dc, r + dr) {
                        if w.grid.owners[j] >= 0 {
                            occ += 1;
                        }
                    }
                }
            }
            if occ >= 6 {
                mat += 1;
            }
        }
    }

    let orgs: Vec<_> = w.organisms.iter().filter(|o| o.alive).collect();
    let n = orgs.len().max(1) as f64;

    // 被困：四邻里没有任何"能走进去"的空格（EMPTY 且无主）
    let mut trapped = 0usize;
    for o in &orgs {
        let mut free = 0;
        for (dc, dr) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            for cell in &o.anatomy.cells {
                let c = o.real_col(cell) + dc;
                let r = o.real_row(cell) + dr;
                if let Some(i) = w.grid.index(c, r) {
                    if w.grid.types[i] == ct::EMPTY && w.grid.owners[i] < 0 {
                        free += 1;
                    }
                }
            }
        }
        if free == 0 {
            trapped += 1;
        }
    }

    let count = |f: u16| orgs.iter().filter(|o| o.anatomy.has(f)).count() as f64;
    let cells: usize = orgs.iter().map(|o| o.cell_count()).sum();

    println!(
        "{:<6} {:>5} {:>7} {:>9.1}% {:>6.1}% {:>9} {:>9} {:>6.1} {:>4.1} {:>4.1} {:>5.1} {:>9.2}",
        tick,
        orgs.len(),
        lit_cells,
        lit_occupied as f64 * 100.0 / lit_cells.max(1) as f64,
        mat as f64 * 100.0 / lit_occupied.max(1) as f64,
        trapped,
        lit_food,
        count(evosim_core::anatomy::Flags::KILLER) * 100.0 / n,
        count(evosim_core::anatomy::Flags::MOUTH) * 100.0 / n,
        count(evosim_core::anatomy::Flags::EYE) * 100.0 / n,
        count(evosim_core::anatomy::Flags::MOVER) * 100.0 / n,
        cells as f64 / n,
    );
}
