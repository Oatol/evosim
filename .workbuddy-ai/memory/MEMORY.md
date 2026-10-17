# evosim 项目约定

网格细胞自动机开放式进化沙盒：生物 = **可共同进化的身体蓝图 + 神经网络大脑**，无适应度函数。
**架构细节见 `DESIGN.md`（权威文档，变更必须同步它）；本文件只记不变量、踩过的坑、下一步。**

原生 JS (ES2020+) + ES Modules + Canvas 2D，**零依赖、无构建**。大脑固定拓扑 + 权重变异（40→12→5）。
物种 = **身体结构指纹**（FNV-1a，权重变异不产生新物种）。世界默认 200×150。

## 不变量（改代码前先看这条）

**网格 / 循环 / 渲染**
- 目录 `src/{core,life,stats,render,ui,io}`，入口 `src/main.js`。网格 **TypedArray SoA**：
  `types`(Uint8) + `owners`(Int32) + **`floor`(Uint8)**，索引 `row*cols+col`
- **`floor` = 地板层**（地形：墙/岩浆/沼泽/辐射带/腐殖质）。生物盖上去只改 `types`，离开时 `World.revealFloor()`
  把地形铺回顶层。**没有这层地形会被生物啃光**。只有玩家画"空"/橡皮擦才真清地板；食物被吃/腐败也走 `revealFloor`。
  食物**不在** floor 里（资源不是地形）
- **写入唯一入口 `world.setCellAt()` / `fillGrid()`**（它们维护食物索引与地形索引，绕过会脱节）。
  **食物索引** `foodCells`(数组) + `foodPos`(Int32Array) 用 swap-remove；腐败抽样按**数组位置** →
  存/读档**必须保持原样顺序**，读档**不能走 `_initFoodIndex()`**
- 主循环**两阶段**：A 感知+决策（只读）→ B 随机顺序结算（写世界），消除遍历顺序偏差
- 渲染**离屏静态层 + 脏格更新**（离屏 1px=1格）；World 用事件通知渲染层，不认识 Renderer。
  随机数走 `RNG.js`（xorshift128 + splitmix32），存档含 RNG 状态

**生物 / 遗传 / 能量**
- **落位后必须 `normalizePosition()`**；繁殖时子代偏移用**未归一化**母体坐标算，否则 `isStraightPath` 误判受阻
- **变异两级**：先掷 `mutability` 决定这胎变不变，再对 `addProb/changeProb/removeProb` 各掷一次；
  `mutability` **只在变异真发生时**漂移。**变异后必须按新蓝图严格复查落位**（加细胞会让子代变大）
- **物种** = FNV-1a 32 位指纹（与细胞顺序/位置/朝向无关）；`Species` 不持 `Anatomy` 引用，只存描述快照。
  两道闸门 `minDiscard`(10) + `MAX_SPECIES = 4000`。**化石记录**定长环形缓冲，`extract()` 抽稀但**末点必留**
- **个体派生量全走属性表且刻意不缓存**：`maxHealth=Σhp`、`upkeepCost=Σcost×metabolismScale`、`foodValueTotal=ΣfoodValue`
- **繁殖阈值 ≠ 繁殖成本**：`reproduceAt()` 是**触发阈值**（储食细胞抬高它），`foodNeeded()` 是**实扣的
  基础成本**，差额留在能量里当储备。**无储食细胞时两者逐位相同**（基线不变）。`energyRatio` 用 `reproduceAt()`
- **再生 = `BodyCell.CELL_BEHAVIORS.regen`**：靠"逐个细胞调行为"的循环天然实现"多一个细胞多回一份血"，
  不需要计数。它抹的是 `damage`（累积伤害），不是加血上限
- **`harm(amount, attacker)`**：只有生物近战（杀手）传 `attacker`，地形伤害不传；**反伤时不再往下传
  attacker**（否则两只带刺的无限互扎）。刺不吃装甲减免（装甲判定在 `killerFunction` 里，两条路）

**大脑与感知**
- **感知 40 维**：`[0..35]` 视觉 4 方向 ×（8 类型 one-hot + 归一化距离）+ `[36]` 能量 `[37]` 年龄
  `[38]` 移动能力 `[39]` 偏置。**眼睛决定能看哪个方向**（无眼方向全 0）。
  `VISUAL_SLOTS = 9` / `SENSE_DIM = 40` **写死**，加可见类型要改 → 作废旧存档。
  **新增身体细胞默认 `visible: false`** → 不动这两者，**不造成存档断代**
- **输出层线性**（argmax），隐藏层 tanh；**`b2` 用 σ=0.5 随机初值**（无眼个体输出全靠 b2，全 0 会造
  种群级方向偏置）。**`turnCooldown`（默认 4）必需**，否则调高 `o[4]` 权重的个体原地打转却不受惩罚
- **新生个体出生那 tick 还没 `think()`**，`brain.lastInput` 为 `null`；遍历全体生物的脚本要跳过这帧

**注册表 / 参数 / 地形**
- **细胞注册表 `CELL_DEFS`**：加细胞 = 加一个定义对象。字段 = `id/key/name/label` + 结构标记
  （`isBodyCell/independent/paintable/visible/directional/flag/behavior`）+ 地形标记（`terrain/passable/enter/stand/tick`）
  + 属性默认（`hp/cost/damage/senseRange/foodValue`）+ `colors`。新行为往 `BodyCell.CELL_BEHAVIORS` 加函数
- **细胞 id 一旦定下不能改**，且**必须 < 64**（`Sensors.VISION_SLOT` 是 `Int8Array(64)`）；
  **`VISIBLE_CELL_TYPES` 顺序不能动**（顺序即大脑输入槽）。运行时值在 `Config.cells[KEY][attr]`，
  注册表只是出厂默认，`cellAttr()` 读不到回退默认
- **参数面板全由 `CONFIG_FIELDS`/`CONFIG_SECTIONS` 生成**，加参数只改 `config.js`；绝大多数**每 tick 现读**。
  抽屉是 `.stage` 的 flex 子项（展开时画布变窄而非被盖住）。**网格尺寸是唯一"延迟生效"参数**：
  拖滑块只写 `ControlPanel._pending`，**不能让滑块直接写 `Config.world.cols`**
- **地形三钩子**：`enter`（踏入，`attemptMove` 调）/ `stand`（驻留每 tick，`act` 移动之后调）/
  `tick`（世界级，`applyTerrainTick` 调，需索引）。函数在 `life/Terrain.js`，数值在 `Config.terrain`。
  **读 `floor` 层而不是顶层 `types`**（生物自己的细胞就压在枢轴格上）

**存档 / 导出（JS 版已冻结，细节见 DESIGN.md）**
- JS 版：存档四件套（配置快照 + 地形 floor 层 + 全部个体含权重 + RNG）+ 物种表，`SAVE_VERSION=2`；
  CSV 带 UTF-8 BOM；`localStorage` 过 `storageAvailable()` 探测
- 浮窗一律**点击穿透**（外层 `pointer-events:none`、面板 `auto`），**绝不用全屏 backdrop**
- **渲染循环与模拟解耦**：`pause()` 不能 `cancelAnimationFrame`（否则 onFrame 不再触发，准星冻住）
- 物种色 = 指纹哈希（确定性、不存表）——"按物种着色"与物种树能互相对照的前提

## 架构决策：JS 版冻结，转 Rust 原生（2026-10-17）

- **JS 版（仓库根）冻结**：不再加功能、不再发版，只作**参照实现 + 黄金快照来源**
- **Rust 版在 `rust/`**：`crates/core`（模拟内核，零窗口依赖）+ `crates/cli`；
  后续 `crates/render`（wgpu）/ `crates/ui`（egui 骨架 + 自绘控件）/ `crates/app`
- 界面路线：**egui 只当输入路由/文本/滚动/IME 的底座**，滑块/笔刷/色块/曲线/物种树/工坊网格用 `painter` 自绘
- 存档：**全新二进制格式，不兼容旧档**（用户明确同意）
- **环境场（M4）与 tick 解耦**：`World::update_fields()` 由主程序按墙钟 60 Hz 调，**不在 `step()` 里**。
  每格一个 u8（温度 / 辐射各一张 30 KB），5 点扩散 + 向本底衰减，实测 0.154 ms/次。
  这样 50× 倍速不会把 CPU 烧在扩散上，且场不影响黄金快照
- **一致性靠黄金快照**：`tools/export-golden.mjs` 导出 JS 基线，
  `tools/parity-check.mjs` + `cargo test` 双向守门。当前 5 种子 × 2000 tick × 每 50 tick = 200 检查点全一致
- 三条铁律（都踩过）：① `CELL_DEFS` 下标必须等于 id ② 压缩死亡个体必须稳定（`Vec::retain`）
  ③ 构造 `Organism` 必须照抄那 545 次高斯调用（虽然被覆盖）
- `Math.log` vs Rust `ln` 有 1–2 ulp 差异（200 个高斯样本差 6 个），但 f32 存储吸收了它 → 摘要仍全等

## 性能基线（2026-10-17 实测，讨论性能前先看这张表）

**稳态**（种群长到 ~1550 只、200×150）：

| 项 | 实测 |
|---|---|
| JS 版（Chrome，1517 只） | 2.0 ms/tick |
| Rust 单线程 | 0.803 ms/tick（2.5×） |
| **Rust + rayon（按物理核 8 线程，fat LTO）** | **0.279 ms/tick（7.2×）** |
| 50× 倍速（目标 3000 tps） | **~3580 tps（119%），已跑满** |
| 环境场 update（两张 30k 格） | 0.154 ms/次 → 60Hz 约占 1% 帧预算 |
| 全图重着色 30k 像素 | 0.043 ms/次；render 全量重建 0.26–0.30 ms/帧 |

- **瓶颈是每只生物 ~1.3 µs/tick 的 CPU 模拟**（其中大脑前向约 1.0 µs，占 77%），不是渲染 →
  换 GPU 渲染几乎零收益；单只成本**与体型线性相关**（act 遍历细胞、think 遍历带朝向的细胞）
- **测性能必须用 `--bench` 的稳态**：整轮平均（含种群从 9 长到 1500 的低负载阶段）会得到
  好看但无意义的 0.21 ms/tick
- **线程数按物理核数**（逻辑核一半）：8 线程 0.415，用满逻辑核反而 0.483

## 踩过的坑（都是真发生过的）

- **egui 窗口里"立刻关闭自己"不能写 `self.open = false`**：闭包外那个 `open` 变量（供 × 用）
  会在函数末尾被写回 `self.open = open`，把闭包里的 false **覆盖掉**。要用 `want_close` 标记，
  末尾 `self.open = open && !want_close`
- **"一次性显式意图"不该挂在工具上**：手动放置原来藏在 `paint_at_cursor` 里，而那条路径要求
  工具是「画笔」→ 拿着检视/平移就放不出来。抽成 `left_action(placing, tool)` 纯函数，放置优先
- **右键统一是"拖动世界画布"**：任何窗口（含工坊）都不吃右键，否则盖在画布上的窗口会抢走平移，
  还会误触擦除。工坊蓝图格只认左键
- **egui-winit 对 Tab 无条件 consume**（拿它做焦点轮换）→ 自己的 Tab 快捷键永远不执行，
  且焦点在顶栏按钮间乱跳。必须在 `state.on_window_event()` **之前**拦下来并 return
- **egui 同一 Order 内的图层按创建顺序绘制**（`GraphicLayers` = `IdMap<PaintList>`）→
  想画在窗口下面，就得在**帧首先**登记那一层，否则后建的一律盖在窗口上
- **地形格的顶层 `types` 存的就是地形自己**（HUMUS/LAVA…），所以"能不能往上放食物"
  的判据是「`EMPTY` **或裸地形**」，只判 EMPTY 永远放不上去
- **egui 想"画在窗口下面"只能用 `Order::Background`**：`Context::layer_painter()` 不创建
  图层（要等 add 图形才登记），而且同一 Order 内**非 Area 图层永远排在 Area（窗口）之后**。
  `Order::Background` 的注释原文就是 "Painted behind all floating windows"
- **改过 UI 文案要重跑 `python tools/subset-font.py`**，否则新字在界面上是方块
  （有测试 `embedded_font_covers_every_ui_char` 守着）。裁字体时 `hinting=False`
  能把 576 KB 砍到 201 KB——12px 屏幕上字体自带的 hinting 用不上
- **全屏 backdrop 遮罩**：检视器和物种树都栽过。症状是"跟随看不见""克隆永远提示先选中""关闭按钮不显眼"——
  同一个原因，改成非模态浮窗后一起消失
- **`pause()` 里 `cancelAnimationFrame`**：暂停后准星/高亮/浮窗全冻住
- **`isPassable()` 给 FOOD 也标了 true**：把 `combat.foodBlocksReproduction` 架空（食物本该挡路）。
  现在它**只对 `terrain: true` 生效**，食物那条特例只由 `isClear`/`canPlaceAt` 一处管。
  phase5-test 的"读档后可复现"在第 48 tick 分叉把它抓了出来
- **地形效果读顶层 `types`**：生物自己的细胞就压在枢轴格上 → "站在辐射带上掉血"永远测不到。必须读 `floor`
- **辐射倍率直接改 `mutability`**：三 tick 就指数封顶。改成每 tick 重置的 `inRadiation` 标记，繁殖那一步才乘倍率
- **刺细胞反伤递归**：不截断会爆栈；`attacker.harm(back)` 必须不带 attacker（递归深度恒为 1）
- **测试赌"演化会自然长出某种细胞"**：加细胞类型会改变变异候选池 → 演化轨迹改变 → 断言翻车
  （phase5-test 的"眼睛带朝向"）。改成用**专门装了眼睛的小世界**验证序列化，不赌演化
- **读档后演化分叉**：RNG 状态 + 食物索引顺序缺一不可；**`grid.fill()` 绕过索引**会让食物索引与网格脱节
- **`setPointerCapture` 在合成 PointerEvent 上抛 `InvalidPointerId`** → 包 try/catch
- **打包不能直接拼接模块**：`Mutation.js` 与 `Camera.js` 都有顶层 `clamp`，会静默覆盖；
  **exe 不能 `shell:true` 调 `npx postject`**（路径含空格会被拆成多个参数）；**缩小时世界被重复平铺**
  （平铺只在"世界比视口大"的方向做）

## 操作映射（详见 DESIGN.md §9.1）

左键拖=绘制 · 右键/中键/Alt+左键=平移 · 滚轮=缩放 · 空格=播放/暂停 · S=单步 · R=重置 · F=适应窗口 ·
P=参数 · T=物种树 · G=工坊 · I=检视工具 · Ctrl/Cmd+S=保存 · Esc 按「检视器→工坊→物种树→参数→退出放置」逐层关

笔刷**由 `TERRAIN_BRUSH_TYPES` 派生**（**不能用 `PAINTABLE_CELL_TYPES`**——它含身体细胞，那只能在工坊蓝图里画）。
**橡皮擦**只清地形、食物与生物一律不动（判据是注册表 `terrain` 标记）；「空」是无差别覆盖，两者并存。

## 工作约定

- **版本控制**：`https://github.com/Oatol/evosim`（私有，远端 `origin`，主分支 `main`）。`.gitignore` 排除
  `node_modules/`、`build/`、`dist/evosim.exe`（83MB 可复现）、`The-Powder-Toy-master.zip`（第三方 GPL 参考）；
  **`dist/evosim.html` 纳入版本控制**。`.gitattributes` 强制 `eol=lf`
- 开发用 `python -m http.server`（ES Modules 需 HTTP）；交付用 `dist/evosim.exe` 或 `dist/evosim.html`。
  **每次改动都要跑全量回归 + 重新打包**（见下）

## 进度与下一步

- **阶段 1–6**（10-15）骨架/生命/遗传/大脑/交互/完整实验室 —— 全完成
- **阶段 7**（10-16）细胞注册表 + 生命值体系 + 物种工坊 + 检视器非模态浮窗；补充：物种树浮窗化、
  渲染循环常驻、工坊手动放置、导出落程序目录、食物量倍率、橡皮擦
- **阶段 8**（10-16）**地形与 onEnter 钩子**：`floor` 地板层 + `enter`/`stand`/`tick` 钩子 +
  岩浆/沼泽/辐射带/腐殖质 + 橡皮擦改法 + 画笔派生 + 存档 v2
- **阶段 9**（10-16）**储食 / 再生 / 刺三种新身体细胞**：`reproduceAt()` 与 `foodNeeded()` 拆开（阈值抬高、
  成本不变，差额=储备）、再生行为、`harm` 加 attacker 与反伤截断。三种都 `visible:false` → **无存档断代**
- **下一步（按用户排的顺序）**：水/冰（`onEnter: slow` / 滑行惯性）→ 毒素/粘附（完整毒素场）→
  环境场（温度/辐射，**先只做可观察**）+ 可见类型扩容（会作废旧存档，**用户已批准存档断代**，只需提前说一声）
- 环境场参考 **The Powder Toy**（zip 在项目根，不纳入仓库）：它的空气/热场是**粗粒度**的
  （`Air.h` 的 `ovx[YCELLS][XCELLS]`，`CELL = 4`）——全分辨率场不可行（扩散 O(30000)/tick，
  ×50 倍速 ≈ 1.2 亿次操作/秒）。**悬停查看**也参考 TPT
- **扩展清单与成本分档见 `DESIGN.md` §15**；阶段 10 = 性能剖析 + TypedArray 调优 + 预设世界模板 + 收尾"眼睛一直稀有"

## 验证方式

- 逻辑层 `tools/phase3-test.mjs` … `phase9-test.mjs`；端到端 `tools/cdp-phase3/4/5/6/7/8/9.mjs`
- 扫描/测量：`phase3-sweep.mjs`（变异率）/ `phase7-sweep.mjs`（代谢消耗）/ `phase4-ecology.mjs`
- 产物：`verify-bundle.mjs [URL]`、`verify-exe.mjs`（起真 exe 验写盘端点 + 路径穿越清洗）
- 打包：`bundle.mjs`（→ dist/evosim.html）、`build-exe.mjs`（→ dist/evosim.exe，Node SEA）

### CDP 硬规矩
- **每次从干净 profile 开始**：`http.server` 不发 `Cache-Control`，Chrome 启发式缓存 JS，复用会拿到上一版代码
- **断言不要硬编码阶段号**，统一 `/^阶段 \d/`（踩过三次）
- **必须自己管住 Chrome 进程**：`chrome.kill()` 只发 SIGTERM，Windows 上常被忽略、留子进程**继续占调试端口**。
  后果极隐蔽——新 Chrome 绑不上端口，`fetch('/json/list')` 连到**旧实例页面**，测试在别人的页面里跑。
  所以：① 启动前 `freePort(PORT)` 杀残留；② 结束用 `taskkill /F /T /PID`；③ 加"启动时没有预置状态"哨兵断言
- **不要返回带循环引用的对象**（如 `renderer.highlight`）——`returnByValue` 报 `Object reference chain is too long`，只返回 `id`
- **鼠标事件会被浮层拦截**（用 `elementFromPoint` 确认目标）；找"画布上的生物"前先 `toolbar.fitToWindow()`；
  **无头 `fps`/`tps` 会抖动**，多次采样取最优
- 调试入口 `window.evosim`：Config/**CellType**/world/camera/renderer/editor/engine/colorScheme/rng/record/charts/stats/inspector/toolbar/controlPanel/timeline/speciesTree/library/speciesEditor +
  `save/load/quickSave/quickLoad/quickInfo/storageAvailable/exportFossil/exportSpecies/saveText/hostSaveDir/toast`
