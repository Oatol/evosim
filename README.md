# evosim 2.0

基于网格细胞自动机的开放式进化沙盒 —— **Rust 原生重写**。

不设适应度函数：每个个体 = **可共同进化的身体蓝图**（基因组 A）+ **神经网络大脑**（基因组 B，40→12→5）。
行为完全由自然选择塑造，物种 = 身体结构指纹（FNV-1a 哈希），权重变异不产生新物种。

## 仓库结构（两条线）

| 目录 | 内容 | 状态 |
|---|---|---|
| **`rust/`** | **2.0 主线**：Rust 内核 + wgpu 渲染 + 原生窗口 | 开发中 |
| 根目录（`src/` `styles/` `tools/` `dist/`） | 1.x：纯原生 JavaScript 实现（浏览器 / 单文件 HTML / Node SEA exe） | **冻结**，只作参照实现与黄金快照来源 |

1.x 的完整历史在 [Oatol/evosim](https://github.com/Oatol/evosim)。

> 为什么 2.0 仓库里还留着 1.x：**黄金快照**（`rust/golden/`）由 1.x 导出，
> Rust 版必须复现出一模一样的世界状态（见 `rust/README.md` 的"一致性"一节）。
> 留着参照实现，才能随时重新生成基线、定位行为漂移。

## 快速开始（Rust 版）

```bash
cd rust
cargo test --release                                  # 单元测试 + 黄金快照回归
cargo run --release --bin evosim -- --sim --seed 1001 --ticks 2000
```

需要 Rust 1.80+（本机实测 1.99.1，`x86_64-pc-windows-msvc`）。

## 现状

| 里程碑 | 内容 | 状态 |
|---|---|---|
| M0 | Cargo 工作区 + 黄金快照导出 | ✅ |
| M1 | 内核移植（14 模块）+ **200 个检查点与 1.x 逐位一致** | ✅ |
| M2 | winit 窗口 + wgpu 渲染（R8 索引 + 256 色 LUT，NEAREST） | 进行中 |
| M3 | egui 骨架 + 自绘控件（参数 / 检视器 / 物种树 / 工坊 / 曲线） | 待做 |
| M4 | 每格温度 / 辐射场 + 图层滤镜 + 悬停卡片 | 待做 |
| M5 | 单 exe 打包（内嵌中文字体子集） | 待做 |

性能（满载 1530 个体、200×150 世界）：1.x 2.0 ms/tick → **Rust 0.21 ms/tick（≈9.5×）**。

## 1.x 的交付物（仍可直接玩）

- `dist/evosim.html` —— 单文件，双击即玩
- `dist/evosim.exe` —— Windows 可执行（Node SEA），源码在 `tools/build-exe.mjs`

## 设计文档

- `DESIGN.md` —— 1.x 的完整设计（网格 / 细胞注册表 / 大脑 / 变异 / 物种 / 渲染 / 存档 / 打包）
- `rust/README.md` —— 2.0 的架构、一致性机制、里程碑

## 许可

个人项目，未指定开源协议；如需引用请先联系作者。
