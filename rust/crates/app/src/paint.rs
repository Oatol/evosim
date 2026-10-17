//! 世界画笔 —— 鼠标位置 → 格子 → 写世界。
//!
//! 与 1.x 的语义保持一致，但多一条保护：**不覆盖活着的生物身体**。
//! 1.x 的「空」笔刷是无差别覆盖，刷过生物会把它们的细胞抹掉、留下 owner 表错乱的残骸；
//! 2.0 里地形笔刷一律跳过 `owners >= 0` 的格子，橡皮擦也只擦地形。

use evosim_core::cell_type::{ct, is_terrain};
use evosim_core::world::World;

/// 橡皮擦的哨兵值（不是合法的细胞类型 id）
pub const ERASER: u8 = 255;

/// 在 (col,row) 处以 `size × size` 的笔刷落笔
pub fn apply_brush(world: &mut World, col: i32, row: i32, brush: u8, size: i32) {
    let size = size.max(1);
    let offset = (size - 1) / 2;
    for dr in 0..size {
        for dc in 0..size {
            let c = col - offset + dc;
            let r = row - offset + dr;
            let Some(idx) = world.grid.index(c, r) else {
                continue;
            };

            if brush == ERASER {
                // 只擦地形：墙 / 岩浆 / 沼泽 / 辐射带 / 腐殖质。食物与生物一律不动。
                if is_terrain(world.grid.floor[idx]) {
                    world.set_cell_at(idx, ct::EMPTY, -1);
                }
                continue;
            }

            // 别把活着的身体抹掉
            if world.grid.owners[idx] >= 0 {
                continue;
            }
            world.set_cell_at(idx, brush, -1);
        }
    }
}

/// 屏幕像素 → 格子坐标（相机可能在世界外，交给 Grid::index 折回）
pub fn screen_to_cell(cam: &evosim_render::Camera, px: f32, py: f32) -> (i32, i32) {
    let (wx, wy) = cam.screen_to_world(px, py);
    (wx.floor() as i32, wy.floor() as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use evosim_core::config::Config;
    use evosim_core::factory::create_origin_organism;
    use evosim_core::rng::Rng;
    use evosim_core::world::World;

    fn small() -> World {
        let mut cfg = Config::default();
        cfg.world.cols = 32;
        cfg.world.rows = 32;
        World::new(cfg, Rng::new(1))
    }

    #[test]
    fn brush_paints_a_square() {
        let mut w = small();
        w.fill_grid(ct::EMPTY, false);
        apply_brush(&mut w, 10, 10, ct::WALL, 3);
        // 3×3 覆盖 (9..11)
        for r in 9..=11 {
            for c in 9..=11 {
                assert_eq!(w.type_at(c, r), ct::WALL, "({},{}) 应该被画成墙", c, r);
            }
        }
        assert_eq!(w.type_at(12, 10), ct::EMPTY, "笔刷不该溢出到 (12,10)");
    }

    #[test]
    fn eraser_removes_terrain_but_keeps_food() {
        let mut w = small();
        w.fill_grid(ct::EMPTY, false);
        w.set_cell(5, 5, ct::WALL, -1);
        w.set_cell(6, 5, ct::FOOD, -1);
        apply_brush(&mut w, 5, 5, ERASER, 3);
        assert_eq!(w.type_at(5, 5), ct::EMPTY, "墙应该被擦掉");
        assert_eq!(w.type_at(6, 5), ct::FOOD, "食物必须保留");
    }

    #[test]
    fn brush_does_not_overwrite_organisms() {
        let mut w = small();
        w.fill_grid(ct::EMPTY, false);
        let org = create_origin_organism(&mut w, 16, 16);
        w.add_organism(org);
        let before = w.type_at(16, 16);

        apply_brush(&mut w, 16, 16, ct::WALL, 5);

        assert_eq!(w.type_at(16, 16), before, "不该把活着的身体涂成墙");
        assert_eq!(w.organisms.len(), 1);
    }
}
