# evosim · Rust 版（原生窗口）

JS 版（仓库根目录）已**冻结**，只作为参照实现与黄金快照来源。
新代码在这里，目标是"TPT 那样双击就能跑的原生程序"。

## 目录

```
rust/
  Cargo.toml                 工作区
  crates/core/               模拟内核（零窗口依赖，可在 cargo test 里跑）
    src/rng.rs               xorshift128，与 JS 逐位一致
    src/jsmath.rs            log / tanh（与 JS Math 对齐的边界层）
    src/cell_type.rs         16 种细胞/地形注册表（含中文简介）
    src/config.rs            参数表
    src/grid.rs              types / owners / floor 三层
    src/anatomy.rs           身体蓝图 + 结构指纹（FNV-1a）
    src/brain.rs             40→12→5 前馈网络
    src/sensors.rs           40 维感知编码
    src/organism.rs          个体与派生量（纯计算）
    src/mutation.rs          元变异 / 结构变异 / 权重变异
    src/species.rs           物种表与剪枝
    src/world.rs             两阶段主循环、食物/地形索引、伤害与繁殖
    src/factory.rs           生命起源 / 随机生物 / 投放
    src/worldgen.rs          示例世界生成
    tests/golden.rs          黄金快照回归（与 JS 逐位对照）
  crates/cli/                无界面 CLI（对照、基准、调试）
  golden/                    JS 版导出的基线数据
```

## 常用命令

```bash
cd rust
cargo build --release                     # 构建
cargo test --release                      # 单元测试 + 黄金快照回归
cargo run --release --bin evosim -- --sim --seed 1001 --ticks 2000
```

CLI 参数：

| 参数 | 作用 |
|---|---|
| `--sim --seed S --ticks N` | 跑 N tick，打印检查点（1/10/50/100/200/500） |
| `--every` | 每个 tick 打一行（定位偏差用） |
| `--dump-orgs T` | 跑到第 T tick，逐个体导出 |
| `--dump-rng N` / `--dump-gauss N` | 导出 RNG / 高斯序列（与 JS 对照） |
| `--pop N` | 初始随机种群数量（默认 8） |

## 与 JS 版的一致性怎么保证

**核心思路：黄金快照。** `golden/world.txt` 是 JS 版导出的"同种子、同 tick 的状态摘要"，
Rust 版必须复现出完全相同的摘要。

摘要算法（两边逐字对应）：

```
FNV-1a 依次混入：
  网格：types[i], owners[i], floor[i]  （每个格）
  个体：id, col, row, rotation, direction, age,
        round(energy*1000), damage, 细胞数，以及每个细胞的 type/dx/dy/dir
```

覆盖范围：**5 个种子 × 2000 tick × 每 50 tick 一个检查点 = 200 个检查点全一致**。

要守住这条，需要三个前提，缺一不可：

1. **RNG 逐位一致**（`rng-u32.txt`：200 个 u32 全等）
2. **RNG 消耗顺序一致** —— 这是最容易踩的坑：JS 的 `new Organism()` 会先用 RNG 随机化一整个大脑
   （545 个高斯数）再被 `copyFrom` 覆盖掉；Rust 版也必须浪费这 545 次，否则整条随机序列错位
3. **数组顺序一致** —— 阶段 B 的结算顺序是"对个体数组洗一次牌"，
   所以压缩死亡个体时**必须稳定压缩**（`Vec::retain`），不能 swap-remove

> 浮点：`Math.log` 与 Rust `f64::ln` 在个别输入上差 1–2 ulp（实测 200 个高斯样本里有 6 个不同）。
> 但大脑权重与激活值最终都存成 f32，这点差异被 f32 的舍入吸收掉了 —— 所以状态摘要仍然全等。
> 如果将来出现真正的分歧，`jsmath.rs` 换成 fdlibm 移植版即可（调用点不用改）。

## 性能

满载 1530 个个体、200×150 世界：

| 版本 | step 耗时 |
|---|---|
| JS 版（Chrome） | 2.0 ms/tick |
| Rust 单线程（release） | 0.453 ms/tick |
| **Rust + rayon 并行 `think()`** | **0.210 ms/tick（≈9.5×）** |

并行是安全的：`think()` 只读世界、只写个体自己的计划与缓冲，**且完全不碰 RNG**，
所以 200 个检查点的摘要与串行版逐位相同。

## 里程碑

- [x] **M0** 工作区 + 黄金快照导出
- [x] **M1** 内核移植 + 200 个检查点全一致
- [ ] **M2** winit 窗口 + wgpu 渲染（R8 索引 + 256 色 LUT，NEAREST 采样）
- [ ] **M3** egui 骨架 + 自绘控件（滑块/笔刷/曲线/物种树/工坊）
- [ ] **M4** 每格温度/辐射场 + 图层滤镜 + 悬停卡片
- [ ] **M5** 打包单 exe（内嵌中文字体子集）
