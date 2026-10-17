# evosim 项目约定

网格细胞自动机开放式进化沙盒：生物 = **可共同进化的身体蓝图 + 神经网络大脑**，无适应度函数。
**架构权威文档 = `DESIGN.md`（改动必须同步它）；本文件只记不变量、踩过的坑、下一步。**

## 仓库与冻结政策

- 根目录 = **JS 版 1.x，已冻结**：只作参照实现 + 黄金快照来源，不再加功能、不再发版。
- `rust/` = **2.0 主线**：`crates/core`（模拟内核，零窗口依赖）+ `render`（wgpu）+ `app`（winit + egui）+ `cli`。
- 双远端：`origin` = `Oatol/evosim`（1.x），`v2` = `Oatol/evosim2.0`（2.0）。**每次推送两边都推。**
- 界面路线：egui 只当输入路由/文本/滚动/IME 的底座；滑块/笔刷/色块/曲线/物种树/工坊网格用 `painter` 自绘。
- 存档：**全新二进制格式，不兼容 1.x**（用户已批准断代）。

## 不变量（改代码前必看）

**核心循环**
- `Grid` 三层：`types`(u8) / `owners`(i32) / **`floor`(u8)**。地形存 floor，生物盖上去只改 types，
  离开时 `reveal_floor` 铺回。**地形效果必须读 floor**——生物自己的细胞就压在枢轴格上。
- 主循环**两阶段**：A 感知+决策（只读）→ B 随机顺序结算（写世界）。
- **`CELL_DEFS` 下标必须等于 id**（有 `order_tests` 守着）；细胞 id 定下不能改。
- 压缩死亡个体必须**稳定**（`Vec::retain`，不能用 swap-remove，否则洗牌序列错位）。
- 构造 `Organism` 必须照抄 JS 的 545 次高斯调用（虽然随后被 copy 覆盖），否则 RNG 序列错位。
- **食物索引顺序即演化状态**：腐败按 `food_cells` **数组位置**抽样 → 存/读档必须保持原顺序，
  读档**不能走 `init_food_index()`**（那会按网格升序重建）。
- 环境场（温度/辐射）**与 tick 解耦**：主程序按墙钟 60 Hz 调 `update_fields()`，不在 `step()` 里。
  这样 50× 倍速不烧 CPU，且场不影响黄金快照。

## 一致性守门

`tools/export-golden.mjs` 导出 JS 基线 → `tools/parity-check.mjs` + `cargo test`（`tests/golden.rs`）。
当前 **5 种子 × 2000 tick × 每 50 tick = 200 检查点逐位一致**。摘要混合网格三层 + 个体全字段。
`Math.log` vs `ln` 有 1–2 ulp 差（200 高斯样本差 6 个），但 f32 存储吸收 → 摘要仍全等。

## 性能基线（讨论性能前先看这张表）

| 项 | 实测 |
|---|---|
| JS 版（Chrome，1517 只） | 2.0 ms/tick |
| Rust 单线程 | 0.803 ms/tick |
| **Rust + rayon（物理核 8 线程，fat LTO）** | **0.279 ms/tick（7.2×）** |
| 50× 倍速（目标 3000 tps） | ~3580 tps（119%） |
| 环境场 update（2 × 30k 格） | 0.154 ms/次 |

- 瓶颈是每只 **~1.3 µs/tick 的 CPU 模拟**（大脑前向占 77%），**不是渲染** → 换 GPU 渲染几乎零收益。
- **测性能必须用 `--bench` 的稳态**：含种群爬升阶段的平均值（0.21 ms）好看但无意义。
- 线程数按**物理核数**（逻辑核一半）：8 线程 0.415，用满逻辑核反而退化到 0.483。

## 踩过的坑（都是真发生过的）

- **egui 窗口里"立刻关闭自己"**：闭包外那个 `open` 会在函数末尾写回，把闭包里的 `false` 覆盖掉。
  要用 `want_close` 标记，末尾 `self.open = open && !want_close`。
- **"一次性显式意图"不该挂在工具上**：手动放置抽成 `left_action(placing, tool)` 纯函数，放置优先于工具。
- **右键统一是"拖动世界画布"**：任何窗口都不吃右键，否则抢走平移还会误触擦除；工坊蓝图格只认左键。
- **egui-winit 对 Tab 无条件 consume**（拿它做焦点轮换）→ 自己的 Tab 快捷键永远不执行、焦点乱跳。
  必须在 `state.on_window_event()` **之前**拦下并 return。
- **egui 想画在窗口下面只能用 `Order::Background`**：`layer_painter()` 不创建图层；同 Order 内
  非 Area 图层永远排在 Area（窗口）之后。这是"高亮浮在工坊/检视器上"的根因（修了两次）。
- **全屏 backdrop 遮罩**：症状是"跟随看不见""关闭按钮不显眼"——改非模态浮窗后一起消失。
- **地形格顶层 `types` 存的就是地形自己** → "能不能往上放食物"的判据是「`EMPTY` **或裸地形**」。
- **改过 UI 文案要重跑 `python rust/tools/subset-font.py`**，否则新字是方块
  （`embedded_font_covers_every_ui_char` 守着）。`hinting=False` 能把 576 KB 砍到 201 KB。
- **缩小时世界被重复平铺** → 逐方向判断平铺 + BACKDROP 底色，只在"世界比视口大"的方向平铺。
- **`SpeciesRegistry::update()` 忘了调** → 物种全空、计数全 0。要在 `step()` 里按 interval 更新。
- **`add_organism()` 会重写 `org.id`** → 测试必须用返回值。
- **egui-wgpu 要求 `RenderPass<'static>`** → `forget_lifetime()`。

## 操作映射

空格 暂停 · S 单步 · `[` `]` 调速 · Tab 着色（类型/物种/温度/辐射）· F 适应 · G 工坊 · I 检视 · Esc 退出。
左键 = 当前工具（放置 > 画笔/检视/平移）· 右键或中键拖 = 平移 · 滚轮 = 以光标为锚缩放。

## 验证与打包

- `cargo test --release`（当前 50 项：app 20 / core 20 / golden 2 / render 6 / colors 2）
- `node tools/parity-check.mjs`（对 JS 基线）
- 打包：`node rust/tools/package.mjs` → `dist/evosim2.exe`（~6.85 MB）
- 截图/压测：`evosim-app --screenshot out.png --ticks N [--mode temp|rad]`、`--bench --seconds 4 --speed 50`

## 进度与下一步

- **M0–M5 完成**：内核对齐、wgpu 渲染、egui 骨架（顶栏/工具栏/参数/悬停）、物种工坊、环境场、高亮修复。
- **下一步顺序（2026-10-17 重排，世界窗口提到最前）**：
  0. **世界窗口 + 存档**：新建（可调尺寸）/ 保存 / 读取 / 丢弃提醒
  1. 检视器增强：缩略图 + 神经网络图 + 行为区
  2. 曲线窗口
  3. 每细胞参数
  4. 谱系树（叠加在时间轴上）
  - 独立项：大体型（体型上限 + 繁殖成本指数 p，先跑 CLI 扫描）
- 排第一的理由：**世界生命周期**（新建/读档会替换 World）必须由所有面板正确处理，
  先做则后续新面板天然写对；否则每加一个面板都要回头改。
