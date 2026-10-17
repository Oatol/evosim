//! 世界生成器。与 JS 版 `src/core/WorldGen.js` 对应。
//!
//! 洞穴状墙体（成簇生长）+ 成片食物草甸 + 零散食物。
//! 所有随机走 world.rng，同一 seed 生成同一世界。

use crate::cell_type::ct;
use crate::world::World;

fn paint_blob(world: &mut World, cx: f64, cy: f64, radius: f64, ty: u8, jitter: f64) {
    let r2 = radius * radius;
    let c0 = (cx - radius - 1.0).floor() as i32;
    let c1 = (cx + radius + 1.0).ceil() as i32;
    let r0 = (cy - radius - 1.0).floor() as i32;
    let r1 = (cy + radius + 1.0).ceil() as i32;

    for row in r0..=r1 {
        for col in c0..=c1 {
            let dx = col as f64 - cx;
            let dy = row as f64 - cy;
            let d2 = dx * dx + dy * dy;
            if d2 > r2 {
                continue;
            }
            if jitter > 0.0 && d2 > r2 * (1.0 - jitter) {
                continue;
            }
            if let Some(index) = world.grid.index(col, row) {
                world.set_cell_at(index, ty, -1);
            }
        }
    }
}

pub fn generate_demo_world(world: &mut World, wall_density: f64, food_density: f64) {
    let cols = world.grid.cols;
    let rows = world.grid.rows;
    let area = (cols * rows) as f64;

    world.reset(false);

    // ── 1. 洞穴状墙体：成簇生长 ──
    let wall_seeds = (((area / 2600.0) * wall_density).round() as i32).max(6);
    for _ in 0..wall_seeds {
        let mut cx = world.rng.range(0.0, cols as f64);
        let mut cy = world.rng.range(0.0, rows as f64);
        let steps = world.rng.int_range(10, 34);
        let mut radius = world.rng.range(1.6, 3.4);
        for _ in 0..steps {
            paint_blob(world, cx, cy, radius, ct::WALL, 0.45);
            cx += world.rng.gaussian(0.0, 2.1);
            cy += world.rng.gaussian(0.0, 2.1);
            radius = (radius + world.rng.gaussian(0.0, 0.25)).max(1.1);
        }
    }

    // ── 2. 食物草甸 ──
    let meadow_count = (((area / 4200.0) * food_density).round() as i32).max(4);
    for _ in 0..meadow_count {
        let cx = world.rng.range(0.0, cols as f64);
        let cy = world.rng.range(0.0, rows as f64);
        let radius = world.rng.range(6.0, 15.0);
        let r2 = radius * radius;
        let row_from = (cy - radius).floor() as i32;
        let row_to = (cy + radius).ceil() as i32;
        let col_from = (cx - radius).floor() as i32;
        let col_to = (cx + radius).ceil() as i32;
        for row in row_from..=row_to {
            for col in col_from..=col_to {
                let dx = col as f64 - cx;
                let dy = row as f64 - cy;
                if dx * dx + dy * dy > r2 {
                    continue;
                }
                let index = match world.grid.index(col, row) {
                    Some(i) => i,
                    None => continue,
                };
                if world.grid.types[index] != ct::EMPTY {
                    continue;
                }
                if world.rng.float() < 0.34 {
                    world.set_cell_at(index, ct::FOOD, -1);
                }
            }
        }
    }

    // ── 3. 零散食物 ──
    let scatter = ((area / 220.0) * food_density).round() as i32;
    for _ in 0..scatter {
        let col = world.rng.int(cols) as i32;
        let row = world.rng.int(rows) as i32;
        if let Some(index) = world.grid.index(col, row) {
            if world.grid.types[index] == ct::EMPTY {
                world.set_cell_at(index, ct::FOOD, -1);
            }
        }
    }
}

pub fn generate_empty_world(world: &mut World) {
    world.reset(false);
}
