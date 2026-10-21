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
    /// 天空光**正午基准**（昼夜系数 = 1 时的 `sky_level`）。
    ///
    /// 昼夜每 tick 都要改 `sky_level`，但重跑 BFS 是 200~400 µs。所以拆成两层：
    /// `rebuild` 只算一次基准，之后 `set_daylight` 只是拿基准乘一个标量。
    /// **不进存档**：它是 `rebuild` 的纯产物，读档后会重算。
    #[serde(skip)]
    pub sky_base: Vec<u8>,
    /// 当前日照系数 0~1（1 = 正午，`night_floor` = 午夜）。
    ///
    /// 只作用于**天空光**：云挡得住太阳，挡不住玩家放的灯，昼夜也一样。
    /// 默认 1.0 = 逐位等于没有昼夜机制。
    pub daylight: f32,
    /// 点光源分量。不被身体遮挡（近似：灯可以从侧面照进来）。
    pub lamp_level: Vec<u8>,
    /// 玩家放置的点光源
    pub sources: Vec<LightSource>,
    /// 每格衰减多少，由世界高度推导（见 `absorb_for`）
    pub absorb: u8,
    /// 天空光强度（0 = 关掉，世界只剩点光源）
    pub sky: u8,

    /// **云影强度** 0~1（0 = 关闭，与旧行为逐位相同）。
    ///
    /// 天空光乘一张平滑噪声图：亮的成"光岛"，暗的成"暗廊"。
    /// 目的不是让世界变暗，而是**把明暗交界从一条水平线拉成几千格的锯齿**——
    /// 异养者需要那条交界（§9.7.11 的结论：光太均匀才是异养的瓶颈）。
    ///
    /// 代价必须知道：`mask ≤ 1`，所以开云影**一定降低总光照**。
    /// `p = 1` 时平均透射率约 0.5，等于世界暗了一半。
    /// 想保持总能量就同时把「天空光」或「光合速率」提上去。
    pub patchiness: f32,
    /// **富贫斑块**：天空光透射率的下限（1.0 = 关闭，默认）。
    ///
    /// 和 `patchiness` 是**两个独立效果**，共用同一张噪声图：
    ///
    /// | 旋钮 | 作用 | 极端值长什么样 |
    /// |---|---|---|
    /// | `patchiness` | 碎片化：把明暗交界拉长 | 1.0 = 光岛 + **全黑**暗廊 |
    /// | `patch_floor` | 质量梯度：暗处能有多暗 | 0.3 = 光岛 + **微光**暗廊 |
    ///
    /// 为什么需要第二个：`patchiness = 1` 的暗廊是**全黑**的，生物干脆不住；
    /// 于是"活着的地方"仍然一样亮，环境压力还是全局均匀的（批次 12.1 实测：
    /// 云影只让整体变穷，r/K 分化不出现）。
    ///
    /// 把下限抬到 0.3，暗廊变成**能住但很穷**的地方——那里光合只够维持、
    /// 不够长身体，芽会卡住 → 提前成熟 → 小体型（r）。光岛里则长满（K）。
    /// **同一张图上出现两种生态策略**，这才是"富斑块 / 贫斑块"的意义。
    pub patch_floor: f32,
    /// 噪声的种子。**不进存档**：它永远等于世界 RNG 的种子，
    /// 读档时由 `into_world` 重新灌进去（见 save.rs）。
    #[serde(skip)]
    pub mask_seed: u32,
    /// 每格的天空光透射率 0~1，**不进存档**——它是
    /// `(cols, rows, patchiness, mask_seed)` 的纯函数，重建时现算并缓存。
    /// 30k 格的噪声生成约 1 ms，地形画笔每帧都会触发 `rebuild`，所以必须缓存。
    #[serde(skip)]
    mask: Vec<f32>,
    /// 缓存键：`(cols, rows, patchiness.to_bits(), mask_seed)`。
    /// 任何一项变了才重算——`patchiness` 滑块是逐帧拖的，不能每次都算。
    /// `patch_floor` 不单独进键：它作用在**同一张噪声图**上，
    /// 变化时由 `World::refresh_light` 直接触发重建（见那里的比较）。
    #[serde(skip)]
    mask_key: (u32, u32, u32, u32),
}

impl Default for Light {
    fn default() -> Self {
        Light {
            len: 0,
            level: Vec::new(),
            sky_level: Vec::new(),
            sky_base: Vec::new(),
            daylight: 1.0,
            lamp_level: Vec::new(),
            sources: Vec::new(),
            absorb: 3,
            sky: SUN_MAX,
            patchiness: 0.0,
            patch_floor: 1.0,
            mask_seed: 0,
            mask: Vec::new(),
            mask_key: (0, 0, 0, 0),
        }
    }
}

impl Light {
    pub fn new(cols: usize, rows: usize) -> Self {
        let mut l = Light::default();
        l.len = cols * rows;
        l.level = vec![0; l.len];
        l.sky_level = vec![0; l.len];
        l.sky_base = vec![0; l.len];
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
        self.sky_base = vec![0; self.len];
        self.lamp_level = vec![0; self.len];
        self.absorb = absorb_for(rows);
        // 尺寸变了，噪声图必须重算
        self.mask.clear();
        self.mask_key = (0, 0, 0, 0);
        // 越界的光源丢掉：世界变小了，它们本来也照不到任何格子
        let (c, r) = (cols as i32, rows as i32);
        self.sources.retain(|s| s.col >= 0 && s.row >= 0 && s.col < c && s.row < r);
    }

    pub fn clear(&mut self) {
        self.level.fill(0);
        self.sky_level.fill(0);
        self.sky_base.fill(0);
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

        // ── 云影：给天空光乘一张平滑噪声图 ──
        //
        // **必须作用在传播之后**，不能只改顶部那一排种子的强度：
        // `absorb = 3` 时横向扩散很强（相邻列只差 3），顶行的逐列调制会被抹平，
        // 结果是一条几乎均匀的亮带，根本不是"岛"。
        // 乘在最终场上才是"云在天上挡住了一片天"这个物理图像。
        //
        // 只乘天空光：云挡得住太阳，挡不住玩家放的灯。
        // 云影（碎片化）和富贫斑块（质量梯度）共用这张掩码，任一开启都要乘。
        if self.patchiness > 0.0 || self.patch_floor < 1.0 {
            self.ensure_mask(grid.cols, grid.rows);
            for i in 0..self.len {
                if self.sky_level[i] > 0 {
                    self.sky_level[i] = (self.sky_level[i] as f32 * self.mask[i])
                        .round()
                        .clamp(0.0, 255.0) as u8;
                }
            }
        }

        // ── 存下"正午基准" ──
        //
        // 昼夜要逐 tick 改天空光，但重跑 BFS 是 200~400 µs。所以基准只算一次，
        // 之后 `set_daylight` 只是拿它乘一个标量（30k 次浮点乘 ≈ 30 µs）。
        if self.sky_base.len() != self.len {
            self.sky_base = vec![0; self.len];
        }
        self.sky_base.copy_from_slice(&self.sky_level);

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

        // 天空光乘日照系数、再和点光源相加（点光源**不随太阳转**）
        self.apply_daylight();
    }

    /// 把当前日照系数作用到天空光上，并刷新总和 `level`。
    ///
    /// `daylight >= 1.0` 走快速路径（直接拷基准），保证**默认配置逐位不变**。
    fn apply_daylight(&mut self) {
        if self.sky_base.len() != self.len {
            self.sky_base = self.sky_level.clone();
        }
        let d = self.daylight;
        if d >= 1.0 {
            self.sky_level.copy_from_slice(&self.sky_base);
        } else {
            let d = d.max(0.0);
            for i in 0..self.len {
                self.sky_level[i] = (self.sky_base[i] as f32 * d)
                    .round()
                    .clamp(0.0, 255.0) as u8;
            }
        }
        for i in 0..self.len {
            self.level[i] = self.sky_level[i].saturating_add(self.lamp_level[i]);
        }
    }

    /// 设置日照系数（0~1）。
    ///
    /// 变化小于 1/512 时直接返回：昼夜是逐 tick 连续变化的，每 tick 重算 30k 格
    /// 纯属浪费——一个 600 tick 的白天最多只需要约 255 次重算（正好是 u8 的级数）。
    pub fn set_daylight(&mut self, d: f32) {
        let d = d.clamp(0.0, 1.0);
        if (d - self.daylight).abs() < 1.0 / 512.0 {
            return;
        }
        self.daylight = d;
        self.apply_daylight();
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

    /// 保证噪声图和当前参数匹配。参数没变就直接用缓存。
    ///
    /// 为什么要缓存：`rebuild` 在地形画笔的每一帧都会被调用，
    /// 而 30k 格 × 2 个八度 × 4 次散列约 1 ms —— 每帧重算会让画笔明显卡顿。
    fn ensure_mask(&mut self, cols: usize, rows: usize) {
        let key = (
            cols as u32,
            rows as u32,
            self.patchiness.to_bits(),
            self.mask_seed,
        );
        if self.mask_key == key && self.mask.len() == self.len {
            return;
        }
        self.mask = build_cloud_mask(
            cols,
            rows,
            self.patchiness,
            self.patch_floor,
            self.mask_seed,
        );
        self.mask_key = key;
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

// ────────────────────────────── 云影噪声 ──────────────────────────────
//
// 天空光透射率 = 1 − p·(1 − 噪声)。p = 0 时恒为 1（旧行为），p = 1 时完全由噪声决定。
// 噪声必须**确定性**：同一个 (尺寸, p, 种子) 每次都要生成同一张图，
// 否则改一次地形（触发 rebuild）云就跳一下。

/// 32 位整数散列 → `[0,1)`。用来生成噪声的格点值。
#[inline]
fn hash2(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = seed
        ^ (x as u32).wrapping_mul(0x9E37_79B9)
        ^ (y as u32).wrapping_mul(0x85EB_CA6B);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB_352D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846C_A68B);
    h ^= h >> 16;
    (h >> 8) as f32 / 16_777_216.0
}

/// 三次平滑（smoothstep），让格点之间不是折线。
#[inline]
fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// 横向**周期**的值噪声。
///
/// 必须周期：`wrap_edges = true` 时第 0 列和最后一列是相邻的，
/// 非周期噪声会在接缝上留一条突兀的直线。
/// 做法是把 x 方向的格点下标对 `period` 取模，`period` 由"列数 / 格宽"取整而来，
/// 再把实际格宽校正成 `列数 / period`，于是左右天然对齐。
fn periodic_noise(col: f32, row: f32, cell: f32, cols: usize, seed: u32, offset: f32) -> f32 {
    let period = ((cols as f32 / cell).round() as i32).max(1);
    let cw = cols as f32 / period as f32;
    // **采样点要避开格线**：如果世界第 0 行/第 0 列正好落在噪声的整数格线上，
    // 那里只有一条插值切片，取值会被压窄（实测顶行只剩 195~255，
    // 明明是"云"却看着像一条均匀的亮带）。偏移量对环绕没有影响：
    // `col` 加常数不改变小数部分，格点下标照样按 `period` 取模。
    let x = col / cw + offset;
    let y = row / cell + offset * 1.7;
    let xi = x.floor();
    let yi = y.floor();
    let u = smooth(x - xi);
    let v = smooth(y - yi);
    let x0 = (xi as i32).rem_euclid(period);
    let x1 = (x0 + 1) % period;
    let y0 = yi as i32;
    let a = hash2(x0, y0, seed);
    let b = hash2(x1, y0, seed);
    let c = hash2(x0, y0 + 1, seed);
    let d = hash2(x1, y0 + 1, seed);
    let top = a + (b - a) * u;
    let bot = c + (d - c) * u;
    top + (bot - top) * v
}

/// 生成整张天空光透射率图（0~1，越大越透）。
///
/// 两个八度：大的（44 格）定"光岛在哪"，小的（17 格）定"岛的边缘有多碎"。
/// 权重 0.65 / 0.35 —— 主八度必须明显占优，否则岛会被打散成斑点。
pub fn cloud_noise(cols: usize, rows: usize, seed: u32) -> Vec<f32> {
    // (格宽, 权重, 采样偏移)。两个八度的偏移必须不同，否则它们在同一条格线上相关，
    // 细节会被主八度吃掉，看不出"碎边"。
    const OCTAVES: [(f32, f32, f32); 2] = [(44.0, 0.65, 0.37), (17.0, 0.35, 0.71)];
    // 两个八度一平均，取值会往 0.5 挤（实测原始噪声只落在 0.25~0.75），
    // 直接用的话"最暗的岛"也还有 60% 的光，根本不算暗。这里拉一次对比度并夹紧——
    // 夹到 0/1 的部分就是"实心暗块 / 实心亮块"，正好是云该有的样子。
    const CONTRAST: f32 = 3.0;

    let mut raw = vec![0.0f32; cols * rows];
    let mut sum = 0.0f32;
    for r in 0..rows {
        for c in 0..cols {
            let mut n = 0.0;
            for (cell, w, off) in OCTAVES {
                n += w * periodic_noise(c as f32, r as f32, cell, cols, seed, off);
            }
            raw[r * cols + c] = n;
            sum += n;
        }
    }

    // **先去掉均值偏差再拉对比度**。小世界里主八度只有几个格点，
    // 实际均值可能偏到 0.35（实测 120×60、p=1 时就是这样），
    // 那等于"云影"顺带把世界调暗了 30%，A/B 对比就分不清是"碎片化"还是"变暗"的功劳。
    // 归一化之后 `p` 变成一个**纯粹的碎片化旋钮**：p=1 永远约等于"平均一半的光，
    // 但一半的地方更亮、一半更暗"。想单独测"变暗"就把「天空光」往下拨。
    let mean = if raw.is_empty() { 0.5 } else { sum / raw.len() as f32 };
    for n in raw.iter_mut() {
        *n = ((*n - mean) * CONTRAST + 0.5).clamp(0.0, 1.0);
    }
    raw
}

/// 生成整张天空光透射率图（0~1，越大越透）。
///
/// 两个独立效果乘在一起，共用同一张噪声 `n`：
///
/// ```text
/// 云影碎片化： 1 - p × (1 - n)          p = patchiness
/// 富贫斑块  ： floor + (1 - floor) × n   floor = patch_floor
/// 透射率    ： 两者相乘
/// ```
///
/// `p = 0 且 floor = 1` 时结果是恒 1 —— 默认配置**逐位等于旧行为**。
fn build_cloud_mask(cols: usize, rows: usize, p: f32, floor: f32, seed: u32) -> Vec<f32> {
    let p = p.clamp(0.0, 1.0);
    let floor = floor.clamp(0.0, 1.0);
    let mut out = vec![1.0f32; cols * rows];
    if p <= 0.0 && floor >= 1.0 {
        return out;
    }
    let n = cloud_noise(cols, rows, seed);
    for (i, v) in n.iter().enumerate() {
        let cloud = 1.0 - p * (1.0 - v);
        let rich = floor + (1.0 - floor) * v;
        out[i] = cloud * rich;
    }
    out
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

    // ── 批次 10.5：云影（天空光的空间碎片化）──

    /// `patchiness = 0` 必须**逐位**等于旧行为：云影是可选效果，不能悄悄改基线。
    #[test]
    fn patchiness_zero_leaves_the_field_untouched() {
        let g = grid(40, 30);
        let mut plain = Light::new(40, 30);
        plain.rebuild(&g);
        let before = plain.level.clone();

        let mut cloudy = Light::new(40, 30);
        cloudy.patchiness = 0.0;
        cloudy.mask_seed = 12345;
        cloudy.rebuild(&g);
        assert_eq!(cloudy.level, before, "云影 0 时不应改变任何一格");
    }

    /// 云影必须真的**在水平方向**造出明暗：顶行既有接近满照的列，也有接近全黑的列。
    ///
    /// 这条是在验"光岛"这件事本身。如果实现成只调制顶行种子的强度，
    /// 横向扩散（absorb = 3，相邻列只差 3）会把调制抹平，测试会挂在这里。
    #[test]
    fn clouds_make_light_patchy() {
        let g = grid(120, 60);
        let mut l = Light::new(120, 60);
        l.patchiness = 1.0;
        l.mask_seed = 777;
        l.rebuild(&g);

        let top: Vec<u8> = (0..120).map(|c| l.at(&g, c, 0)).collect();
        let hi = *top.iter().max().unwrap();
        let lo = *top.iter().min().unwrap();
        assert!(hi > 200, "顶行应该有接近满照的列，实测最亮 {}", hi);
        assert!(lo < 80, "顶行应该有很暗的列，实测最暗 {}", lo);
        assert!(
            hi as i32 - lo as i32 > 150,
            "明暗差应该很大（{} vs {}）",
            hi,
            lo
        );

        // 不是"一条亮带"：顶行的取值必须够分散
        let mut uniq: Vec<u8> = top.clone();
        uniq.sort_unstable();
        uniq.dedup();
        assert!(uniq.len() > 20, "顶行取值应该很分散，实测只有 {} 种", uniq.len());

        // 归一化：p = 1 时平均透射率应该接近 0.5，而不是被"这个种子恰好很阴"带跑。
        // 这一条守的是 A/B 对比的可解释性——p 必须只代表"碎片化"，不代表"变暗"。
        let m = build_cloud_mask(120, 60, 1.0, 1.0, 777);
        let avg = m.iter().sum::<f32>() / m.len() as f32;
        assert!(
            (avg - 0.5).abs() < 0.06,
            "p=1 时平均透射率应该约 0.5，实测 {:.3}",
            avg
        );

        // 真正的"暗区"要够多：只把光调成一片灰不算稀缺。
        // 实测 CONTRAST = 3.0 时约 14% 的格子透射率 < 0.15、20% > 0.85。
        let dark = m.iter().filter(|&&v| v < 0.15).count();
        let bright = m.iter().filter(|&&v| v > 0.85).count();
        assert!(
            dark * 100 / m.len() > 8,
            "透射率 < 0.15 的格子应该占 8% 以上，实测 {}%",
            dark * 100 / m.len()
        );
        assert!(
            bright * 100 / m.len() > 8,
            "透射率 > 0.85 的格子应该占 8% 以上，实测 {}%",
            bright * 100 / m.len()
        );
    }

    /// 同一组 (尺寸, p, 种子) 必须生成同一张图；换种子要换一张图。
    ///
    /// 这一条守的是"地形画笔触发 rebuild 时云不能跳"——噪声如果带随机性，
    /// 每画一格墙云就闪一下。
    #[test]
    fn clouds_are_deterministic_and_seeded() {
        let g = grid(60, 40);
        let build = |seed: u32| {
            let mut l = Light::new(60, 40);
            l.patchiness = 0.8;
            l.mask_seed = seed;
            l.rebuild(&g);
            l.level.clone()
        };
        assert_eq!(build(1), build(1), "同种子必须逐位相同");
        assert_ne!(build(1), build(2), "不同种子应该长出不同的云");
    }

    /// 云只挡太阳，**不挡玩家放的灯**。
    #[test]
    fn clouds_do_not_dim_lamps() {
        let g = grid(60, 60);
        let mut clear = Light::new(60, 60);
        clear.set_source(30, 30, 255);
        clear.rebuild(&g);
        let lamp_before = clear.lamp_level.clone();

        let mut cloudy = Light::new(60, 60);
        cloudy.patchiness = 1.0;
        cloudy.mask_seed = 999;
        cloudy.set_source(30, 30, 255);
        cloudy.rebuild(&g);
        assert_eq!(cloudy.lamp_level, lamp_before, "云影不该影响点光源分量");
        assert!(
            cloudy.level[30 * 60 + 30] >= lamp_before[30 * 60 + 30],
            "灯下总光强不该比灯本身暗"
        );
    }

    /// 噪声在横向必须**首尾相接**：世界是左右环绕的，接缝上不能出现一条硬边。
    #[test]
    fn clouds_wrap_horizontally() {
        let cols = 200;
        let cell = 44.0;
        for seed in [0u32, 7, 424242] {
            let a = periodic_noise(0.0, 13.0, cell, cols, seed, 0.37);
            let b = periodic_noise(cols as f32, 13.0, cell, cols, seed, 0.37);
            assert!(
                (a - b).abs() < 1e-6,
                "第 0 列与第 {} 列（环绕后是同一格）应该相等：{} vs {}",
                cols,
                a,
                b
            );
        }
    }
}

#[cfg(test)]
mod patch_tests {
    use super::*;

    /// `patch_floor = 1.0`（默认）时富贫斑块**完全不生效**：掩码恒为 1。
    ///
    /// 这是默认配置的守门测试——它一旦挂，所有既有生态基线都会悄悄偏移。
    #[test]
    fn floor_one_is_a_no_op() {
        let g = Grid::new(60, 40, true);
        let mut l = Light::new(60, 40);
        l.patchiness = 0.0;
        l.patch_floor = 1.0;
        l.mask_seed = 1234;
        l.rebuild(&g);
        let plain = {
            let mut p = Light::new(60, 40);
            p.mask_seed = 1234;
            p.rebuild(&g);
            p.level
        };
        assert_eq!(l.level, plain, "floor = 1 必须逐位等于没有富贫斑块");
    }

    /// 富贫斑块造的是**梯度**，不是"墙"：最穷的地方也有光，只是很少。
    ///
    /// 这一条是这一批的核心断言。云影（`patchiness = 1, floor = 0`）会把暗廊
    /// 压成全黑——那里生物干脆不住，于是"活着的地方"仍然一样亮，
    /// 环境压力还是全局均匀的。抬高地板之后暗廊**能住但很穷**，
    /// 同一个世界里才会同时出现两种策略。
    #[test]
    fn richness_makes_a_gradient_not_a_wall() {
        let g = Grid::new(120, 60, true);
        let mut l = Light::new(120, 60);
        l.patchiness = 0.0;
        l.patch_floor = 0.3; // = patch_richness 0.7
        l.mask_seed = 777;
        l.rebuild(&g);

        let top: Vec<u8> = (0..120).map(|c| l.at(&g, c, 0)).collect();
        let hi = *top.iter().max().unwrap();
        let lo = *top.iter().min().unwrap();

        assert!(hi > 230, "富斑块应该接近满照，实测最亮 {}", hi);
        // 噪声不一定恰好取到 0，所以最穷的格子会略高于 0.3×255 ≈ 76。
        assert!(
            (60..=110).contains(&(lo as i32)),
            "贫斑块应该在 0.3×255 ≈ 76 附近，实测最暗 {}",
            lo
        );
        assert!(
            lo > 40,
            "贫斑块**不能是黑的**——那会变成坟场而不是生态位，实测最暗 {}",
            lo
        );
        assert!(hi as i32 - lo as i32 > 100, "富贫差距应该很明显");
    }

    /// 两个效果**相乘**：富贫斑块只能让光更少，永远不会让光变多。
    ///
    /// 这条守的是叠加时的方向性。具体来说，云影的"全黑暗廊"在叠加后仍然是全黑
    /// （0 乘任何数都是 0）——这是对的，云真的挡住了太阳。想要"微光暗廊"
    /// 就只开富贫斑块、别开云影，两个旋钮各管一种世界。
    #[test]
    fn richness_never_brightens_anything() {
        let g = Grid::new(120, 60, true);
        let build = |p: f32, floor: f32| {
            let mut l = Light::new(120, 60);
            l.patchiness = p;
            l.patch_floor = floor;
            l.mask_seed = 424242;
            l.rebuild(&g);
            (0..120)
                .map(|c| l.at(&g, c, 0) as u32)
                .collect::<Vec<u32>>()
        };
        let cloud_only = build(0.6, 1.0);
        let both = build(0.6, 0.3);
        for i in 0..cloud_only.len() {
            assert!(
                both[i] <= cloud_only[i],
                "第 {} 列：富贫斑块让光变亮了（{} > {}）",
                i,
                both[i],
                cloud_only[i]
            );
        }
        assert!(
            both.iter().sum::<u32>() < cloud_only.iter().sum::<u32>(),
            "叠加之后总光照应该更少"
        );
        // 只开云影时暗廊是全黑的
        let pure_cloud = build(1.0, 1.0);
        assert_eq!(*pure_cloud.iter().min().unwrap(), 0, "只开云影时暗廊全黑");
    }
}

#[cfg(test)]
mod daylight_tests {
    use super::*;

    /// `daylight = 1.0`（默认）必须**逐位**等于没有昼夜机制。
    ///
    /// 这是默认配置的守门测试：昼夜是可选的，不能悄悄改掉所有既有生态基线。
    #[test]
    fn daylight_one_is_a_no_op() {
        let g = Grid::new(40, 30, true);
        let mut plain = Light::new(40, 30);
        plain.rebuild(&g);
        let before = plain.level.clone();
        let sky_before = plain.sky_level.clone();

        let mut with_day = Light::new(40, 30);
        with_day.daylight = 1.0;
        with_day.rebuild(&g);
        assert_eq!(with_day.level, before, "daylight = 1 不应改变任何一格");
        assert_eq!(with_day.sky_level, sky_before);
    }

    /// 日照只压**天空光**，点光源照旧——灯不随太阳转。
    #[test]
    fn daylight_dims_sky_but_not_lamps() {
        let g = Grid::new(40, 40, true);
        let mut l = Light::new(40, 40);
        l.set_source(20, 30, 255);
        l.rebuild(&g);
        let lamp_before = l.lamp_level.clone();
        let sky_noon = l.sky_at(&g, 20, 0);

        l.set_daylight(0.1);
        assert_eq!(l.lamp_level, lamp_before, "点光源不该随昼夜变暗");
        let sky_night = l.sky_at(&g, 20, 0);
        assert!(
            sky_night < sky_noon,
            "夜间天空光应该更暗（正午 {} → 夜间 {}）",
            sky_noon,
            sky_night
        );
        // 0.1 × 255 ≈ 26，允许 ±2 的取整误差
        assert!(
            (sky_night as i32 - 26).abs() <= 2,
            "night_floor 0.1 时顶行应该约 26，实测 {}",
            sky_night
        );
        // 灯所在的格子仍然有光（夜里唯一的光源）
        assert!(l.at(&g, 20, 30) >= 200, "夜里灯自己那格应该还是亮的");
    }

    /// 日照系数的变化有量化：差不到 1/512 就不重算（性能守门）。
    #[test]
    fn tiny_daylight_changes_are_skipped() {
        let g = Grid::new(20, 20, true);
        let mut l = Light::new(20, 20);
        l.rebuild(&g);
        let before = l.sky_level.clone();
        l.set_daylight(1.0 - 1.0 / 1024.0); // 小于阈值
        assert_eq!(l.sky_level, before, "微小变化不该触发重算");
        l.set_daylight(0.5);
        assert_ne!(l.sky_level, before, "显著变化必须生效");
    }

    /// 重跑 `rebuild` 之后必须仍然按当前日照系数生效，而不是回到满照。
    ///
    /// 这条守的是「画地形 / 放灯触发 rebuild 时，夜晚不能突然变成白天」。
    #[test]
    fn rebuild_keeps_the_current_daylight() {
        let g = Grid::new(20, 20, true);
        let mut l = Light::new(20, 20);
        l.set_daylight(0.2);
        l.rebuild(&g);
        let dark = l.sky_at(&g, 10, 0);
        assert!(dark < 80, "rebuild 之后应该还是夜里，实测 {}", dark);

        l.rebuild(&g); // 再 rebuild 一次，结果必须一样
        assert_eq!(l.sky_at(&g, 10, 0), dark, "rebuild 必须幂等");
    }
}
