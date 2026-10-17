//! 可复现的伪随机数生成器（xorshift128）。
//!
//! 与 JS 版 `src/core/RNG.js` **位级等价**：同一 seed、同一调用序列，
//! 产出的 u32 / 浮点 / 高斯序列完全相同。这是黄金快照对照的基础。
//!
//! 关键点：
//!   · 种子扩展用 splitmix32，四个状态字
//!   · `next_u32` 是 xorshift128（与 JS 的位运算语义一致，全部按 u32 回绕）
//!   · `float()` = u32 / 2^32（f64 除法，与 JS 相同）
//!   · `int(n)` = trunc(float * n)（JS 的 `| 0` 是向零截断）
//!   · `gaussian` 用 Box-Muller，带 spare 缓存——**调用次数会影响序列**，
//!     所以 spare 的存取顺序必须和 JS 一模一样

use serde::{Deserialize, Serialize};

/// splitmix32：把单个 32 位种子扩展成 4 个状态字
fn splitmix32(seed: u32) -> impl FnMut() -> u32 {
    let mut a = seed;
    move || {
        a = a.wrapping_add(0x9e37_79b9);
        let mut t = a ^ (a >> 16);
        t = t.wrapping_mul(0x21f0_aaad);
        t ^= t >> 15;
        t = t.wrapping_mul(0x735a_2d97);
        t ^ (t >> 15)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Rng {
    seed: u32,
    a: u32,
    b: u32,
    c: u32,
    d: u32,
    /// 高斯分布的第二个样本（Box-Muller 一次生成两个）
    spare: Option<f64>,
}

impl Rng {
    pub fn new(seed: u32) -> Self {
        let mut r = Rng {
            seed: 0,
            a: 0,
            b: 0,
            c: 0,
            d: 0,
            spare: None,
        };
        r.reseed(seed);
        r
    }

    pub fn reseed(&mut self, seed: u32) {
        self.seed = if seed == 0 { 1 } else { seed };
        let mut mix = splitmix32(self.seed);
        self.a = mix();
        self.b = mix();
        self.c = mix();
        self.d = mix();
        if (self.a | self.b | self.c | self.d) == 0 {
            self.a = 1;
        }
        self.spare = None;
    }

    pub fn seed(&self) -> u32 {
        self.seed
    }

    /// 原始 32 位无符号整数
    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        let mut t = self.d;
        let s = self.a;
        self.d = self.c;
        self.c = self.b;
        self.b = s;
        t ^= t.wrapping_shl(11);
        t ^= t.wrapping_shr(8);
        self.a = t ^ s ^ (s.wrapping_shr(19));
        self.a
    }

    /// [0, 1)
    #[inline]
    pub fn float(&mut self) -> f64 {
        self.next_u32() as f64 / 4_294_967_296.0
    }

    /// [0, n) 整数（与 JS 的 `(float() * n) | 0` 一致：向零截断）
    #[inline]
    pub fn int(&mut self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        let v = self.float() * n as f64;
        // JS 的 |0 是 int32 截断；n 在我们的用法里远小于 2^31
        (v as i64).clamp(0, n as i64 - 1) as usize
    }

    /// [min, max)
    #[inline]
    pub fn range(&mut self, min: f64, max: f64) -> f64 {
        min + self.float() * (max - min)
    }

    /// [min, max] 整数
    #[inline]
    pub fn int_range(&mut self, min: i64, max: i64) -> i64 {
        min + self.int((max - min + 1) as usize) as i64
    }

    /// 以 prob% 的概率返回 true
    #[inline]
    pub fn chance(&mut self, prob_percent: f64) -> bool {
        self.float() * 100.0 < prob_percent
    }

    /// 以 probability（0-1）的概率返回 true
    #[inline]
    pub fn bernoulli(&mut self, probability: f64) -> bool {
        self.float() < probability
    }

    /// 从切片里随机取一个元素（空切片返回 None）
    #[inline]
    pub fn pick<'a, T>(&mut self, arr: &'a [T]) -> Option<&'a T> {
        if arr.is_empty() {
            return None;
        }
        arr.get(self.int(arr.len()))
    }

    /// 标准正态分布（Box-Muller，带缓存）—— 与 JS 逐位一致
    pub fn gaussian(&mut self, mean: f64, sigma: f64) -> f64 {
        if let Some(v) = self.spare.take() {
            return mean + sigma * v;
        }
        let mut u;
        let mut v;
        let mut s;
        loop {
            u = self.float() * 2.0 - 1.0;
            v = self.float() * 2.0 - 1.0;
            s = u * u + v * v;
            if !(s >= 1.0 || s == 0.0) {
                break;
            }
        }
        let mul = ((-2.0 * crate::jsmath::log(s)) / s).sqrt();
        self.spare = Some(v * mul);
        mean + sigma * u * mul
    }

    /// 原地洗牌（Fisher-Yates），与 JS 的调用顺序一致
    pub fn shuffle<T>(&mut self, arr: &mut [T]) {
        if arr.len() < 2 {
            return;
        }
        let mut i = arr.len() - 1;
        while i > 0 {
            let j = self.int(i + 1);
            arr.swap(i, j);
            i -= 1;
        }
    }

    /// 导出状态（存档用）
    pub fn state(&self) -> [u32; 4] {
        [self.a, self.b, self.c, self.d]
    }

    pub fn set_state(&mut self, seed: u32, st: [u32; 4]) {
        self.seed = seed;
        self.a = st[0];
        self.b = st[1];
        self.c = st[2];
        self.d = st[3];
        self.spare = None;
    }
}
