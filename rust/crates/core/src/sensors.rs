//! 感知编码 —— 世界 → 40 维输入向量。与 JS 版 `src/life/Sensors.js` 对应。
//!
//! [ 0..35]  视觉：4 个相对方向 × 9 维（8 类型 one-hot + 1 距离）
//! [36]      能量比例
//! [37]      年龄比例
//! [38]      会移动
//! [39]      偏置 1

use crate::anatomy::{rotate_col, rotate_row, BodyCell};
use crate::cell_type::{self, ct, DIR_VECTORS, VISION_SLOT, VISIBLE_CELL_TYPES};
use crate::config::AttrField;
use crate::config::Config;
use crate::grid::Grid;

pub const VISION_DIRS: usize = 4;
pub const VISUAL_SLOTS: usize = 9;
pub const VISION_DIM: usize = VISION_DIRS * VISUAL_SLOTS; // 36
pub const STATE_BASE: usize = VISION_DIM;
pub const SENSE_DIM: usize = 40;

/// 感知所需的"个体视图"—— 把 Organism 的字段拆出来，避免借用冲突
pub struct SenseView<'a> {
    pub cells: &'a [BodyCell],
    pub rotation: u8,
    pub col: i32,
    pub row: i32,
    pub id: u32,
    pub energy_ratio: f64,
    pub age_ratio: f64,
    pub has_mover: bool,
}

/// 填充输入向量。就地改写 `out`（长度 = SENSE_DIM）。
pub fn sense(v: &SenseView, grid: &Grid, cfg: &Config, out: &mut [f32]) {
    out.fill(0.0);

    let global_range = cfg.sense.look_range;
    let see_through_self = cfg.sense.see_through_self;

    for cell in v.cells {
        if !cell_type::is_directional(cell.type_id) {
            continue;
        }
        let mut range = cfg.attr(cell.type_id, AttrField::SenseRange) as i32;
        if range == 0 {
            range = global_range;
        }
        if range <= 0 {
            continue;
        }

        let local_dir = (cell.dir & 3) as usize;
        let base = local_dir * VISUAL_SLOTS;
        if base + VISUAL_SLOTS > out.len() {
            continue;
        }

        let dir_vec = DIR_VECTORS[(v.rotation as usize + local_dir) & 3];
        let mut col = v.col + rotate_col(cell.dx, cell.dy, v.rotation);
        let mut row = v.row + rotate_row(cell.dx, cell.dy, v.rotation);

        let mut seen_type: i32 = -1;
        let mut seen_dist: i32 = 0;

        for step in 1..=range {
            col += dir_vec.0;
            row += dir_vec.1;
            let index = match grid.index(col, row) {
                Some(i) => i,
                None => break, // 越界（非环绕世界）
            };

            let ty = grid.types[index];
            if ty == ct::EMPTY {
                continue; // 空气，继续看
            }
            if grid.owners[index] == v.id as i32 {
                if see_through_self {
                    continue; // 透视自身
                }
                break; // 被自己挡住
            }
            seen_type = ty as i32;
            seen_dist = step;
            break;
        }

        if seen_type < 0 {
            continue;
        }
        let slot = if (seen_type as usize) < VISION_SLOT.len() {
            VISION_SLOT[seen_type as usize]
        } else {
            -1
        };
        if slot < 0 {
            continue; // 该类型不参与视觉
        }

        // 同一方向多只眼睛：取更近的那个
        let closeness = 1.0 - seen_dist as f64 / range as f64;
        if closeness > out[base + VISUAL_SLOTS - 1] as f64 {
            for t in 0..VISIBLE_CELL_TYPES.len() {
                out[base + t] = 0.0;
            }
            out[base + slot as usize] = 1.0;
            out[base + VISUAL_SLOTS - 1] = closeness as f32;
        }
    }

    if out.len() > STATE_BASE {
        out[STATE_BASE] = v.energy_ratio as f32;
    }
    if out.len() > STATE_BASE + 1 {
        out[STATE_BASE + 1] = v.age_ratio as f32;
    }
    if out.len() > STATE_BASE + 2 {
        out[STATE_BASE + 2] = if v.has_mover { 1.0 } else { 0.0 };
    }
    if out.len() > STATE_BASE + 3 {
        out[STATE_BASE + 3] = 1.0;
    }
}

/// 40 个输入槽的可读名（检视器用）
pub fn input_labels() -> Vec<String> {
    const DIRS: [&str; 4] = ["前", "右", "后", "左"];
    let mut labels = Vec::with_capacity(SENSE_DIM);
    for d in 0..VISION_DIRS {
        for t in VISIBLE_CELL_TYPES.iter() {
            labels.push(format!("{}·{}", DIRS[d], cell_type::label(*t)));
        }
        labels.push(format!("{}·距离", DIRS[d]));
    }
    labels.push("能量".into());
    labels.push("年龄".into());
    labels.push("会移动".into());
    labels.push("偏置".into());
    labels
}
