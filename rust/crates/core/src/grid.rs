//! 网格数据结构 —— 与 JS 版 `src/core/GridMap.js` 对应。
//!
//!   types  : Vec<u8>    顶层类型（可能被生物的身体盖住）
//!   owners : Vec<i32>   所属生物 id，-1 = 独立格
//!   floor  : Vec<u8>    地板层：这一格的地形是什么
//!   food   : Vec<f64>   **食物格里的能量**（只有 types == FOOD 时有意义）
//!
//! 为什么要有 floor：生物写格子会把 types 覆盖成自己的细胞类型。
//! 地形只存 types 的话，一只生物从岩浆上走过就把岩浆抹掉了。
//! 地形单独记在 floor，生物离开时 `World::reveal_floor` 把它铺回顶层。

use serde::{Deserialize, Serialize};

use crate::cell_type::{ct, TERRAIN_MASK};

#[derive(Clone, Serialize, Deserialize)]
pub struct Grid {
    pub cols: usize,
    pub rows: usize,
    pub wrap: bool,
    pub len: usize,
    pub types: Vec<u8>,
    pub owners: Vec<i32>,
    pub floor: Vec<u8>,
    /// 每格食物里还剩多少能量（批次 8）。
    ///
    /// 以前食物是**二值**的：一格 FOOD 就是一个固定值，表达不了"吃了一半"，
    /// 也没法做渐进腐败。现在食物带存量，腐败是能量一点点流失，
    /// 尸体则携带死者生前的能量（见 `World::kill_organism`）。
    ///
    /// 只有 `types[i] == FOOD` 时这个值才有意义；其他格子恒为 0。
    pub food: Vec<f64>,
}

impl Grid {
    pub fn new(cols: usize, rows: usize, wrap: bool) -> Self {
        let len = cols * rows;
        Grid {
            cols,
            rows,
            wrap,
            len,
            types: vec![ct::EMPTY; len],
            owners: vec![-1; len],
            floor: vec![ct::EMPTY; len],
            food: vec![0.0; len],
        }
    }

    /// 世界坐标 → 扁平索引；越界且不环绕时返回 None
    #[inline]
    pub fn index(&self, col: i32, row: i32) -> Option<usize> {
        let (c, r) = if self.wrap {
            (
                col.rem_euclid(self.cols as i32),
                row.rem_euclid(self.rows as i32),
            )
        } else {
            if col < 0 || row < 0 || col >= self.cols as i32 || row >= self.rows as i32 {
                return None;
            }
            (col, row)
        };
        Some(r as usize * self.cols + c as usize)
    }

    /// 扁平索引 → (col, row)
    #[inline]
    pub fn coords(&self, index: usize) -> (i32, i32) {
        ((index % self.cols) as i32, (index / self.cols) as i32)
    }

    #[inline]
    pub fn type_at(&self, col: i32, row: i32) -> u8 {
        match self.index(col, row) {
            Some(i) => self.types[i],
            None => ct::EMPTY,
        }
    }

    #[inline]
    pub fn owner_at(&self, col: i32, row: i32) -> i32 {
        match self.index(col, row) {
            Some(i) => self.owners[i],
            None => -1,
        }
    }

    /// 整张网格填成同一类型。`keep_terrain` 为真时保留 floor 层里的地形。
    pub fn fill(&mut self, ty: u8, keep_terrain: bool) {
        // 无论哪条路径，食物能量都要清掉：填完之后那些格子已经不是食物了
        self.food.fill(0.0);
        if !keep_terrain {
            self.types.fill(ty);
            self.owners.fill(-1);
            let floor_ty = if TERRAIN_MASK[ty as usize] { ty } else { ct::EMPTY };
            self.floor.fill(floor_ty);
            return;
        }
        for i in 0..self.len {
            let f = self.floor[i];
            self.types[i] = if TERRAIN_MASK[f as usize] { f } else { ty };
            self.owners[i] = -1;
        }
    }

    /// 这一格食物还剩多少能量（非食物格恒为 0）
    #[inline]
    pub fn food_at(&self, col: i32, row: i32) -> f64 {
        self.index(col, row).map(|i| self.food[i]).unwrap_or(0.0)
    }

    /// 统计各类型格子数量
    pub fn count_by_type(&self) -> [u32; 32] {
        let mut counts = [0u32; 32];
        for &t in &self.types {
            counts[(t as usize).min(31)] += 1;
        }
        counts
    }

    pub fn count_non_empty(&self) -> usize {
        self.types.iter().filter(|&&t| t != ct::EMPTY).count()
    }
}
