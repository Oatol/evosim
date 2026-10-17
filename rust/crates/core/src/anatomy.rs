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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Anatomy {
    pub cells: Vec<BodyCell>,
    pub birth_distance: i32,
    pub flags: Flags,
}

impl Default for Anatomy {
    fn default() -> Self {
        Anatomy {
            cells: Vec::new(),
            birth_distance: 4,
            flags: Flags::default(),
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
    }

    /// 添加一个细胞。位置已被占用时返回 false。
    pub fn add_cell(&mut self, type_id: u8, dx: i32, dy: i32, dir: u8) -> bool {
        if self.cell_at(dx, dy).is_some() {
            return false;
        }
        self.cells.push(BodyCell::new(type_id, dx, dy, dir));

        let distance = (dy.abs() * 2 + 2).max(dx.abs() * 2 + 2);
        if distance > self.birth_distance {
            self.birth_distance = distance;
        }
        self.apply_flag(type_id, true);
        true
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
