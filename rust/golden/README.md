# golden —— 只剩 RNG 基线

## 现状（批次 7 起）

**模拟行为的黄金快照已废弃。** 原来这里有 `world.txt`（3 个种子 × 6 个 tick 点的状态摘要）
和 `gauss.txt`，用来断言 Rust 版与 JS 版逐位一致。那是 1.x → 2.0 迁移期的质量保证，
现在 2.0 要改模拟本身（光场、能量守恒、卵生），逐位对齐只会挡路，已删除。

替代品是 `crates/core/tests/regression.rs` 里的**特征化测试**：跑固定 tick 数，
断言种群 / 体型 / 光场梯度落在合理区间。它抓不到"第 37 tick 多算了一点"，
但能抓到"改完生态崩了"。

## 保留 `rng-u32.txt`

seed=12345 的前 200 个 `nextUint32()`（8 位十六进制，每行一个）。

**RNG 不是模拟行为，是确定性基础。** 它的序列一漂，所有存档重放、实验复现、
A/B 对照全部失效——和"要不要对齐 1.x"无关，所以留着。

`regression.rs::rng_matches_js_sequence` 校验前 8 个。

## 关于高斯样本的历史注记

`gauss.txt` 曾经有 6/200 个样本和 JS 对不上：`Math.log`（V8 的 fdlibm 移植）
与 Rust `f64::ln`（系统 libm）在个别输入上差 1–2 ulp。当时不影响模拟，因为
大脑权重最终存成 f32，1–2 ulp 的 f64 差异被 f32 舍入吸收。

这个文件已随黄金快照一起删除。**如果以后真的需要 bit 级复现**，
把 `crates/core/src/jsmath.rs` 换成 fdlibm 移植版即可。
