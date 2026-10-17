//! 世界画笔 —— 鼠标位置 → 格子 → 写世界。
//!
//! 三种笔刷形状：方形（默认）/ 圆形 / 喷枪。
//! 与 1.x 的语义一致，但多一条保护：**不覆盖活着的生物身体**。

use evosim_core::cell_type::{ct, is_terrain};
use evosim_core::world::World;

/// 橡皮擦的哨兵值（不是合法的细胞类型 id）
pub const ERASER: u8 = 255;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BrushShape {
    Square,
    Circle,
    Spray,
}

impl BrushShape {
    pub fn label(self) -> &'static str {
        match self {
            BrushShape::Square => "方形",
            BrushShape::Circle => "圆形",
            BrushShape::Spray => "喷枪",
        }
    }

    pub fn all() -> [BrushShape; 3] {
        [BrushShape::Square, BrushShape::Circle, BrushShape::Spray]
    }
}

/// 喷枪用的局部哈希随机（**不碰 world.rng**）
///
/// 关键：画笔是"玩家的手"，不能消耗模拟的随机序列，否则同一存档
/// 因为"画了两笔"就走向不同分支。这里用坐标 + tick 混一个局部种子。
#[inline]
fn spray_hash(col: i32, row: i32, tick: u64) -> u32 {
    let mut h = (col as u32).wrapping_mul(0x9e37_79b9)
        ^ (row as u32).wrapping_mul(0x85eb_ca6b)
        ^ (tick as u32).wrapping_mul(0xc2b2_ae35);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2545_f491);
    h ^= h >> 13;
    h
}

/// 在 (col,row) 处以 `size × size` 的笔刷落笔
pub fn apply_brush(world: &mut World, col: i32, row: i32, brush: u8, size: i32, shape: BrushShape) {
    let size = size.max(1);
    let offset = (size - 1) / 2;
    let radius = (size as f32) * 0.5;
    let r2 = radius * radius;
    let tick = world.tick;

    for dr in 0..size {
        for dc in 0..size {
            let c = col - offset + dc;
            let r = row - offset + dr;

            let fx = dc as f32 - offset as f32;
            let fy = dr as f32 - offset as f32;
            match shape {
                BrushShape::Square => {}
                BrushShape::Circle => {
                    // 以中心为圆心；+0.35 让 1 格 / 3 格的手感不至于太小
                    if fx * fx + fy * fy > r2 + 0.35 {
                        continue;
                    }
                }
                BrushShape::Spray => {
                    if fx * fx + fy * fy > r2 + 1.2 {
                        continue;
                    }
                    // 约 45% 的落点保留，形成飞散感
                    if spray_hash(c, r, tick) % 100 >= 45 {
                        continue;
                    }
                }
            }

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

    fn count_of(w: &World, ty: u8) -> usize {
        (0..w.grid.len).filter(|&i| w.grid.types[i] == ty).count()
    }

    #[test]
    fn square_brush_paints_a_square() {
        let mut w = small();
        w.fill_grid(ct::EMPTY, false);
        apply_brush(&mut w, 10, 10, ct::WALL, 3, BrushShape::Square);
        for r in 9..=11 {
            for c in 9..=11 {
                assert_eq!(w.type_at(c, r), ct::WALL, "({},{}) 应该被画成墙", c, r);
            }
        }
        assert_eq!(w.type_at(12, 10), ct::EMPTY, "方形笔刷不该溢出到 (12,10)");
    }

    #[test]
    fn circle_brush_rounds_the_corners() {
        let mut w = small();
        w.fill_grid(ct::EMPTY, false);
        apply_brush(&mut w, 10, 10, ct::WALL, 5, BrushShape::Circle);
        assert_eq!(w.type_at(10, 10), ct::WALL, "圆心必须有");
        assert_eq!(w.type_at(8, 8), ct::EMPTY, "圆形的角落应该是空的");
        assert_eq!(w.type_at(10, 8), ct::WALL, "正上方仍在圆内");
        assert_eq!(w.type_at(12, 10), ct::WALL, "正右方仍在圆内");
    }

    #[test]
    fn spray_paints_fewer_cells_than_square() {
        let mut w = small();
        w.fill_grid(ct::EMPTY, false);
        apply_brush(&mut w, 10, 10, ct::WALL, 5, BrushShape::Square);
        let square = count_of(&w, ct::WALL);

        let mut w2 = small();
        w2.fill_grid(ct::EMPTY, false);
        apply_brush(&mut w2, 10, 10, ct::WALL, 5, BrushShape::Spray);
        let spray = count_of(&w2, ct::WALL);

        assert!(spray > 0, "喷枪至少要落几笔");
        assert!(
            spray < square,
            "喷枪应当比实心方形稀疏（{} vs {}）",
            spray,
            square
        );
    }

    #[test]
    fn spray_does_not_touch_simulation_rng() {
        let mut a = small();
        a.fill_grid(ct::EMPTY, false);
        let before = a.rng.state();

        let mut b = small();
        b.fill_grid(ct::EMPTY, false);
        apply_brush(&mut b, 10, 10, ct::WALL, 5, BrushShape::Spray);

        assert_eq!(before, b.rng.state(), "画笔不该改变 RNG 状态");
    }

    #[test]
    fn eraser_removes_terrain_but_keeps_food() {
        let mut w = small();
        w.fill_grid(ct::EMPTY, false);
        w.set_cell(5, 5, ct::WALL, -1);
        w.set_cell(6, 5, ct::FOOD, -1);
        apply_brush(&mut w, 5, 5, ERASER, 3, BrushShape::Square);
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

        apply_brush(&mut w, 16, 16, ct::WALL, 5, BrushShape::Square);

        assert_eq!(w.type_at(16, 16), before, "不该把活着的身体涂成墙");
        assert_eq!(w.organisms.len(), 1);
    }
}
