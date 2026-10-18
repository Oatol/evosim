//! 光场：从世界顶部垂直投射，被墙遮挡，随深度衰减（批次 7）。
//!
//! ## 为什么和温度/辐射分开
//!
//! `fields.rs` 里那两个是**各向同性扩散**（5 点模板 + 每帧衰减），光不是：
//!
//! | | 温度 / 辐射 | 光 |
//! |---|---|---|
//! | 传播 | 各向同性扩散 | **方向性投射**（从上方来，逐行往下传） |
//! | 更新 | 每帧（物理时间 60 Hz） | 只在拓扑变化时（地形纯函数） |
//! | 衰减 | `saturating_sub` | 随深度衰减，**遇墙归零** |
//! | 缓冲 | 需要 scratch 双缓冲 | 单次扫描，原地写 |
//!
//! 塞进 `Fields::update()` 会让那个函数变成三种算法轮流跑的大杂烩，
//! 而且光场本来可以不每帧算，混进去就被迫跟着算。
//!
//! ## 这个场存在的意义
//!
//! 加光场之前，模拟里没有**空间梯度**：食物由生产者细胞在自己旁边凭空产出，
//! 所以「去哪里」不是决策，「身体朝哪边」没有意义，「移动」只是逃命。
//! 光从上方来、被墙挡住、随深度变暗之后：
//!
//! * 位置有价值了 → 往亮处走
//! * 身体朝向有价值了 → 生产者该长在朝光的一侧
//! * 阴影有价值了 → 墙下面能躲、能埋伏
//!
//! ## 生物不挡光
//!
//! 只有实心地形（墙）挡光。生物透光，理由有三：
//!
//! 1. 否则 `level` 依赖每 tick 都在变的 `types`，光场必须每 tick 重算；
//! 2. 否则大身体会用自己的上半身遮住自己的下半身，体型越大越吃亏——
//!    正好和「让体型有意义」这个目标相反；
//! 3. 物理上也说得通，生物是薄薄一层组织。
//!
//! ## 数值约定
//!
//! 值域 0–255，255 = 满照（世界顶行），0 = 全黑。渲染层用 256 项色表映射。

use serde::{Deserialize, Serialize};

use crate::cell_type::{self, ct};
use crate::grid::Grid;

/// 顶行（地表）的满照值
pub const SUN_MAX: u8 = 255;

#[derive(Clone, Serialize, Deserialize)]
pub struct Light {
    pub len: usize,
    /// 每格光强（0–255）
    pub level: Vec<u8>,
    /// 每往下一格衰减多少，由世界高度推导（见 `absorb_for`）
    pub absorb: u8,
}

impl Light {
    pub fn new(cols: usize, rows: usize) -> Self {
        Light {
            len: cols * rows,
            level: vec![0; cols * rows],
            absorb: absorb_for(rows),
        }
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        if self.len == cols * rows {
            return;
        }
        self.len = cols * rows;
        self.level = vec![0; self.len];
        self.absorb = absorb_for(rows);
    }

    pub fn clear(&mut self) {
        self.level.fill(0);
    }

    /// 重算整张光场。`O(cols × rows)`，30k 格约 30–50 µs。
    ///
    /// 每列独立：从顶行往下扫，遇墙归零，其余每格减去 `absorb`。
    /// 光只垂直走，所以**不处理水平环绕**——列之间互不影响。
    pub fn rebuild(&mut self, grid: &Grid) {
        if self.len != grid.len {
            self.resize(grid.cols, grid.rows);
        }
        let cols = grid.cols;
        let rows = grid.rows;
        let absorb = self.absorb;
        for c in 0..cols {
            let mut v: u8 = SUN_MAX;
            for r in 0..rows {
                let i = r * cols + c;
                if blocks_light(grid.floor[i]) {
                    // 墙：这一格及以下全黑（硬阴影）
                    v = 0;
                } else if r > 0 {
                    v = v.saturating_sub(absorb);
                }
                self.level[i] = v;
            }
        }
    }

    /// 取某格光强（越界返回 0）
    #[inline]
    pub fn at(&self, grid: &Grid, col: i32, row: i32) -> u8 {
        grid.index(col, row).map(|i| self.level[i]).unwrap_or(0)
    }

    /// 取某格光强的归一化值（0.0–1.0），光合用这个
    #[inline]
    pub fn lit_at(&self, grid: &Grid, col: i32, row: i32) -> f64 {
        self.at(grid, col, row) as f64 / SUN_MAX as f64
    }
}

/// 每往下一格衰减多少。
///
/// 目标是「世界高度的 80% 处恰好归零」：保证底部是暗的（有地方可以躲），
/// 同时上部有足够的梯度让生物分层。150 行 → `ceil(255/120) = 3`，第 85 行归零。
fn absorb_for(rows: usize) -> u8 {
    let span = (rows as f64 * 0.8).max(1.0);
    ((SUN_MAX as f64 / span).ceil() as u8).max(1)
}

/// 这种地形挡不挡光。
///
/// **只有实心地形挡**：`terrain` 且不可通行，目前就是墙。
/// 岩浆 / 沼泽 / 辐射带 / 腐殖质都是"铺在地上的东西"，光能照到；
/// 空和食物也不挡。
#[inline]
pub fn blocks_light(ty: u8) -> bool {
    ty != ct::EMPTY && cell_type::is_terrain(ty) && !cell_type::is_passable(ty)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_grid(cols: usize, rows: usize) -> Grid {
        Grid::new(cols, rows, false)
    }

    #[test]
    fn surface_is_full_bright() {
        let g = empty_grid(8, 20);
        let mut l = Light::new(8, 20);
        l.rebuild(&g);
        for c in 0..8 {
            assert_eq!(l.at(&g, c as i32, 0), SUN_MAX, "顶行第 {} 列应该是满照", c);
        }
    }

    #[test]
    fn light_decreases_with_depth() {
        // 空世界：每列都是从满照开始，每行减 absorb，减到 0 为止
        let g = empty_grid(4, 30);
        let mut l = Light::new(4, 30);
        l.rebuild(&g);
        let absorb = l.absorb as i32;
        for r in 0..30i32 {
            let expect = (SUN_MAX as i32 - r * absorb).max(0) as u8;
            assert_eq!(
                l.at(&g, 0, r),
                expect,
                "第 {} 行光强应为 {}（absorb = {}）",
                r,
                expect,
                absorb
            );
        }
        assert_eq!(l.at(&g, 0, 29), 0, "底部应该全黑");
    }

    #[test]
    fn wall_casts_shadow() {
        let mut g = empty_grid(9, 20);
        // 在第 5 列、第 3 行放一堵墙
        g.floor[3 * 9 + 5] = ct::WALL;
        let mut l = Light::new(9, 20);
        l.rebuild(&g);

        assert_eq!(l.at(&g, 5, 3), 0, "墙自己那格应该全黑");
        for r in 3..20 {
            assert_eq!(l.at(&g, 5, r), 0, "墙正下方第 {} 行应该在阴影里", r);
        }
        // 同高度的邻居不受影响
        assert!(l.at(&g, 4, 5) > 0, "墙左边的同高度格子不该被遮");
        assert!(l.at(&g, 6, 5) > 0, "墙右边的同高度格子不该被遮");
    }

    #[test]
    fn light_resumes_below_a_floating_wall_only_after_the_shadow_ends() {
        // 硬阴影：墙下方的整列都是黑的（光不会绕过去）。
        // 这条是"故意的"——软阴影（散射光）留给以后。
        let mut g = empty_grid(3, 30);
        g.floor[5 * 3 + 1] = ct::WALL;
        let mut l = Light::new(3, 30);
        l.rebuild(&g);
        assert_eq!(l.at(&g, 1, 6), 0);
        assert_eq!(l.at(&g, 1, 20), 0);
    }

    #[test]
    fn passable_terrain_does_not_block() {
        // 岩浆 / 沼泽 / 辐射带 / 腐殖质都铺在地上，光能照到
        for ty in [ct::LAVA, ct::SWAMP, ct::RAD, ct::HUMUS, ct::FOOD] {
            let mut g = empty_grid(3, 20);
            g.floor[2 * 3 + 1] = ty;
            let mut l = Light::new(3, 20);
            l.rebuild(&g);
            assert!(
                l.at(&g, 1, 2) > 0,
                "{} 不该挡光（它铺在地上，光能照到）",
                cell_type::label(ty)
            );
        }
    }

    #[test]
    fn light_is_independent_of_organisms() {
        // 生物不挡光：往 `types` 里塞一堆身体细胞，光场必须一模一样。
        // 这是「光场是地形纯函数」的守门测试——一旦有人改成读 `types`，这里会炸。
        let mut g = empty_grid(6, 20);
        g.floor[4 * 6 + 2] = ct::WALL;
        let mut l = Light::new(6, 20);
        l.rebuild(&g);
        let before = l.level.clone();

        // 模拟一堆生物占位
        for i in 0..g.len {
            if g.floor[i] == ct::EMPTY {
                g.types[i] = ct::PRODUCER;
            }
        }
        l.rebuild(&g);
        assert_eq!(l.level, before, "生物占位不该影响光场");
    }

    #[test]
    fn absorb_covers_eighty_percent_of_height() {
        // 150 行的世界，光应该在 85 行左右归零
        let g = empty_grid(2, 150);
        let mut l = Light::new(2, 150);
        l.rebuild(&g);
        assert_eq!(l.absorb, 3, "150 行的 absorb 应该是 3");
        assert!(l.at(&g, 0, 84) > 0, "第 84 行还有光");
        assert_eq!(l.at(&g, 0, 85), 0, "第 85 行归零");
    }

    #[test]
    fn resize_clears_and_recomputes_absorb() {
        let mut l = Light::new(4, 10);
        l.level.fill(200);
        l.resize(4, 10);
        assert_eq!(l.level[0], 200, "尺寸没变时不该清空");
        l.resize(6, 30);
        assert_eq!(l.len, 180);
        assert!(l.level.iter().all(|&v| v == 0), "换尺寸后应该清零");
        assert_eq!(l.absorb, absorb_for(30));
    }
}
