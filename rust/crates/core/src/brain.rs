//! 神经网络 —— 基因组 B。与 JS 版 `src/life/Brain.js` 对应。
//!
//! 固定拓扑：40 → 12 → 5，只变异权重，不改拓扑。
//!
//! **循环连接（批次 37）**：`recurrent = true` 时隐藏层多一项
//! `Σ w_rec[j][k] · hidden_prev[k]`，即上一 tick 的隐藏层参与这一 tick 的计算。
//! 这是给网络一个**记忆**——没有它，个体每 tick 只看当前格，分不清
//! 「我在新地方」和「我在原地打转」（批次 36 的结论）。
//! 关掉时 `w_rec` 是空 `Vec`，`forward` 走原来的路径，**逐位等于旧行为**。
//! 权重用 f32（对应 JS 的 Float32Array），但**中间求和用 f64**——
//! JS 的 `sum += a * b` 也是 f64 累加，这样两边才会一致。

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::jsmath;
use crate::rng::Rng;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Brain {
    pub in_size: usize,
    pub hidden_size: usize,
    pub out_size: usize,
    pub w1: Vec<f32>, // in × hidden（按隐藏神经元分组：[j*in + i]）
    pub b1: Vec<f32>,
    pub w2: Vec<f32>, // hidden × out（[j*out + k]）
    pub b2: Vec<f32>,
    pub hidden: Vec<f32>,
    pub output: Vec<f32>,
    /// 循环权重（hidden × hidden，`[j*hidden + k]`）。关掉循环时是空 `Vec`。
    #[serde(default)]
    pub w_rec: Vec<f32>,
    /// 上一 tick 的隐藏层（循环用）。
    #[serde(default)]
    pub hidden_prev: Vec<f32>,
    /// 循环权重的初始化量级（从 config 抄进来，`randomize` 用）。
    #[serde(default)]
    pub rec_scale: f32,
}

impl Brain {
    pub fn new(cfg: &Config, rng: Option<&mut Rng>) -> Self {
        let b = &cfg.brain;
        let mut brain = Brain {
            in_size: b.in_size,
            hidden_size: b.hidden_size,
            out_size: b.out_size,
            w1: vec![0.0; b.in_size * b.hidden_size],
            b1: vec![0.0; b.hidden_size],
            w2: vec![0.0; b.hidden_size * b.out_size],
            b2: vec![0.0; b.out_size],
            hidden: vec![0.0; b.hidden_size],
            output: vec![0.0; b.out_size],
            // 关掉循环时留空：`forward` 走旧路径，随机流也完全不碰。
            w_rec: if b.recurrent {
                vec![0.0; b.hidden_size * b.hidden_size]
            } else {
                Vec::new()
            },
            hidden_prev: if b.recurrent {
                vec![0.0; b.hidden_size]
            } else {
                Vec::new()
            },
            rec_scale: b.recurrent_scale as f32,
        };
        if let Some(rng) = rng {
            brain.randomize(rng);
        }
        brain
    }

    pub fn weight_count(&self) -> usize {
        self.w1.len() + self.b1.len() + self.w2.len() + self.b2.len() + self.w_rec.len()
    }

    /// 循环是否开着（`w_rec` 非空）。
    #[inline]
    pub fn is_recurrent(&self) -> bool {
        !self.w_rec.is_empty()
    }

    /// 随机初始化（He 量级；b2 用 σ=0.5，保证没有眼睛的个体也有个体差异）
    pub fn randomize(&mut self, rng: &mut Rng) {
        let s1 = (1.0 / self.in_size as f64).sqrt();
        let s2 = (1.0 / self.hidden_size as f64).sqrt();

        for i in 0..self.w1.len() {
            self.w1[i] = rng.gaussian(0.0, s1) as f32;
        }
        for i in 0..self.b1.len() {
            self.b1[i] = 0.0;
        }
        for i in 0..self.w2.len() {
            self.w2[i] = rng.gaussian(0.0, s2) as f32;
        }
        for i in 0..self.b2.len() {
            self.b2[i] = rng.gaussian(0.0, 0.5) as f32;
        }
        // 循环权重单独一个小量级：太大 tanh 直接饱和，记忆反而变成常数。
        let s_rec = self.rec_scale as f64;
        for i in 0..self.w_rec.len() {
            self.w_rec[i] = rng.gaussian(0.0, s_rec) as f32;
        }
    }

    /// 前向传播：h = tanh(W1·in + b1 [+ Wrec·h_prev])，o = W2·h + b2（输出层线性）
    pub fn forward(&mut self, inputs: &[f32]) {
        let in_size = self.in_size;
        let hid = self.hidden_size;
        let out_n = self.out_size;
        let n = inputs.len().min(in_size);

        // 循环：先把上一 tick 的隐藏层存下来（下面会覆写 `hidden`）。
        let recurrent = self.is_recurrent();
        if recurrent {
            self.hidden_prev.copy_from_slice(&self.hidden);
        }

        for j in 0..hid {
            let mut sum = self.b1[j] as f64;
            let base = j * in_size;
            for i in 0..n {
                sum += inputs[i] as f64 * self.w1[base + i] as f64;
            }
            if recurrent {
                let rbase = j * hid;
                for k in 0..hid {
                    sum += self.hidden_prev[k] as f64 * self.w_rec[rbase + k] as f64;
                }
            }
            self.hidden[j] = jsmath::tanh(sum) as f32;
        }

        for k in 0..out_n {
            let mut sum = self.b2[k] as f64;
            for j in 0..hid {
                sum += self.hidden[j] as f64 * self.w2[j * out_n + k] as f64;
            }
            self.output[k] = sum as f32;
        }
    }

    pub fn copy_from(&mut self, other: &Brain) -> bool {
        if other.in_size != self.in_size
            || other.hidden_size != self.hidden_size
            || other.out_size != self.out_size
        {
            return false;
        }
        self.w1.copy_from_slice(&other.w1);
        self.b1.copy_from_slice(&other.b1);
        self.w2.copy_from_slice(&other.w2);
        self.b2.copy_from_slice(&other.b2);
        if self.w_rec.len() == other.w_rec.len() {
            self.w_rec.copy_from_slice(&other.w_rec);
        }
        true
    }

    pub fn strongest_weight(&self) -> f32 {
        let mut best = 0.0f32;
        for arr in [&self.w1, &self.w2] {
            for &v in arr.iter() {
                let a = v.abs();
                if a > best {
                    best = a;
                }
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brain(recurrent: bool) -> Brain {
        let mut cfg = Config::default();
        cfg.brain.recurrent = recurrent;
        let mut rng = Rng::new(7);
        Brain::new(&cfg, Some(&mut rng))
    }

    fn inputs(seed: f32) -> Vec<f32> {
        (0..40).map(|i| ((i as f32) * 0.1 + seed).sin()).collect()
    }

    /// 关掉循环：同一输入永远给同一输出（没有记忆），而且 `w_rec` 是空的。
    #[test]
    fn recurrent_off_is_memoryless() {
        let mut b = brain(false);
        assert!(b.w_rec.is_empty(), "关掉循环时不该分配循环权重");

        let x = inputs(0.0);
        b.forward(&x);
        let first = b.output.clone();
        // 中间喂点别的，再喂回来：关掉循环时应该完全一致
        b.forward(&inputs(3.0));
        b.forward(&x);
        assert_eq!(first, b.output, "没有循环就不该有记忆");
    }

    /// 打开循环：同样的输入，历史不同则输出不同（这就是"记忆"）。
    #[test]
    fn recurrent_on_remembers() {
        let mut b = brain(true);
        assert_eq!(b.w_rec.len(), 12 * 12, "循环权重是 hidden × hidden");

        let x = inputs(0.0);
        b.forward(&x);
        let first = b.output.clone();

        // 换一段历史再回来
        b.forward(&inputs(3.0));
        b.forward(&x);
        assert_ne!(first, b.output, "有循环时历史应该影响输出");
    }

    /// `copy_from` 要连循环权重一起抄——不然芽的大脑会"断片"。
    #[test]
    fn copy_from_copies_recurrent_weights() {
        let mut a = brain(true);
        let mut b = brain(true);
        assert!(a.copy_from(&b));
        assert_eq!(a.w_rec, b.w_rec);
    }
}
