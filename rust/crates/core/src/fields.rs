//! 每格环境场：温度 / 辐射（M4）。
//!
//! ## 为什么是"每格一个 u8"而不是粗粒度场
//!
//! 1.x 时代（浏览器 / JS）我评估过：30k 格全分辨率扩散 × 每秒上千 tick 撑不住，
//! 所以当时打算退到 4×4 或 8×8 的粗场（The Powder Toy 也是这么做的，它的
//! `Air.h` 里 `CELL = 4`）。
//!
//! 换成 Rust 之后这笔账变了：30k 格的一次 5 点扩散是**几十微秒**，
//! 而且这里把场的更新**与 tick 解耦**（按墙钟 60 Hz 推进，不随倍速），
//! 所以全分辨率不但可行，还比粗场更好用——悬停读到的就是光标那一格的准确值，
//! 图层滤镜也是逐像素的，没有块状梯度。
//!
//! ## 为什么更新不跟着 tick 走
//!
//! 50× 倍速下 tick 是 3000/s，如果场也跟着跑 3000 次扩散，纯粹是烧 CPU；
//! 而且"岩浆烤热周围"这件事在 1× 和 50× 下应该**一样快**才对（它是物理时间，
//! 不是模拟步数）。所以主程序按固定 60 Hz 调 `World::update_fields()`。
//!
//! ## 数值约定
//!
//! 值域 0–255，0 = 环境本底。显示时由渲染层的 256 项色表映射成颜色。

use serde::{Deserialize, Serialize};

use crate::cell_type::ct;
use crate::grid::Grid;

/// 一次扩散里"自己"占的权重（邻域各 1，共 6）
const SELF_W: u32 = 2;
const TOTAL_W: u32 = 6;

#[derive(Clone, Serialize, Deserialize)]
pub struct Fields {
    pub len: usize,
    /// 温度场（0–255）
    pub temp: Vec<u8>,
    /// 辐射场（0–255）
    pub rad: Vec<u8>,
    scratch_t: Vec<u8>,
    scratch_r: Vec<u8>,
    /// 每帧向本底衰减多少（0 = 不衰减）
    pub temp_decay: u8,
    pub rad_decay: u8,
}

impl Fields {
    pub fn new(len: usize) -> Self {
        Fields {
            len,
            temp: vec![0; len],
            rad: vec![0; len],
            scratch_t: vec![0; len],
            scratch_r: vec![0; len],
            temp_decay: 1,
            rad_decay: 1,
        }
    }

    pub fn resize(&mut self, len: usize) {
        if self.len == len {
            return;
        }
        self.len = len;
        self.temp = vec![0; len];
        self.rad = vec![0; len];
        self.scratch_t = vec![0; len];
        self.scratch_r = vec![0; len];
    }

    pub fn clear(&mut self) {
        self.temp.fill(0);
        self.rad.fill(0);
    }

    /// 推进一步：源 → 扩散 → 向本底衰减
    ///
    /// 源直接钉在满值（岩浆 = 高温，辐射带 = 强辐射），所以只要地形还在，
    /// 它就会持续把周围烤热 / 照亮，同时自己不断衰减——形成稳定的梯度。
    pub fn update(&mut self, grid: &Grid) {
        if self.len != grid.len {
            self.resize(grid.len);
        }
        let cols = grid.cols;
        let rows = grid.rows;
        let floor = &grid.floor;
        let wrap = grid.wrap;
        let decay = self.temp_decay;

        pass(
            &self.temp,
            &mut self.scratch_t,
            floor,
            cols,
            rows,
            wrap,
            ct::LAVA,
            decay,
        );
        std::mem::swap(&mut self.temp, &mut self.scratch_t);

        let decay = self.rad_decay;
        pass(
            &self.rad,
            &mut self.scratch_r,
            floor,
            cols,
            rows,
            wrap,
            ct::RAD,
            decay,
        );
        std::mem::swap(&mut self.rad, &mut self.scratch_r);
    }

    pub fn temp_at(&self, grid: &Grid, col: i32, row: i32) -> u8 {
        grid.index(col, row).map(|i| self.temp[i]).unwrap_or(0)
    }

    pub fn rad_at(&self, grid: &Grid, col: i32, row: i32) -> u8 {
        grid.index(col, row).map(|i| self.rad[i]).unwrap_or(0)
    }
}

#[allow(clippy::too_many_arguments)]
fn pass(
    cur: &[u8],
    next: &mut [u8],
    floor: &[u8],
    cols: usize,
    rows: usize,
    wrap: bool,
    source: u8,
    decay: u8,
) {
    for r in 0..rows {
        for c in 0..cols {
            let i = r * cols + c;

            // 源格：直接钉满
            if floor[i] == source {
                next[i] = 255;
                continue;
            }

            let v = cur[i] as u32;
            let mut sum = v * SELF_W;

            let up = if r == 0 {
                if wrap { (rows - 1) * cols + c } else { i }
            } else {
                i - cols
            };
            let dn = if r + 1 == rows {
                if wrap { c } else { i }
            } else {
                i + cols
            };
            let lf = if c == 0 {
                if wrap { i + cols - 1 } else { i }
            } else {
                i - 1
            };
            let rt = if c + 1 == cols {
                // 写成 `i + 1 - cols` 而不是 `i - cols + 1`：后者在**第一行最后一格**
                // 会先算 `i - cols`（i = cols-1 < cols）再 +1，debug 下直接
                // "attempt to subtract with overflow" panic。release 下因为回绕
                // 恰好得到 0 才没暴露——歪打正着，别依赖它。
                if wrap { i + 1 - cols } else { i }
            } else {
                i + 1
            };

            sum += cur[up] as u32 + cur[dn] as u32 + cur[lf] as u32 + cur[rt] as u32;

            let mut nv = (sum / TOTAL_W) as u8;
            if decay > 0 {
                nv = nv.saturating_sub(decay);
            }
            next[i] = nv;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::rng::Rng;
    use crate::world::World;

    fn small() -> World {
        let mut cfg = Config::default();
        cfg.world.cols = 16;
        cfg.world.rows = 16;
        World::new(cfg, Rng::new(1))
    }

    #[test]
    fn lava_heats_itself_and_neighbors() {
        let mut w = small();
        w.fill_grid(ct::EMPTY, false);
        w.set_cell(8, 8, ct::LAVA, -1);

        for _ in 0..30 {
            w.update_fields();
        }

        assert_eq!(w.fields.temp_at(&w.grid, 8, 8), 255, "岩浆自己应该是满温");
        let near = w.fields.temp_at(&w.grid, 9, 8);
        assert!(near > 0, "岩浆旁边应该被烤热，实测 {}", near);
        let far = w.fields.temp_at(&w.grid, 15, 15);
        assert!(far < near, "远处应该比近处凉（远 {}, 近 {}）", far, near);
    }

    #[test]
    fn temperature_cools_down_after_lava_is_removed() {
        let mut w = small();
        w.fill_grid(ct::EMPTY, false);
        w.set_cell(8, 8, ct::LAVA, -1);
        for _ in 0..30 {
            w.update_fields();
        }
        assert!(w.fields.temp_at(&w.grid, 8, 8) > 0);

        // 橡皮擦式清除：连地板一起清掉
        w.set_cell(8, 8, ct::EMPTY, -1);
        for _ in 0..400 {
            w.update_fields();
        }
        assert_eq!(w.fields.temp_at(&w.grid, 8, 8), 0, "岩浆没了应该凉回去");
    }

    #[test]
    fn radiation_is_independent_from_temperature() {
        let mut w = small();
        w.fill_grid(ct::EMPTY, false);
        w.set_cell(4, 4, ct::RAD, -1);

        for _ in 0..20 {
            w.update_fields();
        }

        assert!(w.fields.rad_at(&w.grid, 4, 4) > 0, "辐射带应该产生辐射");
        assert_eq!(w.fields.temp_at(&w.grid, 4, 4), 0, "辐射带不该发热");
        assert_eq!(w.fields.rad_at(&w.grid, 12, 12), 0, "远处不该有辐射");
    }

    #[test]
    fn fields_survive_grid_resize() {
        let mut w = small();
        w.set_cell(8, 8, ct::LAVA, -1);
        for _ in 0..10 {
            w.update_fields();
        }
        w.resize(24, 24);
        assert_eq!(w.fields.len, 24 * 24);
        assert_eq!(w.fields.temp_at(&w.grid, 8, 8), 0, "重建世界后场应清零");
    }

    /// 性能预算：200×150 两张场一次 update 应该远低于 1 ms
    /// （60 Hz 下每帧最多跑 4 次，必须留足余量给模拟本身）
    #[test]
    fn field_update_is_cheap() {
        let mut w = World::new(Config::default(), Rng::new(1));
        for (c, r) in [(10, 10), (100, 80), (180, 140)] {
            w.set_cell(c, r, ct::LAVA, -1);
            w.set_cell(c + 2, r + 2, ct::RAD, -1);
        }
        // 预热
        for _ in 0..10 {
            w.update_fields();
        }
        let t0 = std::time::Instant::now();
        let n = 100;
        for _ in 0..n {
            w.update_fields();
        }
        let per = t0.elapsed().as_secs_f64() * 1000.0 / n as f64;
        println!("环境场 update: {:.3} ms/次（{}×{}）", per, w.grid.cols, w.grid.rows);
        assert!(per < 2.0, "环境场太慢：{:.3} ms/次", per);
    }
}
