//! 世界地形速览（批次 44 的诊断工具，只读）。
//!
//! 把生成出来的世界打成一张 ASCII 图，用来**肉眼确认地形摆得对**：
//! 河流是不是从上到下、岩浆是不是压在底部、洞穴是不是在地下一半。
//!
//! 用法：
//! ```text
//! cargo run --release --example map_dump -- [seed] [--flat]
//! ```
//!
//! 说明：这只是诊断，不参与模拟，也不写任何文件。

use evosim_core::cell_type::ct;
use evosim_core::config::Config;
use evosim_core::rng::Rng;
use evosim_core::world::World;
use evosim_core::worldgen::generate_demo_world;

fn glyph(ty: u8) -> char {
    match ty {
        ct::EMPTY => '·',
        ct::FOOD => ',',
        ct::WALL => '#',
        ct::WATER => '~',
        ct::ICE => '*',
        ct::SWAMP => 's',
        ct::RAD => 'r',
        ct::HUMUS => ':',
        ct::LAVA => '^',
        _ => '?',
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let seed: u32 = args
        .iter()
        .find_map(|a| a.parse::<u32>().ok())
        .unwrap_or(1);
    let flat = args.iter().any(|a| a == "--flat");

    let mut cfg = Config::default();
    cfg.world.cols = 160;
    cfg.world.rows = 80;
    cfg.world.rich_terrain = !flat;
    let mut w = World::new(cfg, Rng::new(seed));
    generate_demo_world(&mut w, 1.0, 1.0);

    let mut counts = std::collections::BTreeMap::new();
    for &ty in &w.grid.floor {
        *counts.entry(ty).or_insert(0usize) += 1;
    }

    println!(
        "seed {} · {}×{} · 丰富地形 {}",
        seed,
        w.grid.cols,
        w.grid.rows,
        if w.config.world.rich_terrain { "开" } else { "关" }
    );
    for (ty, n) in &counts {
        let pct = *n as f64 * 100.0 / w.grid.len as f64;
        println!(
            "  {:<8} {:>6} 格  {:>5.1}%",
            evosim_core::cell_type::label(*ty),
            n,
            pct
        );
    }
    println!();

    // 每 2 列取 1，避免一行 160 字符在终端里折行
    for row in 0..w.grid.rows {
        let mut line = String::with_capacity(w.grid.cols);
        for col in (0..w.grid.cols).step_by(2) {
            let i = w.grid.index(col as i32, row as i32).unwrap();
            line.push(glyph(w.grid.floor[i]));
        }
        println!("{:>3} {}", row, line);
    }
}
