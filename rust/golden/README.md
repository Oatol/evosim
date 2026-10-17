# golden —— JS 版导出的对照基线

这些文件由 `node tools/export-golden.mjs` 生成，**不要手改**。
Rust 版的行为必须与它们逐位一致（`cargo test` 里的 `tests/golden.rs` 会检查）。

| 文件 | 内容 |
|---|---|
| `rng-u32.txt` | seed=12345 的前 200 个 `nextUint32()`（8 位十六进制，每行一个） |
| `gauss.txt` | 同种子的前 200 个 `gaussian(0,1)`（f64 位模式，16 位十六进制） |
| `world.txt` | 3 个种子（1001/4242/7777）在 1/10/50/100/200/500 tick 的状态摘要 |

`world.txt` 的一行：

```
seed 1001 tick 50 orgs 10 food 336 species 9 body 40 digest 5f067ca6
```

- `orgs` 个体数 · `food` 食物格数 · `species` 物种数 · `body` 身体格总数
- `digest` = FNV-1a 摘要，混入网格三层与全部个体字段（算法见 `rust/README.md`）

## 为什么高斯样本有 6 个对不上，摘要却全等

`Math.log`（V8 的 fdlibm 移植）与 Rust `f64::ln`（系统 libm）在个别输入上差 1–2 ulp。
200 个样本里 6 个不同，但：

- 大脑权重最终存成 **f32**，1–2 ulp 的 f64 差异被 f32 舍入吸收
- 实测 5 个种子 × 2000 tick，状态摘要**全部一致**

如果哪天出现真正的分歧，把 `crates/core/src/jsmath.rs` 换成 fdlibm 移植版即可。

## 重新生成

```bash
node tools/export-golden.mjs          # 重写这三份文件
cd rust && cargo test --release       # 立刻验证 Rust 是否还跟得上
node tools/parity-check.mjs --ticks 2000   # 更大范围（5 种子 × 2000 tick）
```
