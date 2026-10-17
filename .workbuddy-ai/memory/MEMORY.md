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
- **原生窗口没有 CDP，UI 验证只能靠截图**：① 按 `GetClientRect` + `ClientToScreen` **裁客户区**，
  图像像素才等于 egui 逻辑像素；② 点击前**必须先点一下顶栏**让窗口拿到焦点，否则 `keybd_event`
  （空格/快捷键）全不生效；③ **绝不要 `ShowWindow(SW_RESTORE)`**，它会把窗口从最大化还原、
  位置一变所有按截图算的坐标全废；④ **Read 显示图片时会缩放**（1600 宽显示成 1080），
  照显示图估坐标会差 1.5 倍 —— 最可靠的是「移动鼠标 → 截图读右下角 hover 读数」反推格子映射。
  现成脚本：`tools/shot.ps1 -Out x.png [-Crop x,y,w,h] [-Scale n]`（`Scale` 走最近邻，
  验 1px 线条必须放大看）、`tools/click.ps1 -Points "x,y;x,y"`（相对客户区）。
  **内联 `Add-Type` 会被安全策略拦**，所以只能落盘成 .ps1 再 `powershell -File` 跑。
- **顶栏坐标（1600×950 客户区，`right_to_left` 从右往左排）**：世界 1573 · 物种工坊 1517 ·
  工具 1461 · 参数 1417 · 细胞 1373 · 适应窗口 1317。猜错一次就会误关邻居窗口。
- **egui 的 Slider 会在绘制时 clamp/吸附值**：`step_by` 大于默认值量级会把值吸附成 0
  （消耗 0.006 配 0.05 步长 → 0），`hi` 远大于真实取值域会让拖柄永远趴在最左端。
  两个都表现为「打开窗口什么都没碰，配置就变了」。必须「step 贴合量级 + 量程贴合取值域 +
  只在 `vals != before` 时写回」，并且用测试守住「量程盖住所有出厂值」。
- **滑块轨道在 egui 0.31 里取 `widgets.inactive.bg_fill`**（全库只有滑块用它，按钮走
  `weak_bg_fill`），我们的面板底色只比它深 6 级灰度 → 轨道隐形、滑块看着像坏了。
  提亮这一项 + `visuals.slider_trailing_fill = true` 一起用，值贴近 0 时才读得出位置。
- **`Slider::text()` 把名字排在数值框之后**（「滑块 1.0 血量」）。要「名字在左、滑块对齐」
  得用 `Grid` 两列，别指望 `text()`。

## 操作映射

空格 暂停 · S 单步 · `[` `]` 调速 · Tab 着色（类型/物种/温度/辐射）· F 适应 · G 工坊 · I 检视 · Esc 退出。
左键 = 当前工具（放置 > 画笔/检视/平移）· 右键或中键拖 = 平移 · 滚轮 = 以光标为锚缩放。

## 验证与打包

- `cargo test --release`（当前 65 项：app 29 / core 26 / golden 2 / render 6 / colors 2）
- `node tools/parity-check.mjs`（对 JS 基线）
- 打包：`node rust/tools/package.mjs` → `dist/evosim2.exe`（~7.03 MB）
- 截图/压测：`evosim-app --screenshot out.png --ticks N [--mode temp|rad]`、`--bench --seconds 4 --speed 50`
- **编译必须零警告**：`Stroke::new(1.0, ..)` 这类浮点字面量会回退到 `f32`
  （`future_incompatible`，未来变硬错误），统一写 `1.0_f32`。

## 进度与下一步

- **M0–M5 完成**：内核对齐、wgpu 渲染、egui 骨架（顶栏/工具栏/参数/悬停）、物种工坊、环境场、高亮修复。
- **批次 0 完成**（`1dbbc3b`）：世界窗口（新建/调尺寸/保存/读取/丢弃二次确认）+ 二进制存档
  （`EVOSIM2` 魔数 + 36 字节定长头 + bincode；`food_cells` 与 `terrain` 索引**按原数组顺序**存）。
- **批次 1 完成**（`4305d98`）：检视器（`app/inspector.rs`）—— 缩略图 / 神经网络 40→12→5 连线图 /
  行为区；`Organism` 加 `last_move_ok` + `energy_gained_last`（不进 `state_digest`）；
  **存档 VERSION → 2**。
- **批次 2 完成**（`fa226f7`）：曲线窗口（`app/chart.rs`）—— 6 指标多选 / 悬停读数 /
  滚轮缩放 / 拖动平移；采样改为**自适应抽稀**（满 1200 点间隔翻倍、隔一个丢一个）。
- **批次 3 完成**（`4bfc955`）：细胞参数（`app/cells.rs`）—— 16 种 × 5 属性，Grid 布局 +
  出厂值小字 + 「N 项被改过」；量程/步长/写回三条守门（详见当日日志）。
- **下一步**：
  4. 谱系树（叠加在时间轴上）
  - 独立项：大体型（体型上限 + 繁殖成本指数 p，先跑 CLI 扫描）
- **⚠️ 待决策**：物种判定粒度过细（长跑后物种数 ≈ 种群数，tick 63487 时 980 只 / 2029 物种）。
  它会让谱系树退化成"每个个体一根枝条"，**建议排在批次 4 前面修**——已向用户提过，尚未定。
