//! 光场：**可放置的点光源** + 天空光，多源扩散，被墙遮挡（批次 7.5）。
//!
//! ## 为什么从「垂直投射」改成「多源扩散」
//!
//! 第一版是每列从顶行往下扫、遇墙归零。它有个致命的设计后果：**世界被强行分成上下**。
//! 上半亮、下半黑，生物只能活在上半部，玩家想让光出现在地底或某个洞里完全做不到。
//! 那不是「光」，那是「地表」。
//!
//! 现在改成：
//!
//! * **点光源** —— 独立的一层数据（`sources`），**不占物质世界的格子**，
//!   生物可以穿过它，画笔也擦不到它。玩家想在哪放就在哪放。
//! * **天空光** —— 退化成「顶部一排光源」，可以整个关掉（`sky = 0`）。
//!   关掉之后世界全黑，只剩玩家放的那些光。
//! * 两者统一用**多源 BFS 扩散**计算，遇墙不传播。
//!
//! 顺带白捡一个好处：扩散会让光**绕进洞口**，墙的阴影边缘是软的（不再是硬切）。
//!
//! ## 算法：桶排序多源 BFS
//!
//! 把所有光源按强度丢进 256 个桶，从 255 往 1 扫。每个格子只在「被更亮的覆盖」时
//! 才入桶，所以每个格子最多被处理一次 —— 复杂度 `O(面积)`，与光源数量几乎无关。
//!
//! ```text
//! for v in (1..=255).rev():
//!     for i in buckets[v]:            // 这一格的最亮到达值就是 v
//!         if level[i] != v: continue  // 已被更亮的覆盖，跳过
//!         for 四邻 n:
//!             if 墙(n): continue
//!             nv = v - absorb
//!             if nv > level[n]: level[n] = nv; buckets[nv].push(n)
//! ```
//!
//! ## 数值约定
//!
//! 值域 0–255，255 = 满照，0 = 全黑。渲染层用 256 项色表映射。

use serde::{Deserialize, Serialize};

use crate::cell_type::{self, ct};
use crate::grid::Grid;

/// 满照
pub const SUN_MAX: u8 = 255;

/// 一个点光源。**独立于物质世界**：不占格子、不挡路、画笔擦不到。
///
/// 放在墙上也允许 —— 光从那一格发出，能照到墙外的空地（墙只挡"穿过"它的光）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LightSource {
    pub col: i32,
    pub row: i32,
    /// 0–255。决定它能照多远：`strength / absorb` 格。
    pub strength: u8,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Light {
    pub len: usize,
    /// 每格光强（0–255）= `sky_level + lamp_level`（饱和相加）。
    ///
    /// 光合、渲染、悬停读数都读这一层。**它只是两个分量的和**，
    /// 真正需要分量的地方是遮挡：天空光会被身体挡住，点光源不会
    /// （方向不同，见 `World::shade_factor`）。
    pub level: Vec<u8>,
    /// 天空光分量。被生物身体**垂直遮挡**的就是它。
    pub sky_level: Vec<u8>,
    /// 点光源分量。不被身体遮挡（近似：灯可以从侧面照进来）。
    pub lamp_level: Vec<u8>,
    /// 玩家放置的点光源
    pub sources: Vec<LightSource>,
    /// 每格衰减多少，由世界高度推导（见 `absorb_for`）
    pub absorb: u8,
    /// 天空光强度（0 = 关掉，世界只剩点光源）
    pub sky: u8,
}

impl Default for Light {
    fn default() -> Self {
        Light {
            len: 0,
            level: Vec::new(),
            sky_level: Vec::new(),
            lamp_level: Vec::new(),
            sources: Vec::new(),
            absorb: 3,
            sky: SUN_MAX,
        }
    }
}

impl Light {
    pub fn new(cols: usize, rows: usize) -> Self {
        let mut l = Light::default();
        l.len = cols * rows;
        l.level = vec![0; l.len];
        l.sky_level = vec![0; l.len];
        l.lamp_level = vec![0; l.len];
        l.absorb = absorb_for(rows);
        l
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        if self.len == cols * rows {
            return;
        }
        self.len = cols * rows;
        self.level = vec![0; self.len];
        self.sky_level = vec![0; self.len];
        self.lamp_level = vec![0; self.len];
        self.absorb = absorb_for(rows);
        // 越界的光源丢掉：世界变小了，它们本来也照不到任何格子
        let (c, r) = (cols as i32, rows as i32);
        self.sources.retain(|s| s.col >= 0 && s.row >= 0 && s.col < c && s.row < r);
    }

    pub fn clear(&mut self) {
        self.level.fill(0);
        self.sky_level.fill(0);
        self.lamp_level.fill(0);
    }

    /// 重算整张光场：天空光和点光源**各跑一次 BFS**，然后相加。
    ///
    /// 为什么要分开：遮挡只该作用于天空光——生物的身体挡得住"从头顶下来"的光，
    /// 挡不住"从侧面照进来"的灯。合成一个场就没法区分了。
    ///
    /// 两次 BFS 仍是 `O(面积)`，30k 格约 200–400 µs；靠 `World.light_dirty`
    /// 只在必要时重算，所以这点开销落在"改地形/放灯"那一下，不在每帧。
    pub fn rebuild(&mut self, grid: &Grid) {
        if self.len != grid.len {
            self.resize(grid.cols, grid.rows);
        }

        // ── 天空光：顶部一排光源 ──
        //
        // 顶部是墙的那一列不发光——否则光会从墙里冒出来，看着像 bug。
        let mut sky_seeds: Vec<(usize, u8)> = Vec::new();
        if self.sky > 0 {
            for c in 0..grid.cols {
                if blocks_light(grid.floor[c]) {
                    continue;
                }
                sky_seeds.push((c, self.sky));
            }
        }
        self.sky_level = self.propagate(grid, &sky_seeds);

        // ── 点光源 ──
        let mut lamp_seeds: Vec<(usize, u8)> = Vec::new();
        for s in &self.sources {
            if s.strength == 0 {
                continue;
            }
            let Some(i) = grid.index(s.col, s.row) else {
                continue;
            };
            lamp_seeds.push((i, s.strength));
        }
        self.lamp_level = self.propagate(grid, &lamp_seeds);

        for i in 0..self.len {
            self.level[i] = self.sky_level[i].saturating_add(self.lamp_level[i]);
        }
    }

    /// 桶排序多源 BFS，返回一张场。
    ///
    /// 每个格子只在「被更亮的覆盖」时入桶，所以每格最多处理一次。
    fn propagate(&self, grid: &Grid, seeds: &[(usize, u8)]) -> Vec<u8> {
        let mut level = vec![0u8; self.len];
        let mut buckets: Vec<Vec<u32>> = (0..256).map(|_| Vec::new()).collect();

        for &(i, v) in seeds {
            if i < level.len() && v > level[i] {
                level[i] = v;
                buckets[v as usize].push(i as u32);
            }
        }

        let cols = grid.cols;
        let rows = grid.rows as i32;
        let absorb = self.absorb;
        for v in (1..=255u8).rev() {
            let mut cur = std::mem::take(&mut buckets[v as usize]);
            for &ii in cur.iter() {
                let i = ii as usize;
                if level[i] != v {
                    continue; // 已被更亮的覆盖
                }
                let c = (i % cols) as i32;
                let r = (i / cols) as i32;
                for (dc, dr) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                    // 注意：**垂直方向永不环绕**，水平方向按 `grid.wrap` 走。
                    //
                    // 这一条踩过：`wrap_edges = true` 时如果垂直也环绕，天空光会从
                    // 第 0 行的"上邻居"= 第 149 行漏回来，于是世界变成「上下两头亮、
                    // 中间黑」（实测剖面 r0=255 / r75=30 / r149=252）。
                    // 高度方向代表"地下"，上下本来就不该相连；左右才是环绕的柱面世界。
                    let nr = r + dr;
                    if nr < 0 || nr >= rows {
                        continue;
                    }
                    let Some(n) = grid.index(c + dc, nr) else {
                        continue;
                    };
                    if blocks_light(grid.floor[n]) {
                        continue;
                    }
                    let nv = v.saturating_sub(absorb);
                    if nv > level[n] {
                        level[n] = nv;
                        buckets[nv as usize].push(n as u32);
                    }
                }
            }
            cur.clear();
            buckets[v as usize] = cur; // 放回去复用
        }
        level
    }

    /// 取某格光强（越界返回 0）
    #[inline]
    pub fn at(&self, grid: &Grid, col: i32, row: i32) -> u8 {
        grid.index(col, row).map(|i| self.level[i]).unwrap_or(0)
    }

    /// 天空光分量（0–255）。**会被生物身体遮挡**的那一半。
    #[inline]
    pub fn sky_at(&self, grid: &Grid, col: i32, row: i32) -> u8 {
        grid.index(col, row)
            .map(|i| self.sky_level[i])
            .unwrap_or(0)
    }

    /// 点光源分量（0–255）。不被身体遮挡。
    #[inline]
    pub fn lamp_at(&self, grid: &Grid, col: i32, row: i32) -> u8 {
        grid.index(col, row)
            .map(|i| self.lamp_level[i])
            .unwrap_or(0)
    }

    /// 取某格光强的归一化值（0.0–1.0），光合用这个
    #[inline]
    pub fn lit_at(&self, grid: &Grid, col: i32, row: i32) -> f64 {
        self.at(grid, col, row) as f64 / SUN_MAX as f64
    }

    /// 这一格有没有点光源（用来做「点一下放置、再点一下移除」）
    pub fn source_index_at(&self, col: i32, row: i32) -> Option<usize> {
        self.sources
            .iter()
            .position(|s| s.col == col && s.row == row)
    }

    /// 放一个光源。同一格已有时直接改强度，不重复加。
    pub fn set_source(&mut self, col: i32, row: i32, strength: u8) {
        match self.source_index_at(col, row) {
            Some(k) => self.sources[k].strength = strength,
            None => self.sources.push(LightSource { col, row, strength }),
        }
    }

    /// 移除某格的光源，返回有没有删掉
    pub fn remove_source_at(&mut self, col: i32, row: i32) -> bool {
        match self.source_index_at(col, row) {
            Some(k) => {
                self.sources.remove(k);
                true
            }
            None => false,
        }
    }
}

/// 每格衰减多少。
///
/// 目标是「世界高度的 80% 处衰减完」：天空光能照到世界大半，但又明显有梯度。
/// 150 行 → `ceil(255/120) = 3`，也就是天空光能照 85 格深；
/// 一个满强度点光源的照射半径也是 85 格（想小一点就调低强度）。
fn absorb_for(rows: usize) -> u8 {
    let span = (rows as f64 * 0.8).max(1.0);
    ((SUN_MAX as f64 / span).ceil() as u8).max(1)
}

/// 这种地形挡不挡光。
///
/// **只有实心地形挡**：`terrain` 且不可通行，目前就是墙。
/// 岩浆 / 沼泽 / 辐射带 / 腐殖质都是"铺在地上的东西"，光能照到；空和食物也不挡。
#[inline]
pub fn blocks_light(ty: u8) -> bool {
    ty != ct::EMPTY && cell_type::is_terrain(ty) && !cell_type::is_passable(ty)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(cols: usize, rows: usize) -> Grid {
        Grid::new(cols, rows, false)
    }

    #[test]
    fn sky_lights_the_surface() {
        let g = grid(8, 30);
        let mut l = Light::new(8, 30);
        l.rebuild(&g);
        for c in 0..8 {
            assert_eq!(l.at(&g, c as i32, 0), SUN_MAX, "顶行第 {} 列应该满照", c);
        }
    }

    #[test]
    fn sky_can_be_turned_off() {
        // 关掉天空光之后，没有点光源的世界必须全黑 —— 这是"世界不再强行分上下"的前提
        let g = grid(8, 30);
        let mut l = Light::new(8, 30);
        l.sky = 0;
        l.rebuild(&g);
        assert!(
            l.level.iter().all(|&v| v == 0),
            "sky = 0 且没有点光源时应该全黑"
        );
    }

    #[test]
    fn a_lamp_lights_its_surroundings_regardless_of_depth() {
        // 这是本次改动的核心：光可以出现在**世界底部**，不再被"上亮下黑"绑死
        let g = grid(40, 40);
        let mut l = Light::new(40, 40);
        l.sky = 0;
        l.set_source(20, 35, 255); // 世界底部
        l.rebuild(&g);

        assert_eq!(l.at(&g, 20, 35), 255, "光源自己那格应该最亮");
        assert!(l.at(&g, 21, 35) > 0, "光源右边应该有光");
        assert!(l.at(&g, 20, 30) > 0, "光源上方应该有光");
        assert!(
            l.at(&g, 20, 34) > l.at(&g, 20, 30),
            "离光源越远越暗（34 行应该比 30 行亮）"
        );
        assert_eq!(l.at(&g, 0, 0), 0, "远处（左上角）应该还是黑的");
    }

    #[test]
    fn wall_blocks_and_light_wraps_around_the_edge() {
        // 一堵竖墙把世界分成两半：墙右边没光源 → 应该基本黑；
        // 但光能绕过墙的**边缘**（扩散不是直线），所以顶端/底端会漏一点。
        let mut g = grid(20, 20);
        for r in 0..18 {
            g.floor[r * 20 + 10] = ct::WALL;
        }
        let mut l = Light::new(20, 20);
        l.sky = 0;
        l.set_source(5, 10, 255);
        l.rebuild(&g);

        assert!(l.at(&g, 5, 10) > 0, "光源处有光");
        assert!(l.at(&g, 9, 10) > 0, "墙左侧还有光");
        let behind = l.at(&g, 12, 10);
        let front = l.at(&g, 8, 10);
        assert!(
            behind < front,
            "墙后面（{}）应该比墙前面（{}）暗得多",
            behind,
            front
        );
    }

    #[test]
    fn stronger_lamp_reaches_farther() {
        // 用 60×60 的世界（absorb = 6），光源放中间，横向 30 格处
        let g = grid(60, 60);
        let reach = |strength: u8| -> u8 {
            let mut l = Light::new(60, 60);
            l.sky = 0;
            l.set_source(0, 30, strength);
            l.rebuild(&g);
            l.at(&g, 30, 30)
        };
        assert!(
            reach(255) > reach(120),
            "强光源照得更远（255 → {}，120 → {}）",
            reach(255),
            reach(120)
        );
        assert!(reach(255) > 0, "满强度光源 30 格外应该还有光");
        assert_eq!(reach(0), 0, "强度 0 的光源不发光");
    }

    #[test]
    fn sources_are_independent_of_the_material_grid() {
        // 光源不占格子：网格全空，光源列表照样工作
        let g = grid(10, 10);
        let mut l = Light::new(10, 10);
        l.sky = 0;
        l.set_source(3, 3, 200);
        l.rebuild(&g);
        assert!(l.at(&g, 3, 3) == 200);
        assert!(
            g.types.iter().all(|&t| t == ct::EMPTY),
            "放光源不该改动物质网格"
        );
    }

    #[test]
    fn set_and_remove_source() {
        let mut l = Light::new(10, 10);
        l.set_source(2, 2, 100);
        assert_eq!(l.sources.len(), 1);
        l.set_source(2, 2, 200); // 同一格改强度，不重复加
        assert_eq!(l.sources.len(), 1);
        assert_eq!(l.sources[0].strength, 200);
        assert!(l.remove_source_at(2, 2));
        assert!(l.sources.is_empty());
        assert!(!l.remove_source_at(2, 2), "再删一次应该返回 false");
    }

    #[test]
    fn resize_drops_out_of_bounds_sources() {
        let mut l = Light::new(20, 20);
        l.set_source(15, 15, 100);
        l.set_source(2, 2, 100);
        l.resize(10, 10);
        assert_eq!(l.sources.len(), 1, "越界的光源应该被丢掉");
        assert_eq!(l.sources[0].col, 2);
    }

    #[test]
    fn light_is_independent_of_organisms() {
        // 生物不挡光：往 `types` 里塞一堆身体细胞，光场必须一模一样
        let mut g = grid(12, 20);
        g.floor[8 * 12 + 4] = ct::WALL;
        let mut l = Light::new(12, 20);
        l.set_source(3, 12, 255);
        l.rebuild(&g);
        let before = l.level.clone();

        for i in 0..g.len {
            if g.floor[i] == ct::EMPTY {
                g.types[i] = ct::PRODUCER;
            }
        }
        l.rebuild(&g);
        assert_eq!(l.level, before, "生物占位不该影响光场");
    }

    /// 天空光和点光源是**两层**，`level` 只是它们的和。
    ///
    /// 分开存是为了遮挡：身体挡得住从头顶下来的光，挡不住从侧面照进来的灯。
    #[test]
    fn sky_and_lamp_are_separate_components() {
        let g = grid(20, 20);
        let mut l = Light::new(20, 20);
        l.sky = 200;
        // 灯放在第 5 行：这一行天空光还活着（200 − 5×absorb），两层能同时非零。
        // 放到最底部就没天空光了——20 行的 absorb 是 16，第 17 行早就归零。
        l.set_source(10, 5, 255);
        l.rebuild(&g);

        // 顶部：天空光满额，点光源够不着
        assert_eq!(l.sky_at(&g, 10, 0), 200, "顶部天空光应该等于设定强度");
        assert_eq!(
            l.lamp_at(&g, 0, 19),
            0,
            "左下角离灯 24 格（reach 15），点光源分量应该衰减到 0"
        );

        // 灯旁边：两层都有
        let near = (10, 5);
        assert!(l.sky_at(&g, near.0, near.1) > 0, "灯附近也还有天空光");
        assert!(l.lamp_at(&g, near.0, near.1) > 0, "灯附近有点光源分量");
        assert_eq!(
            l.at(&g, near.0, near.1),
            l.sky_at(&g, near.0, near.1)
                .saturating_add(l.lamp_at(&g, near.0, near.1)),
            "level 必须是两个分量的和"
        );

        // 关掉天空光 → 天空光分量必须全 0，点光源分量不受影响
        l.sky = 0;
        l.rebuild(&g);
        assert!(l.sky_level.iter().all(|&v| v == 0), "sky = 0 时天空光分量应为 0");
        assert!(l.lamp_at(&g, 10, 5) > 0, "天空光关掉不该影响点光源");
    }

    #[test]
    fn absorb_covers_eighty_percent_of_height() {
        let g = grid(2, 150);
        let mut l = Light::new(2, 150);
        l.rebuild(&g);
        assert_eq!(l.absorb, 3, "150 行的 absorb 应该是 3");
        assert!(l.at(&g, 0, 84) > 0, "第 84 行还有光");
        assert_eq!(l.at(&g, 0, 85), 0, "第 85 行归零");
    }
}
