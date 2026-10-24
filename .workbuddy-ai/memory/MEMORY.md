# evosim 项目约定

网格细胞自动机开放式进化沙盒：生物 = **可共同进化的身体蓝图 + 神经网络大脑**，无适应度函数。
**架构权威文档 = `DESIGN.md`（改动必须同步它）；本文件只记不变量、踩过的坑、下一步。**

## 仓库与政策

- 根目录 = **JS 版 1.x，已冻结**（参照实现，不再加功能）。
- `rust/` = **2.0 主线**：`core`（模拟内核）+ `render`（wgpu）+ `app`（winit+egui）+ `cli`。
- 双远端：`origin`=Oatol/evosim，`v2`=Oatol/evosim2.0。**每次两边都推。**
- **golden 快照已放弃，不再维护**；存档可断代（用户已明确授权）。
- 界面：egui 只做输入/文本/滚动/IME 底座，滑块/色块/曲线/树/网格一律 `painter` 自绘。

## 不变量

- `Grid` 三层 `types`/`owners`/`floor`。地形在 floor，生物盖上去只改 types，离开 `reveal_floor` 铺回。
  **地形效果必须读 floor**——生物细胞就压在枢轴格上。
- 主循环两阶段：A 感知+决策（只读）→ B 随机顺序结算（写世界）。
- `CELL_DEFS` 下标 == id（`order_tests` 守着）；细胞 id 定下不能改。
- 死亡压缩必须**稳定**（`Vec::retain`，不能 swap-remove）。
- 食物索引**顺序即演化状态**：腐败按 `food_cells` 数组位置抽样 → 读档不能走 `init_food_index()`。
- 环境场与 tick 解耦：按墙钟 60 Hz 调 `update_fields()`，不在 `step()` 里。
- `add_organism()` 会重写 `org.id` → 测试用返回值。
- 编译**零警告**：浮点字面量写 `1.0_f32`。
- 改过 UI 文案必须重跑 `python rust/tools/subset-font.py`，否则新字是方块
  （`embedded_font_covers_every_ui_char` 守着）。

## 性能基线

JS 2.0 ms/tick · Rust 单线程 0.803 · **Rust+rayon(8 物理核) 0.279（7.2×）** · 50× 倍速 ~3580 tps。
瓶颈是每只 ~1.3 µs/tick 的 CPU 模拟（大脑前向 77%），**不是渲染**。
**测性能只用 `--bench` 稳态**（含种群爬升的平均值无意义）。线程按物理核数，用满逻辑核反而退化。

## 验证与打包

- **【硬约束】验证一律程序化，不碰用户鼠标、不截图。** 用户在用机器，长时间满载会过热蓝屏。
  允许：`cargo test`、CLI（`--screenshot --ticks N` 只看数字、`--bench`）、`examples/*` 诊断。
  禁止：`tools/click.ps1` / `drag.ps1` / `wheel.ps1` 注入输入、长时间跑满 CPU。
- 长跑扫描单次 ≤1 分钟（~15000 tick × 少量组合）。
- `cargo test --release`；打包 `node rust/tools/package.mjs` → `dist/evosim2.exe`。

## 操作映射

空格 暂停 · S 单步 · `[` `]` 调速 · Tab 着色 · F 适应 · G 工坊 · I 检视 · Esc 退出。
左键 = 当前工具（放置 > 画笔/检视/平移）· 右键/中键拖 = 平移 · 滚轮 = 光标锚缩放。

## 踩过的坑

**UI / egui**
- 窗口"立刻关闭自己"：闭包外的 `open` 会覆盖闭包里的 `false` → 用 `want_close` 标记。
- 右键统一是拖画布，任何窗口都不吃右键；工坊蓝图格只认左键。
- egui-winit 对 Tab 无条件 consume → 必须在 `state.on_window_event()` **之前**拦下。
- 想画在窗口下面只能 `Order::Background`；`layer_painter()` 不创建图层。
- Slider 绘制时会 clamp/吸附：step 要贴合量级、hi 要贴合取值域、只在 `vals != before` 时写回；
  轨道取 `widgets.inactive.bg_fill`（全库只有滑块用它），要提亮 + `slider_trailing_fill`。
  `Slider::text()` 把名字排在数值后 → 要"名字在左"用 `Grid` 两列。
- `ScrollArea::show_rows` 行高 = 不含 item_spacing，内容每行必须真是 `行高 + spacing`。
- 拖拽不能在 `dragged()` 里读当前状态当基准（每帧再算一次 → 指数收敛），按下时快照。
- `i - cols + 1` 会先算 `i - cols` → 第一行末格 usize 下溢，写 `i + 1 - cols`。
- 缩小时世界重复平铺 → 逐方向判断 + BACKDROP 底色。
- tools 下 `.ps1` 只用 ASCII 注释（PS 5.1 按 ANSI 读，UTF-8 中文会吞掉换行把 `param()` 读坏）。

**模拟内核**
- 加能量统一走 `World::add_energy`（旧路径不查容量 → energy 超 cap 时凭空蒸发）。
- 重复记账：腐败 / 嘴吃 都必须**先清零 `grid.food[index]` 再 `reveal_floor`**。
- `SpeciesRegistry::size()` 是**累计**，`alive_count` 才是当前；`update()` 要在 `step()` 里按 interval 调。
- `act_organism` 末尾含繁殖判定 → 测"吃了多少能量"要先抬繁殖阈值。
- 遮挡/光场：垂直方向**不能环绕**（`row - d < 0` 直接 break）。
- 默认值 1.0 的倍率乘错地方/压根没乘，编译器和测试都不报错 → 必须调离 1.0 写端到端测试。

## 关键演化结论（都是实测，别再重试已证伪的路线）

- **体型长期不可遗传的根因**：旧代码在 `!grew` 时走"随机长一格"兜底，而成体判据是
  `len >= plan.len() && old_enough` → 成长期窗口里每 tick 白长一格。关掉后成体体型 16.69 → **2.65**。
  **批次 5–44 所有"体型 X 格 / +N%"的数字量的是兜底填充，不是基因。**（批次 45 修正）
- **规模中性**是核心病根：产出/成本/容量/繁殖门槛全线性于体型 → 繁殖速率 0.11/tick 与体型无关，
  唯一非线性项"出生要 n 格连通空地"在**惩罚**大体型。
- 已证伪：`breed_cost_exponent<1`、提高繁殖搜索候选数（反而变小）、Kleiber 成本折扣、
  富贫斑块（有梯度≠有分化）、云影造异养、维持成本造异养、`foodBlocksReproduction` 拆分、
  温度热区体型优势（+5~20% 方向对但幅度小）。
- **有效杠杆**：`growth_min_ticks=20`（堵"1 格=出生即成年"）、**出芽**（紧贴母体 + 母体供养，
  唯一"每项都变好"的机制，异养 2.1%→22.8%）、**整口咬合**（咬合力=单格×杀手数，
  击杀时间从 √n 变 n）、**基因组/段级算子**（体型 2.54→4.81、物种 24.3→68.3）。
- 异养瓶颈：一个生产者细胞 = 一个完整个体（满照 0.12/tick、成本 0.010、`food_needed=1`），
  免费的太阳能永远赢过追着别人跑的捕食。
- `growth_cost` **不是**刹车（1.0/0.3/0.1 → 基因组 6.56/4.18/2.96，越便宜越小，r 选择）。
- 变异率有天花板：50% 进入 runaway（pending 934 / dropped 235）。
- 系统坐在分岔点上，"调大 = 更好"是错的直觉（多次验证）。
- 体型不能当"成本生效"的判据——它主要由空间竞争决定。

## 进度

- M0–M5、批次 0–19 完成（世界窗口/存档/检视器/曲线/类型参数/谱系树/环境场/温度/遮挡/嘴咬/
  维持成本/云影/富贫斑块/最短成长期/出芽/整口咬合/收入审计）。
- 批次 45：基因组 + 发育（`genome.rs`，`Genome→BodyPlan`，`expand()` 纯函数无 RNG，
  BFS 8 邻域发育序，`MAX_PLAN_CELLS=64`）。
- 批次 46：段级算子（`repeat±1`≤8 / `mirror` / 插删段 / 移锚点），双轨物种身份
  （指纹=谱系、组成 Jaccard=形态）。`VERSION`=36。
- 当前 193 core + 50 app + 10 regression + 3 render 测试。

## 下一步

1. **体型收益**（进行中）：要**结构性**收益（只有大体型能做到的事），线性加成全部失败过。
2. **阶段 2**：嵌套段（`children`，depth ≤ 2）。
3. 候选：纯异养（抬生产者成本/降 bite_rate/光更稀缺）、长期 50000 tick 稳态、卵生、
   洞穴+暗区腐殖质、捕食者-猎物周期可视化。
