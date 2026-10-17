//! 神经网络 —— 基因组 B。与 JS 版 `src/life/Brain.js` 对应。
//!
//! 固定拓扑前馈：40 → 12 → 5，只变异权重，不改拓扑。
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
        };
        if let Some(rng) = rng {
            brain.randomize(rng);
        }
        brain
    }

    pub fn weight_count(&self) -> usize {
        self.w1.len() + self.b1.len() + self.w2.len() + self.b2.len()
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
    }

    /// 前向传播：h = tanh(W1·in + b1)，o = W2·h + b2（输出层线性）
    pub fn forward(&mut self, inputs: &[f32]) {
        let in_size = self.in_size;
        let hid = self.hidden_size;
        let out_n = self.out_size;
        let n = inputs.len().min(in_size);

        for j in 0..hid {
            let mut sum = self.b1[j] as f64;
            let base = j * in_size;
            for i in 0..n {
                sum += inputs[i] as f64 * self.w1[base + i] as f64;
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
