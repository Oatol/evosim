//! 生物工厂：造出可用的起始生物。与 JS 版 `src/life/OrganismFactory.js` 对应。

use crate::cell_type::{ct, LIVING_CELL_TYPES};
use crate::organism::Organism;
use crate::rng::Rng;
use crate::world::World;

const NEIGHBORS_4: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];

/// 生命起源：中心一个嘴，两个对角生产者
pub fn create_origin_organism(world: &mut World, col: i32, row: i32) -> Organism {
    let mut org = Organism::new(&world.config, &mut world.rng, &world.grid, col, row);
    org.anatomy.add_cell(ct::MOUTH, 0, 0, 0);
    org.anatomy.add_cell(ct::PRODUCER, 1, 1, 0);
    org.anatomy.add_cell(ct::PRODUCER, -1, -1, 0);
    org
}

/// 随机小生物：从中心向邻域随机生长 2-6 个细胞
pub fn create_random_organism(world: &mut World, col: i32, row: i32) -> Organism {
    let mut org = Organism::new(&world.config, &mut world.rng, &world.grid, col, row);
    org.rotation = world.rng.int(4) as u8;
    org.direction = world.rng.int(4) as u8;

    let center_type = if world.rng.chance(50.0) {
        ct::MOUTH
    } else {
        ct::PRODUCER
    };
    org.anatomy.add_cell(center_type, 0, 0, 0);

    let growth = world.rng.int_range(2, 6);
    for _ in 0..growth {
        let base_idx = match org.anatomy.random_cell_index(&mut world.rng) {
            Some(i) => i,
            None => break,
        };
        let (bx, by) = (org.anatomy.cells[base_idx].dx, org.anatomy.cells[base_idx].dy);
        let n = NEIGHBORS_4[world.rng.int(4)];
        let dx = bx + n.0;
        let dy = by + n.1;
        if dx.abs() > 3 || dy.abs() > 3 {
            continue;
        }
        if org.anatomy.cell_at(dx, dy).is_some() {
            continue;
        }
        let type_id = LIVING_CELL_TYPES[world.rng.int(LIVING_CELL_TYPES.len())];
        let dir = if type_id == ct::EYE {
            world.rng.int(4) as u8
        } else {
            0
        };
        org.anatomy.add_cell(type_id, dx, dy, dir);
    }
    org
}

/// 找一个能放下给定半径生物的空位
pub fn find_clear_spot(world: &mut World, radius: i32, attempts: usize) -> Option<(i32, i32)> {
    for _ in 0..attempts {
        let col = world.rng.int(world.grid.cols) as i32;
        let row = world.rng.int(world.grid.rows) as i32;
        if is_area_empty(world, col, row, radius) {
            return Some((col, row));
        }
    }
    None
}

/// 以 (col,row) 为中心的方形区域是否全空
pub fn is_area_empty(world: &World, col: i32, row: i32, radius: i32) -> bool {
    for dr in -radius..=radius {
        for dc in -radius..=radius {
            match world.grid.index(col + dc, row + dr) {
                Some(index) => {
                    if world.grid.types[index] != ct::EMPTY {
                        return false;
                    }
                }
                None => return false,
            }
        }
    }
    true
}

/// 在世界中央投放生命起源
pub fn seed_origin_of_life(world: &mut World) -> u32 {
    let col = (world.grid.cols / 2) as i32;
    let row = (world.grid.rows / 4) as i32;

    // 先把中心一小块清空
    for dr in -3..=3 {
        for dc in -3..=3 {
            if let Some(index) = world.grid.index(col + dc, row + dr) {
                world.set_cell_at(index, ct::EMPTY, -1);
            }
        }
    }

    let org = create_origin_organism(world, col, row);
    world.add_organism(org)
}

/// 在指定位置投放一个随机生物
pub fn spawn_random_organism(world: &mut World, col: i32, row: i32) -> Option<u32> {
    let org = create_random_organism(world, col, row);
    if !org.is_clear(&world.grid, &world.config, col, row, org.rotation) {
        return None;
    }
    Some(world.add_organism(org))
}

/// 撒一片随机生物，返回成功投放的数量
pub fn spawn_population(world: &mut World, count: usize, radius: i32) -> usize {
    let mut spawned = 0;
    for _ in 0..count {
        let spot = match find_clear_spot(world, radius, 40) {
            Some(s) => s,
            None => break,
        };
        if spawn_random_organism(world, spot.0, spot.1).is_some() {
            spawned += 1;
        }
    }
    spawned
}

/// 随机游走用的空 RNG（占位，避免未使用告警）
pub fn _unused(_rng: &mut Rng) {}
