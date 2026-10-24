//! 身体蓝图 —— 基因组 A。与 JS 版 `src/life/Anatomy.js` 对应。

use serde::{Deserialize, Serialize};

use crate::cell_type::{def, is_directional, flag};
use crate::rng::Rng;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BodyCell {
    pub type_id: u8,
    pub dx: i32,
    pub dy: i32,
    /// 局部朝向 0-3，仅带朝向的细胞（眼睛）使用
    pub dir: u8,
}

impl BodyCell {
    pub fn new(type_id: u8, dx: i32, dy: i32, dir: u8) -> Self {
        BodyCell {
            type_id,
            dx,
            dy,
            dir: if is_directional(type_id) { dir & 3 } else { 0 },
        }
    }

    /// 用于物种指纹：类型 + 位置（眼睛的朝向也算结构的一部分）
    pub fn signature(&self) -> String {
        if is_directional(self.type_id) {
            format!("{}:{},{},{}", self.type_id, self.dx, self.dy, self.dir)
        } else {
            format!("{}:{},{}", self.type_id, self.dx, self.dy)
        }
    }
}

/// 能力标志位集合
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flags(pub u16);

impl Flags {
    #[inline]
    pub fn has(self, bit: u16) -> bool {
        self.0 & bit != 0
    }
    #[inline]
    pub fn set(&mut self, bit: u16, on: bool) {
        if on {
            self.0 |= bit;
        } else {
            self.0 &= !bit;
        }
    }
    pub const MOUTH: u16 = flag::MOUTH;
    pub const PRODUCER: u16 = flag::PRODUCER;
    pub const MOVER: u16 = flag::MOVER;
    pub const KILLER: u16 = flag::KILLER;
    pub const ARMOR: u16 = flag::ARMOR;
    pub const EYE: u16 = flag::EYE;
    pub const STORAGE: u16 = flag::STORAGE;
    pub const REGEN: u16 = flag::REGEN;
    pub const SPIKE: u16 = flag::SPIKE;
}

/// 身体坐标里的**八**邻域方向。
///
/// 为什么是八邻域而不是四邻域：身体本身是**八连通**的（发育用八邻域 BFS，
/// 见 `world::grow_organism`），所以一条对角线的身体是**一个**身体。
/// 用四邻域数暴露面的话，这种身体每个细胞的 4 个面全都"暴露"，
/// 算出来和一堆互不相连的孤立格完全一样——机制就看不见它了。
///
/// 实测证据：改成八邻域之前，2000 tick 的平均 `svr` 是 1.09，
/// 而平均体型是 3.55 格——3.5 个格子的身体不可能真的只有 9% 的"厚度"，
/// 那是四邻域把对角连接当成了断开的。
const DIRS: [(i32, i32); 8] = [
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (1, 1),
    (1, -1),
    (-1, 1),
    (-1, -1),
];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Anatomy {
    pub cells: Vec<BodyCell>,
    pub birth_distance: i32,
    pub flags: Flags,
    /// **暴露面数**：所有细胞 4 邻域里「不属于本体」的方向的个数。
    ///
    /// 这是「体表面积」的离散版本，也是批次 47「体型收益」的唯一几何依据。
    /// 1 格身体 = 4；2 格并排 = 6；2×2 方块 = 8；3×3 方块 = 12；
    /// 3 格的直线 = 8（同样是 3 格，直线比方块多 2 个暴露面）。
    ///
    /// **它是缓存值**：`add_cell` 增量维护，`recompute` 全量重算。
    /// 直接改 `cells` 而不走这两个入口会让它变旧——`perimeter_matches_recompute`
    /// 那条测试就是防这个的。
    pub perimeter: u16,
}

impl Default for Anatomy {
    fn default() -> Self {
        Anatomy {
            cells: Vec::new(),
            birth_distance: 4,
            flags: Flags::default(),
            perimeter: 0,
        }
    }
}

impl Anatomy {
    pub fn cell_count(&self) -> usize {
        self.cells.len()
    }

    pub fn has(&self, bit: u16) -> bool {
        self.flags.has(bit)
    }

    pub fn clear(&mut self) {
        self.cells.clear();
        self.birth_distance = 4;
        self.flags = Flags::default();
        self.perimeter = 0;
    }

    /// 添加一个细胞。位置已被占用时返回 false。
    pub fn add_cell(&mut self, type_id: u8, dx: i32, dy: i32, dir: u8) -> bool {
        if self.cell_at(dx, dy).is_some() {
            return false;
        }
        // 暴露面的增量：新格带来 8 个面，每有一个已有邻居，两边各少一个面。
        let adjacent = DIRS
            .iter()
            .filter(|(ox, oy)| self.cell_at(dx + ox, dy + oy).is_some())
            .count() as i32;
        self.perimeter = (self.perimeter as i32 + 8 - 2 * adjacent).max(0) as u16;
        self.cells.push(BodyCell::new(type_id, dx, dy, dir));

        let distance = (dy.abs() * 2 + 2).max(dx.abs() * 2 + 2);
        if distance > self.birth_distance {
            self.birth_distance = distance;
        }
        self.apply_flag(type_id, true);
        true
    }

    /// 体表面积（暴露面数）的**全量**重算。O(n²)，只在结构变动时调。
    pub fn recompute_perimeter(&self) -> u16 {
        let mut p = 0u16;
        for c in &self.cells {
            for (ox, oy) in DIRS {
                if self.cell_at(c.dx + ox, c.dy + oy).is_none() {
                    p += 1;
                }
            }
        }
        p
    }

    /// 体表/体积比：`暴露面数 / (8 × 细胞数)`。
    ///
    /// **归一化到 1 格 = 1.0**（1 格身体 8 个面 / 8 = 1.0），越大越紧凑越小：
    /// 3×3 方块 = 0.33，10×10 = 0.125。
    /// 直线身体几乎吃不到好处（n 格直线 ≈ 0.56），所以「紧凑」和「大」
    /// 是两件不同的事——这正是批次 47 想让它可被选择的东西。
    pub fn exposure(&self) -> f64 {
        let n = self.cells.len();
        if n == 0 {
            return 0.0;
        }
        self.perimeter.max(1) as f64 / (8.0 * n as f64)
    }

    pub fn cell_at(&self, dx: i32, dy: i32) -> Option<&BodyCell> {
        self.cells.iter().find(|c| c.dx == dx && c.dy == dy)
    }

    pub fn cell_at_mut(&mut self, dx: i32, dy: i32) -> Option<&mut BodyCell> {
        self.cells.iter_mut().find(|c| c.dx == dx && c.dy == dy)
    }

    /// 按局部坐标移除。中心格 (0,0) 不可移除。
    pub fn remove_cell_at(&mut self, dx: i32, dy: i32) -> bool {
        if dx == 0 && dy == 0 {
            return false;
        }
        if let Some(i) = self.cells.iter().position(|c| c.dx == dx && c.dy == dy) {
            self.cells.remove(i);
            self.recompute();
            return true;
        }
        false
    }

    /// 删除一个随机细胞（中心格除外，且至少保留一个）
    pub fn remove_random_cell(&mut self, rng: &mut Rng) -> bool {
        if self.cells.len() <= 1 {
            return false;
        }
        for _ in 0..8 {
            let i = rng.int(self.cells.len());
            let (dx, dy) = (self.cells[i].dx, self.cells[i].dy);
            if dx == 0 && dy == 0 {
                continue;
            }
            return self.remove_cell_at(dx, dy);
        }
        // 兜底：找第一个非中心格
        for i in 0..self.cells.len() {
            let (dx, dy) = (self.cells[i].dx, self.cells[i].dy);
            if dx != 0 || dy != 0 {
                return self.remove_cell_at(dx, dy);
            }
        }
        false
    }

    /// 随机取一个细胞的索引
    pub fn random_cell_index(&self, rng: &mut Rng) -> Option<usize> {
        if self.cells.is_empty() {
            None
        } else {
            Some(rng.int(self.cells.len()))
        }
    }

    pub fn recompute(&mut self) {
        self.flags = Flags::default();
        let mut birth = 4;
        for c in &self.cells {
            if let Some(d) = def(c.type_id) {
                if let Some(bit) = d.flag {
                    self.flags.set(bit, true);
                }
            }
            let distance = (c.dy.abs() * 2 + 2).max(c.dx.abs() * 2 + 2);
            if distance > birth {
                birth = distance;
            }
        }
        self.birth_distance = birth;
        self.perimeter = self.recompute_perimeter();
    }

    fn apply_flag(&mut self, type_id: u8, value: bool) {
        if let Some(d) = def(type_id) {
            if let Some(bit) = d.flag {
                self.flags.set(bit, value);
            }
        }
    }

    pub fn copy_from(&mut self, other: &Anatomy) {
        self.clear();
        self.birth_distance = other.birth_distance;
        self.cells.extend_from_slice(&other.cells);
        self.recompute();
    }

    /// 最大半径（出生位置与渲染高亮用）
    pub fn radius(&self) -> i32 {
        self.cells
            .iter()
            .map(|c| c.dx.abs().max(c.dy.abs()))
            .max()
            .unwrap_or(0)
    }

    /// 结构指纹 —— 物种判据。与位置、细胞顺序无关。
    pub fn fingerprint(&self) -> String {
        let mut parts: Vec<String> = self.cells.iter().map(|c| c.signature()).collect();
        parts.sort();
        hash_string(&parts.join("|"))
    }

    /// **身体构成**：每种细胞各几个。索引 = 细胞类型 id。
    ///
    /// 这是「物种」的**连续**判据用的量（批次 32）。和 `fingerprint()` 的分工：
    /// - `fingerprint()` 是**离散**的：变异一格就换一个指纹。它管**谱系**（谁从谁变来）。
    /// - `composition()` 是**连续**的：加一格只让一个数 +1。它管**物种**（差几格还算同一种）。
    ///
    /// 为什么不用逐格位置：位置版本（多重集对称差）要排序，而这条路径在**每次咬**都会走。
    /// 类型计数是 O(格数) 且不分配，够快。代价是分不出「5 格一字长蛇」和「5 格团」——
    /// 但项目里人读的物种名本来也是 `describe()` 那种构成描述（"嘴×1 生产者×2"），
    /// 所以这个粒度和界面口径是一致的。
    pub fn composition(&self) -> [u16; 17] {
        let mut c = [0u16; 17];
        for cell in &self.cells {
            c[(cell.type_id as usize).min(16)] += 1;
        }
        c
    }

    /// **构成距离**（0~1）：两个身体"差多少"。0 = 完全一样，1 = 一格都不重合。
    ///
    /// 用 Jaccard 距离（`1 - Σmin / Σmax`）而不是绝对格数差，是因为绝对差**不看体型**：
    /// 「1 格嘴」和「1 格生产者」绝对差是 2，「14 格身体多一格」也是 1——前者该是
    /// 完全不同的东西，后者几乎没变。归一化之后前者是 1.0、后者是 0.07。
    ///
    /// 这是**物种**的连续判据：指纹（逐格哈希）管谱系，这个管"多少种形态在共存"。
    pub fn composition_distance(&self, other: &Anatomy) -> f32 {
        let a = self.composition();
        let b = other.composition();
        let mut inter = 0u32;
        let mut union = 0u32;
        for i in 0..17 {
            let (x, y) = (a[i] as u32, b[i] as u32);
            inter += x.min(y);
            union += x.max(y);
        }
        if union == 0 {
            return 0.0;
        }
        1.0 - inter as f32 / union as f32
    }

    /// 可读的结构描述，例如 "嘴×1 生产者×2"
    pub fn describe(&self) -> String {
        let mut counts = [0u32; 16];
        for c in &self.cells {
            counts[(c.type_id as usize).min(15)] += 1;
        }
        let mut out = String::new();
        for (t, n) in counts.iter().enumerate() {
            if *n == 0 {
                continue;
            }
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(crate::cell_type::label(t as u8));
            out.push('×');
            out.push_str(&n.to_string());
        }
        out
    }
}

/// FNV-1a 32 位哈希，输出 8 位十六进制（与 JS 版逐位一致）
pub fn hash_string(s: &str) -> String {
    let mut hash: u32 = 0x811c_9dc5;
    for b in s.bytes() {
        hash ^= b as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    format!("{:08x}", hash)
}

/// 局部坐标绕中心枢轴顺时针旋转 rotation × 90°
#[inline]
pub fn rotate_col(dx: i32, dy: i32, rotation: u8) -> i32 {
    match rotation & 3 {
        0 => dx,
        1 => -dy,
        2 => -dx,
        _ => dy,
    }
}

#[inline]
pub fn rotate_row(dx: i32, dy: i32, rotation: u8) -> i32 {
    match rotation & 3 {
        0 => dy,
        1 => dx,
        2 => -dy,
        _ => -dx,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell_type::ct;

    #[test]
    fn fingerprint_is_order_independent() {
        let mut a = Anatomy::default();
        a.add_cell(ct::MOUTH, 0, 0, 0);
        a.add_cell(ct::PRODUCER, 1, 1, 0);

        let mut b = Anatomy::default();
        b.add_cell(ct::PRODUCER, 1, 1, 0);
        b.add_cell(ct::MOUTH, 0, 0, 0);

        assert_eq!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn rotation_is_clockwise() {
        // (1,0) 顺时针 90° → (0,1)
        assert_eq!((rotate_col(1, 0, 1), rotate_row(1, 0, 1)), (0, 1));
        // 再转 90° → (-1,0)
        assert_eq!((rotate_col(1, 0, 2), rotate_row(1, 0, 2)), (-1, 0));
    }
}

#[cfg(test)]
mod storage_tests {
    use super::*;
    use crate::cell_type::ct;

    #[test]
    fn storage_flag_is_set() {
        let mut a = Anatomy::default();
        a.add_cell(ct::MOUTH, 0, 0, 0);
        a.add_cell(ct::STORAGE, 1, 1, 0);
        a.add_cell(ct::PRODUCER, -1, -1, 0);
        assert!(a.has(Flags::STORAGE), "add_cell 应置位 STORAGE，flags={:?}", a.flags);
        let mut b = Anatomy::default();
        b.copy_from(&a);
        assert!(b.has(Flags::STORAGE), "copy_from 后应保留 STORAGE");
    }
}

#[cfg(test)]
mod composition_tests {
    use super::*;
    use crate::cell_type::ct;

    fn body(cells: &[(u8, i32, i32)]) -> Anatomy {
        let mut a = Anatomy::default();
        for (t, dx, dy) in cells {
            a.add_cell(*t, *dx, *dy, 0);
        }
        a.recompute();
        a
    }

    /// 构成距离：一样是 0，多一格是小值，换一种细胞是完全不同的东西。
    #[test]
    fn composition_distance_is_scale_free() {
        let a = body(&[
            (ct::PRODUCER, 0, 0),
            (ct::PRODUCER, 1, 0),
            (ct::PRODUCER, 2, 0),
            (ct::MOUTH, 3, 0),
        ]);
        assert_eq!(a.composition_distance(&a), 0.0, "自己和自己应该是 0");

        // 14 格身体多一格：距离很小
        let mut big = Anatomy::default();
        for k in 0..10 {
            big.add_cell(ct::PRODUCER, k, 0, 0);
        }
        for k in 0..4 {
            big.add_cell(ct::MOUTH, k, 1, 0);
        }
        big.recompute();
        let mut bigger = big.clone();
        bigger.add_cell(ct::PRODUCER, 10, 0, 0);
        bigger.recompute();
        let d = big.composition_distance(&bigger);
        assert!(d < 0.1, "14 格多一格应该 < 0.1，实测 {}", d);

        // 1 格嘴 vs 1 格生产者：完全不同的东西
        let m = body(&[(ct::MOUTH, 0, 0)]);
        let p = body(&[(ct::PRODUCER, 0, 0)]);
        assert_eq!(
            m.composition_distance(&p),
            1.0,
            "一格全换掉应该是 1.0（Jaccard 归一化的意义就在这里）"
        );
    }

    /// 位置不算：同构成不同摆法 = 距离 0（这是"物种"口径，不是"谱系"口径）。
    #[test]
    fn composition_ignores_layout_but_fingerprint_does_not() {
        let a = body(&[(ct::PRODUCER, 0, 0), (ct::MOUTH, 1, 0)]);
        let b = body(&[(ct::PRODUCER, 0, 0), (ct::MOUTH, 0, 1)]);
        assert_eq!(a.composition_distance(&b), 0.0, "构成一样就是 0");
        assert_ne!(
            a.fingerprint(),
            b.fingerprint(),
            "指纹应该不同（位置变了）——两个口径本来就要分开"
        );
    }
}

#[cfg(test)]
mod perimeter_tests {
    use super::*;
    use crate::cell_type::ct;

    fn body(cells: &[(i32, i32)]) -> Anatomy {
        let mut a = Anatomy::default();
        for (dx, dy) in cells {
            a.add_cell(ct::PRODUCER, *dx, *dy, 0);
        }
        a
    }

    /// 离散体表面积：每个细胞贡献 4 个面，每个相邻对消掉 2 个。
    #[test]
    fn perimeter_of_known_shapes() {
        assert_eq!(body(&[(0, 0)]).perimeter, 8, "单格 8 面");
        assert_eq!(body(&[(0, 0), (1, 0)]).perimeter, 14, "并排两格 14 面");
        // 对角相邻也算连通（身体是八连通的）
        assert_eq!(
            body(&[(0, 0), (1, 1)]).perimeter,
            14,
            "对角两格也是 14 面——四邻域会错算成 16"
        );
        assert_eq!(body(&[(0, 0), (1, 0), (2, 0)]).perimeter, 20, "三格直线 20 面");
        // 2×2 方块：每格 3 个邻居 → 32 − 12 = 20
        assert_eq!(
            body(&[(0, 0), (1, 0), (0, 1), (1, 1)]).perimeter,
            20,
            "2×2 方块 20 面（和 3 格直线面数一样，但格数更多 → 更厚实）"
        );
        // 3×3 方块：20 个邻接对 → 72 − 40 = 32
        let mut sq = Anatomy::default();
        for dy in 0..3 {
            for dx in 0..3 {
                sq.add_cell(ct::PRODUCER, dx, dy, 0);
            }
        }
        assert_eq!(sq.perimeter, 32, "3×3 方块 32 面");
    }

    /// 增量维护必须和全量重算逐位一致——这是缓存字段唯一的风险点。
    #[test]
    fn perimeter_matches_recompute_after_edits() {
        let mut a = Anatomy::default();
        let coords = [
            (0, 0),
            (1, 0),
            (0, 1),
            (-1, 0),
            (0, -1),
            (1, 1),
            (2, 0),
            (2, 1),
            (2, 2),
            (-1, -1),
        ];
        for (dx, dy) in coords {
            a.add_cell(ct::PRODUCER, dx, dy, 0);
            assert_eq!(
                a.perimeter,
                a.recompute_perimeter(),
                "加 ({dx},{dy}) 之后增量值就不对了"
            );
        }
        // 删除走的是 `recompute()`，也要对
        while a.cells.len() > 1 {
            let (dx, dy) = {
                let c = a.cells.iter().find(|c| c.dx != 0 || c.dy != 0).unwrap();
                (c.dx, c.dy)
            };
            a.remove_cell_at(dx, dy);
            assert_eq!(a.perimeter, a.recompute_perimeter());
        }
    }

    /// 体表/体积比：紧凑比细长低，格数越多越低，单格恒为 4。
    #[test]
    fn exposure_falls_with_size_and_compactness() {
        let one = body(&[(0, 0)]);
        assert!((one.exposure() - 1.0).abs() < 1e-12, "单格必须归一化成 1.0");

        // 3 格直线：20 / 24 = 0.833
        let line3 = body(&[(0, 0), (1, 0), (2, 0)]);
        assert!((line3.exposure() - 20.0 / 24.0).abs() < 1e-12);

        // 9 格：3×3 方块 12/9 = 1.33，直线 20/9 = 2.22
        let mut sq = Anatomy::default();
        for dy in 0..3 {
            for dx in 0..3 {
                sq.add_cell(ct::PRODUCER, dx, dy, 0);
            }
        }
        let mut line9 = Anatomy::default();
        for dx in 0..9 {
            line9.add_cell(ct::PRODUCER, dx, 0, 0);
        }
        assert!(
            sq.exposure() < line9.exposure(),
            "同样 9 格，方块必须比直线更\"厚实\"：{} vs {}",
            sq.exposure(),
            line9.exposure()
        );
        assert!(
            sq.exposure() < one.exposure(),
            "大而紧凑的身体体表/体积比必须更低"
        );
    }

    /// 空身体的暴露面是 0（不能除以零）
    #[test]
    fn empty_body_has_zero_exposure() {
        let a = Anatomy::default();
        assert_eq!(a.perimeter, 0);
        assert_eq!(a.exposure(), 0.0);
    }
}
