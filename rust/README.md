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
  crates/render/             wgpu 渲染（R8 索引 + 调色板 / 物种色纹理）
    src/grid.wgsl            全屏三角形着色器：屏幕像素 → 世界坐标 → 查纹理
    tests/render.rs          像素级回归（精确颜色 / NEAREST / 世界环绕 / 物种色）
  crates/app/                原生窗口（winit）：相机、着色模式、暂停/调速
  golden/                    JS 版导出的基线数据
  shots/                     离屏渲染出来的截图（type.png / species.png）
```

## 常用命令

```bash
cd rust
cargo build --release                     # 构建
cargo test --release                      # 单元 + 黄金快照 + 渲染像素回归

cargo run --release --bin evosim -- --sim --seed 1001 --ticks 2000   # 无界面
cargo run --release -p evosim-app                                     # 打开窗口
cargo run --release -p evosim-app -- --screenshot shots/type.png --ticks 400
cargo run --release -p evosim-app -- --bench --seconds 4 --speed 50    # 压测

node tools/package.mjs                    # 打包交付用的单 exe → ../dist/evosim2.exe
python tools/subset-font.py               # 重新生成中文界面字体子集（改过 UI 文案后要跑）
python tools/subset-font.py --check       # 只检查：有没有子集覆盖不到的字
```

窗口快捷键：空格 暂停/继续 · S 单步 · `[` `]` 调速（1/5/10/50×）· **Tab 类型/物种着色**
（Tab 在交给 egui 之前就被截走，不会在顶栏按钮之间轮换焦点）·
F 适应窗口 · Esc 退出；滚轮以光标为锚缩放，右键/中键拖动平移，**左键落笔**（画笔工具）。

界面：顶部状态条 · 左侧工具/笔刷 · 右侧 6 个关键参数 · 右下悬停读数 ·
检视器（点个体）· 物种面板（点选高亮）· 曲线（种群/食物/物种）。

工具：画笔（左键落笔）· 平移（左键拖动）· **检视（左键点个体）**。
顶栏右侧有「工具」「工坊」开关；每个窗口标题栏都有 × 关闭。
`G` 打开物种工坊：画好蓝图 → 「手动放置」装进放置模式，在画布上点哪里就放哪里（Esc 取消），
或「放到视野中心」直接投放；「存入生物库」后可以从列表里反复放置。
笔刷方块是自绘控件（`painter` 直接画），鼠标悬停显示该细胞的中文简介，**选中后简介也会
直接显示在笔刷下方**——文案来自 `CellType` 注册表的 `desc` 字段，一处维护、三处复用
（笔刷/工坊/参数）。物种工坊的细胞调色板同样会在选中后显示简介。

检视器有**图钉**：锁上之后用「检视」点画布不会切换、也不会清空当前目标（1.x 的图钉是
"点画布不关浮窗"，2.0 的检视器本来就是 egui 窗口，所以把图钉落到"锁目标"上）。选中个体后
画布上会描出它的身体，同物种的其他个体用该物种的指纹色（暗一档）标出，可关。

高亮画在**窗口下面**：egui 同一 Order 内的图层按创建顺序绘制，所以这一层在帧首先登记一次，
否则会浮在所有窗口之上。

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

**稳态**（种群长到 ~1550 只、200×150 世界，用 `--bench` 连续跑 4 秒测的）：

| 版本 | step 耗时 | 相对 JS |
|---|---|---|
| JS 版（Chrome，1517 只） | 2.0 ms/tick | 1× |
| Rust 单线程（`RAYON_NUM_THREADS=1`，thin LTO） | 0.803 ms/tick | 2.5× |
| Rust + rayon（8 线程，thin LTO） | 0.414 ms/tick | 4.8× |
| **Rust + rayon（8 线程，fat LTO）** | **0.279 ms/tick** | **7.2×** |

对应 50× 倍速（目标 3000 tps）：实测 **~3580 tps（119%）** —— 50× 已经能跑满了。

两个容易搞错的点：

1. **别用"整轮平均"当性能数字**。种群从 9 只长到 1500 只，前几百 tick 几乎不花时间；
   平均下来会得到 0.21 ms/tick 这种好看但没意义的值。稳态才是真实负载。
2. **线程数按物理核数，不要用满逻辑核**。实测 8 线程 0.415 ms/tick，
   用满全部逻辑核反而退化到 0.483 —— 超线程在这种"每 tick 一小段并行"的负载上
   只增加调度与缓存竞争。`evosim_core::init_threads()` 已按此配置。

并行安全的前提：`think()` 只读世界、只写个体自己的计划与缓冲，**且完全不碰 RNG**，
所以所有黄金检查点的摘要与串行版逐位相同。

## 里程碑

- [x] **M0** 工作区 + 黄金快照导出
- [x] **M1** 内核移植 + 200 个检查点全一致
- [x] **M2** winit 窗口 + wgpu 渲染（R8 索引 + 调色板 / 物种色纹理，NEAREST 采样 + 世界环绕）
- [x] **M3** egui 骨架 + 自绘控件
  - [x] 顶部状态条（tick/种群/食物/物种/fps/tps + 暂停/单步/倍速/着色/适应）
  - [x] 左侧工具栏：画笔/平移/**检视**、**8 个自绘笔刷方块**（7 地形 + 橡皮擦，hover 显示简介）、
        **三种笔刷形状**（方形 / 圆形 / 喷枪）
  - [x] 右侧参数面板：6 个关键旋钮（消耗倍率/食物量倍率/造食率/变异率/种群上限/寿命）
  - [x] 检视器**图钉**（锁目标）+ 选中个体与同物种高亮（画在窗口之下）
  - [x] 右下悬停读数：坐标 · 类型 · 地形 · 归属个体 · 物种 · 能量 · 体型
  - [x] 中文字体（运行时读系统 msyh.ttc，找不到退回 simhei）
  - [x] **检视器**：点个体看能量/年龄/生命/体型/变异率/结构/能力徽章，可跟随、居中
  - [x] **物种面板**：按个体数排序的条形列表（颜色取自指纹），点选后世界高亮该物种
  - [x] **曲线**：种群/食物/物种数三条线（每 20 tick 采样，留 600 点）
  - [x] **物种工坊**：15×15 蓝图画布（点格子放细胞、右键擦、眼睛可设朝向）+ 生物库
        （存/载入/放置/居中/删除，落盘 `creatures.json`）
- [x] **M5** 字体子集 + 单 exe 打包 + release 调优
  - **中文界面字体内嵌**：`tools/subset-font.py` 扫全部源码收集用到的字符（当前 1000 个），
    用 fontTools 从微软雅黑裁出 **201 KB** 子集（关掉 hinting 从 576 KB 砍到 201 KB），
    `include_bytes!` 进 exe。**系统字体作为后备**挂在后面 —— 玩家自己起的名字里
    有子集外的字也能显示
  - 两条守护测试：`embedded_font_covers_every_ui_char`（用 ttf-parser 逐个查 cmap，
    防止"改了文案忘重新裁字体"→ 界面上出现方块）与 `embedded_font_is_small`（< 400 KB）
  - `release` profile：`lto = "fat"` + `codegen-units = 1` + `strip = "symbols"`
  - `node tools/package.mjs` → `dist/evosim2.exe` **6.84 MB**（1.x 的 Node SEA 是 83.6 MB）
- [x] **M4** 每格温度/辐射场 + 图层滤镜 + 悬停读数
  - `core/src/fields.rs`：**每格一个 u8** 的温度场与辐射场，源 = 岩浆 / 辐射带，
    5 点扩散 + 向本底衰减；实测 **0.154 ms/次**（200×150 两张场），60 Hz 下占 1% 的帧预算
  - **更新与 tick 解耦**：主程序按墙钟 60 Hz 推进，不随倍速——50× 下 tick 是 3000/s，
    场若跟着跑就是白烧 CPU；而且"岩浆烤热周围"是物理时间，1× 和 50× 应该一样快。
    也因此**不影响黄金快照**（场不参与 `step()`）
  - 滤镜：`Tab` 四档循环 类型 → 物种 → 温度 → 辐射；渲染层多采一张 R8 纹理 + 256 项色表，
    着色器只多一个分支；左下角有渐变图例
  - 悬停读数显示该格的**温度 / 辐射**百分比（每格精确值，不是粗粒度块）
  - `--screenshot --mode temp|rad` 可离屏出热力图
