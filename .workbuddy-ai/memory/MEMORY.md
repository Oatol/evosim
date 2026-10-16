# evosim 项目约定

网格细胞自动机开放式进化沙盒。生物 = **可共同进化的身体蓝图 + 神经网络大脑**，无适应度函数。
**详细设计见 `DESIGN.md`（权威文档，架构变更必须同步它）；本文件只记不变量、踩过的坑、与工作约定。**

## 决策

| 项 | 结论 |
|---|---|
| 技术栈 | 原生 JS (ES2020+) + ES Modules + Canvas 2D，**零依赖、无构建** |
| 大脑 | 固定拓扑神经网络 + 权重变异（40→12→5） |
| 物种 | **身体结构指纹**（FNV-1a 布局哈希）；权重变异不产生新物种 |
| 世界 | 运行时可选尺寸，默认 200×150 |
| 目标 | 可玩沙盒（好看、好玩、可干预） |

## 不变量（改代码前先看这条）

- 目录 `src/{core,life,stats,render,ui,io}`，入口 `src/main.js`
- 网格 **TypedArray SoA**：`types`(Uint8) + `owners`(Int32) + **`floor`(Uint8)**，索引 `row*cols+col`
- **`floor` = 地板层**（地形：墙/岩浆/沼泽/辐射带/腐殖质）。生物盖上去只改 `types`，离开时 `World.revealFloor()`
  把地形铺回顶层。**没有这层，地形会被生物啃光**。只有玩家画"空"/橡皮擦才真清地板；
  食物被吃/腐败也走 `revealFloor`。食物**不在** floor 里（资源不是地形）
- **网格写入唯一入口 `world.setCellAt()` / `fillGrid()`**——它们维护食物索引与地形索引。绕过会破坏索引
- **食物索引**：`foodCells`(数组) + `foodPos`(Int32Array)，swap-remove O(1)；腐败抽样按**数组位置**，
  所以存/读档**必须保持原样顺序**，读档**不能走 `_initFoodIndex()`**
- 主循环**两阶段**：A 感知+决策（只读）→ B 随机顺序结算（写世界），消除遍历顺序偏差
- 渲染**离屏静态层 + 脏格更新**（离屏 1px=1格）；World 用事件通知渲染层，不认识 Renderer
- 随机数走 `RNG.js`（xorshift128 + splitmix32），存档含 RNG 状态
- **落位后必须 `normalizePosition()`**；繁殖时子代偏移用**未归一化**母体坐标算，否则 `isStraightPath` 误判受阻
- **变异两级**：先掷 `mutability` 决定这胎变不变，再对 `addProb/changeProb/removeProb` 各掷一次；
  `mutability` **只在变异真发生时**漂移。**变异后必须按新蓝图严格复查落位**（加细胞会让子代变大）
- **物种** = FNV-1a 32 位指纹（与细胞顺序/位置/朝向无关）；`Species` 不持 `Anatomy` 引用，只存描述快照。
  两道闸门：`minDiscard`(10) + `MAX_SPECIES = 4000`
- **化石记录**定长环形缓冲；`extract()` 按 stride 抽稀但**末点必留**
- **感知 40 维**：`[0..35]` 视觉 4 方向 ×（8 类型 one-hot + 归一化距离）+ `[36]` 能量 `[37]` 年龄 `[38]` 移动能力 `[39]` 偏置。
  **眼睛决定能看哪个方向**（无眼方向全 0）。`VISUAL_SLOTS = 9` 与 `SENSE_DIM = 40` **写死**，加可见类型要改 → 作废旧存档
- **输出层线性**（argmax 决策），隐藏层 tanh；**`b2` 用 σ=0.5 随机初值**（无眼个体输出全靠 b2，全 0 会造种群级方向偏置）
- **`turnCooldown`（默认 4）必需**，否则调高 `o[4]` 权重的个体原地打转却不受选择压力惩罚
- **新生个体出生那 tick 还没 `think()`**，`brain.lastInput` 为 `null`；遍历全体生物的脚本要跳过这帧
- **参数面板全部由 `CONFIG_FIELDS`/`CONFIG_SECTIONS` 生成**，加参数只改 `config.js`；绝大多数**每 tick 现读**
- **网格尺寸是唯一"延迟生效"参数**：拖滑块只写 `ControlPanel._pending`，**不能让滑块直接写 `Config.world.cols`**
- **参数抽屉是 `.stage` 的 flex 子项**不是浮层（展开时画布变窄而非被盖住）
- **细胞注册表 `CELL_DEFS`**：加细胞 = 加一个定义对象。字段 = `id/key/name/label` + 结构标记
  （`isBodyCell/independent/paintable/visible/directional/flag/behavior`）+ 地形标记（`terrain/passable/enter/stand/tick`）
  + 属性默认（`hp/cost/damage/senseRange/foodValue`）+ `colors` 三套配色。新行为往 `BodyCell.CELL_BEHAVIORS` 加函数
- **细胞 id 一旦定下不能改**，且**必须 < 64**（`Sensors.VISION_SLOT` 是 `Int8Array(64)`）；
  **`VISIBLE_CELL_TYPES` 顺序不能动**（顺序即大脑输入槽）
- **运行时属性值在 `Config.cells[KEY][attr]`**，注册表只是出厂默认；`cellAttr()` 读不到回退默认
- **个体派生量全走属性表且刻意不缓存**（`maxHealth=Σhp`、`upkeepCost=Σcost×metabolismScale`、`foodValueTotal=ΣfoodValue`）
- **地形三钩子**：`enter`（踏入，`attemptMove` 调）/ `stand`（驻留每 tick，`act` 移动之后调）/
  `tick`（世界级，`applyTerrainTick` 调，需索引）。函数在 `life/Terrain.js`，数值在 `Config.terrain`。
  **读 `floor` 层而不是顶层 `types`**（生物自己的细胞就压在枢轴格上）
- **存档四件套**：配置快照 + 地形 + 全部个体（含 base64 Float32 权重）+ RNG 状态；外加物种表。
  地形存 `floor` 层（含被生物踩住的），按类型 key 分组 `grid.terrain`；`SAVE_VERSION = 2`，v1 仍可读
- **`localStorage` 必须过 `storageAvailable()` 探测**（`file://` 下抛 `SecurityError`）
- **CSV 带 UTF-8 BOM**；字段按 RFC 4180 引号包裹
- **浮窗点击穿透**：外层 `pointer-events:none`、只有面板 `auto`。**绝不用全屏 backdrop**——它会吃掉所有点击
  （含工具栏），这是"克隆选中"永远失败的根因
- **未锁定浮窗只被"点世界画布"关闭**（判据 `event.target.closest('.canvas-wrap')`），点工具栏/顶栏不关。
  **物种树默认锁上**（选完物种还要去画布操作），检视器默认不锁
- **关闭路径必须收敛到各自 `close()`**（`onClose`/`onOpenChanged` 只在这里触发），`close()` 要加 `if (this.root.hidden) return;`
- **Esc 分层**：检视器 → 物种工坊 → 物种树 → 参数抽屉 → 退出放置模式
- **渲染循环与模拟解耦**：`running` 只控制"是否推进世界"，RAF 循环常驻。**`pause()` 不能 `cancelAnimationFrame`**
  （否则 `onFrame` 不再触发，准星/高亮/浮窗冻在最后一帧）
- **`engine.fps`/`tps` 暂停时是冻结旧读数**；测性能必须先 `engine.start()`
- **物种树只展开存活分支**（前序遍历；`buildLineage` 是不碰 DOM 的纯函数），节点上限 240、存活物种一律保留
- **物种色由指纹哈希决定**（确定性不存表）——这是"按物种着色"与物种树能互相对照的前提
- **两种高亮都画在主画布上**（非世界状态）；相机跟随由 `engine.onFrame` 消费，个体死亡/检视器关闭都要 `setFollow(false)`
- **生物库读回统一走 `blueprintOf()`**（曾手写解析把 `dx` 读成 `type`，蓝图整个错位）
- **桌面版写盘通道**：exe 本地服务 `GET /__evosim/ping` + `POST /__evosim/save`，HTML 注入
  `window.__EVOSIM_HOST__={app,dir}`。**浏览器版没标记就一个探测请求都不发**（失败请求会留控制台错误，
  而"无控制台错误"是端到端硬指标）。`safeName()` 只取 basename 并清洗非法字符

## 踩过的坑（都是真发生过的）

- **全屏 backdrop 遮罩**：检视器和物种树都栽过。症状是"跟随看不见""克隆永远提示先选中""关闭按钮不显眼"——
  同一个原因。改成非模态浮窗后一起消失
- **`pause()` 里 `cancelAnimationFrame`**：暂停后准星/高亮/浮窗全冻住
- **`isPassable()` 给 FOOD 也标了 true**：把 `combat.foodBlocksReproduction` 架空（食物本该挡路）。
  `isPassable()` 现在**只对 `terrain: true` 生效**，食物那条特例只由 `isClear`/`canPlaceAt` 一处管。
  phase5-test 的"读档后可复现"在第 48 tick 分叉，把它抓了出来
- **地形效果读顶层 `types`**：生物自己的细胞就压在枢轴格上 → "站在辐射带上掉血"永远测不到。必须读 `floor`
- **辐射倍率直接改 `mutability`**：5% ×3 ×3 ×3 三 tick 就封顶（指数爆炸）。改成打一个每 tick 重置的
  `inRadiation` 标记，由 `Mutation` 在繁殖那一步临时乘倍率
- **`setPointerCapture` 在合成 PointerEvent 上抛 `InvalidPointerId`** → 包 try/catch
- **读档后演化分叉**：RNG 状态 + 食物索引顺序缺一不可
- **`grid.fill()` 绕过索引**：会让食物索引与网格脱节
- **打包不能直接拼接模块**：`Mutation.js` 与 `Camera.js` 都有顶层 `clamp`，会静默覆盖
- **exe 不能 `shell:true` 调 `npx postject`**：路径含空格会被拆成多个参数
- **缩小时世界被重复平铺**：平铺只在"世界比视口大"的方向做

## 操作映射

左键拖=绘制 · 右键/中键/Alt+左键=平移 · 滚轮=以光标为锚缩放 · 空格=播放/暂停 · S=单步 · R=重置 ·
F=适应窗口 · P=参数抽屉 · T=物种树 · G=物种工坊 · I=切换检视工具 · Ctrl/Cmd+S=保存 ·
Esc=按「检视器 → 物种工坊 → 物种树 → 参数抽屉 → 退出放置」逐层关

- 笔刷（**由 `TERRAIN_BRUSH_TYPES` 派生**，不再手写）：空/食物/墙/岩浆/沼泽/辐射带/腐殖质/**橡皮擦**/随机生物/克隆
  - 注意**不能用 `PAINTABLE_CELL_TYPES`**——它含身体细胞（只能在工作坊蓝图里画）
- **橡皮擦**：只清地形，**食物与生物一律不动**（判据是注册表 `terrain` 标记）。走 `setCellAt(EMPTY)`（连地板一起清）。
  「空」笔刷是无差别覆盖（会抹掉生物身体），两者并存
- 检视器头部：保存生物 · 跟随 · 图钉 · ×
- 克隆笔刷 / 工坊「手动放置」：装蓝图 → 画布上点即投放（Esc 取消）

## 工作约定

- **版本控制**：`https://github.com/Oatol/evosim`（私有），远端 `origin`，主分支 `main`。
  `.gitignore` 排除 `node_modules/`、`build/`、`dist/evosim.exe`（83MB 可复现）、`The-Powder-Toy-master.zip`（第三方 GPL 参考）；
  **`dist/evosim.html` 纳入版本控制**。`.gitattributes` 强制 `eol=lf`
- 开发用 `python -m http.server`（ES Modules 需 HTTP）；交付用 `dist/evosim.exe` 或 `dist/evosim.html`（双击即玩）
- **每次改动都要跑全量回归 + 重新打包**（见下面「验证方式」）

## 进度与下一步

- **阶段 1–6**（10-15）骨架/生命/遗传/大脑/交互/完整实验室 —— 全完成
- **阶段 7**（10-16）细胞注册表 + 生命值体系 + 物种工坊 + 检视器非模态浮窗；
  补充：物种树浮窗化、渲染循环常驻、工坊手动放置、导出落程序目录、食物量倍率、橡皮擦
- **阶段 8**（10-16）**地形与 onEnter 钩子**：`floor` 地板层 + `enter`/`stand`/`tick` 钩子 +
  岩浆/沼泽/辐射带/腐殖质 + 橡皮擦改法 + 画笔派生 + 存档 v2。**批次 1 已交付并验收**
- **下一步（按用户排的顺序）**：储食细胞（能量缓冲，让代谢倍率成为主玩法）→ 再生+刺 → 水/冰 →
  毒素/粘附（完整毒素场）→ 环境场（温度/辐射，**先只做可观察**）+ 可见类型扩容（会作废旧存档，
  **用户已批准存档断代**，只需提前说一声）
- 环境场参考 **The Powder Toy**（`The-Powder-Toy-master.zip` 在项目根，不纳入仓库）：它的空气/热场是
  **粗粒度**的（`Air.h` 的 `ovx[YCELLS][XCELLS]`，`CELL = 4`）——全分辨率场不可行（扩散 O(30000)/tick，
  ×50 倍速 ≈ 1.2 亿次操作/秒）。**悬停查看**也参考 TPT
- **扩展清单与成本分档见 `DESIGN.md` §15**；阶段 9 = 性能剖析 + TypedArray 调优 + 预设世界模板 + 收尾"眼睛一直稀有"

## 验证方式

- 逻辑层：`tools/phase3-test.mjs` … `phase8-test.mjs`（node 直接跑）
- 参数扫描：`phase3-sweep.mjs`（变异率）/ `phase7-sweep.mjs`（代谢消耗）；生态测量：`phase4-ecology.mjs`
- 端到端：`tools/cdp-phase3/4/5/6/7/8.mjs`（http server + 无头 Chrome + CDP）
- 产物验证：`verify-bundle.mjs [URL]`；`verify-exe.mjs`（起真 exe，验写盘端点 + 路径穿越清洗）
- 打包：`bundle.mjs`（→ dist/evosim.html）、`build-exe.mjs`（→ dist/evosim.exe，Node SEA）

### CDP 硬规矩
- **每次从干净 profile 开始**：`http.server` 不发 `Cache-Control`，Chrome 启发式缓存 JS，复用 profile 会拿到上一版代码
- **断言不要硬编码阶段号**，统一 `/^阶段 \d/`（踩过三次）
- **必须自己管住 Chrome 进程**：`chrome.kill()` 只发 SIGTERM，Windows 上常被忽略、留子进程**继续占调试端口**。
  后果极隐蔽——新 Chrome 绑不上端口，`fetch('/json/list')` 连到**旧实例页面**，测试在别人的页面里跑。
  所以：① 启动前 `freePort(PORT)` 杀残留；② 结束用 `taskkill /F /T /PID`；③ 加"启动时没有预置状态"哨兵断言
- **不要返回带循环引用的对象**（如 `renderer.highlight`）——`returnByValue` 报 `Object reference chain is too long`，只返回 `id`
- **鼠标事件会被浮层拦截**：用 `document.elementFromPoint(x,y)` 确认点击目标；
  找"画布上的生物"前先 `toolbar.fitToWindow()`，否则个体跑到视口外会偶发失败
- **无头浏览器 `fps`/`tps` 会抖动**：多次采样取最优
- 调试入口 `window.evosim`：Config/world/camera/renderer/editor/engine/colorScheme/rng/record/charts/stats/inspector/toolbar/controlPanel/timeline/speciesTree/library/speciesEditor +
  `save/load/quickSave/quickLoad/quickInfo/storageAvailable/exportFossil/exportSpecies/saveText/hostSaveDir/toast`
