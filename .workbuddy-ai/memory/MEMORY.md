# evosim 项目约定

网格细胞自动机开放式进化沙盒。参考 The Life Engine。差异：生物有**可共同进化的身体蓝图 + 神经网络大脑**，无适应度函数。

## 决策

| 项 | 结论 |
|---|---|
| 技术栈 | 原生 JS (ES2020+) + ES Modules + Canvas 2D，**零依赖、无构建** |
| 大脑 | 固定拓扑神经网络 + 权重变异（40→12→5） |
| 物种 | **身体结构指纹**（布局哈希）；权重变异不产生新物种 |
| 世界 | 运行时可选尺寸，默认 200×150 |
| 界面 | 完整实验室：参数面板 + 曲线 + 物种树 + 检视器 + 世界编辑器 + 物种工坊 + 存档 + CSV |
| 目标 | 可玩沙盒（好看、好玩、可干预） |

## 核心架构

- 目录 `src/{core,life,stats,render,ui,io}`，入口 `src/main.js`
- 网格 **TypedArray SoA**（`Uint8Array types` + `Int32Array owners`），索引 `row*cols+col`，越界收在 `GridMap.index()`
- 主循环**两阶段**：A 感知+决策（只读）→ B 随机顺序结算（写世界），消除遍历顺序偏差
- 渲染**离屏静态层 + 脏格更新**：离屏 1px=1格，全量走 ImageData + Uint32 调色板，增量 fillRect；主画布只 drawImage 缩放/平铺。缩放代价与格子数无关
- World 用事件（`cellChanged/fullRedraw/reset/stepped/organismAdded`）通知渲染层，不认识 Renderer
- 随机数走 `RNG.js`（xorshift128 + splitmix32），可复现
- 大脑只决策"去哪儿"，进食/攻击是身体细胞的被动能力

### 网格 / 食物
- **写入唯一入口 `world.setCellAt()` / `world.fillGrid()`**——它们维护食物索引。直接 `grid.fill()` 会破坏索引
- **食物索引**：`foodCells`（数组存 index）+ `foodPos`（Int32Array，-1 非食物），swap-remove O(1)，腐败/统计从 O(网格) 降到 O(食物数)
- **食物腐败**（`applyFoodDecay()` 2%）：生产者造的食物一半嘴够不着 → 堆积 → 堵死繁殖 → 灭绝。0% 会灭绝，2% 稳定 ~1774

### 生物 / 遗传
- 落位后必须 `normalizePosition()`，否则环绕移动让 col/row 漂移。繁殖时子代偏移用**未归一化**母体坐标算，否则 `isStraightPath` 误判受阻
- **变异两级**：先掷 `mutability`（默认 5%）决定这胎变不变，通过后对 `addProb/changeProb/removeProb` 各掷一次。`mutability` **只在变异真发生时**漂移（元变异）
- **变异后必须复查位置**：加细胞会让子代变大，原安全距离可能不够。流程「预检 → 造子代 → 变异 → **变异后严格复查**」。跳过会让子代与母体压同格，互相 `clearGrid()` 抹掉身体
- **物种**：FNV-1a 32 位指纹，与细胞顺序/位置/朝向无关；`Species` 刻意不持 `Anatomy` 引用，只存描述快照
- **物种爆炸两道闸门**：`minDiscard`（默认 10）+ `MAX_SPECIES = 4000`
- **化石记录**定长环形缓冲（`Float64Array`），`extract()` 按 stride 抽稀但**末点必留**，返回复用缓冲

### 神经网络
- **感知 40 维**：`[0..35]` 视觉 4 相对方向 ×（8 类型 one-hot + 归一化距离）、`[36]` 能量、`[37]` 年龄、`[38]` 移动能力、`[39]` 偏置。**眼睛决定能看哪个方向**（无眼方向全 0），射线跳过 EMPTY，同向多眼取更近
- **输出层线性**（argmax 决策，tanh 会抹平差异），隐藏层 tanh。He 初始化 + **`b2` 用 σ=0.5 随机初值**（无眼个体输出全靠 b2，全 0 造成种群级方向偏置）
- **输出是相对方向**：`o[0..3]` 前/右/后/左，`o[4]` 转身。`planDirection = (rotation + argmax) & 3`
- **`turnCooldown`（默认 4）必需**：否则调高 `o[4]` 权重的个体原地打转，能量收支却正常，选择压力抓不住
- **新生个体出生那 tick 还没 `think()`**，`brain.lastInput` 为 `null`，检视器按 0 兜底，遍历脚本要跳过这帧

### 参数面板
- **元数据生成**：`config.js` 的 `CONFIG_FIELDS` + `CONFIG_SECTIONS`。加参数只改 config.js。`DEFAULT_CONFIG` 是模块加载时的深拷贝快照
- 绝大多数参数**每 tick 现读**；少数需回调的收在 `main.js` 一个 `switch`（`wrapEdges`→`grid.wrap`；`recordSize`→重建缓冲；`baseTicksPerSecond`/`maxStepsPerFrame`→推给引擎）
- **网格尺寸是唯一"延迟生效"参数**：拖滑块只写 `ControlPanel._pending`，点"应用并重建世界"才真重建。**不能让滑块直接写 `Config.world.cols`**
- **参数抽屉是 `.stage` 的 flex 子项**不是浮层：展开时画布**变窄**而非被盖住
- 字段键支持一层**点号路径**（`MOUTH.hp`），`readPath/writePath` 解析；`CONFIG_FIELDS.cells` 由注册表自动填出
- **两个全局能量旋钮**（`life` 组）：**全局消耗量倍率** `metabolismScale`（乘在 `Σ cost` 上，默认 0 = 关闭）
  与**食物量倍率** `foodEnergyScale`（一份食物换多少能量，默认 1）。前者往上推会把生态推向灭绝，
  后者不会——它只是让个体更快攒够繁殖阈值，是安全的"加速键"。两者互补：想"卷"推消耗，想"肥"推食物

### 存档 / 导出
- **四件套缺一不可**：配置快照 + 地形（只存非空）+ 全部个体（含大脑权重）+ **RNG 状态**；外加物种表（否则读档后 id 重排、祖先链断）
- **权重 base64 编码原始 Float32 字节**：精确（JSON 浮点会分叉）+ 小（3.4→2.6KB）
- **食物索引必须按 `foodCells` 原样顺序存/恢复**：腐败按**数组位置**抽样，顺序是 swap-remove 的历史产物。读档**不能走 `_initFoodIndex()`**。顺序错了跑 250 tick 就分叉
- **读档落位**：先恢复配置与网格尺寸再建生物；生物走 `world.adoptOrganism()`（**不登记物种**）
- **`localStorage` 必须过 `storageAvailable()` 探测**（`file://` 下抛 `SecurityError`），快存失败降级成友好提示
- **CSV 带 UTF-8 BOM**（Excel 中文表头），字段按 RFC 4180 引号包裹
- **桌面版写盘通道**：exe 本地服务 `GET /__evosim/ping` + `POST /__evosim/save`，HTML 注入 `window.__EVOSIM_HOST__={app,dir}`。**浏览器版没标记就一个探测请求都不发**（失败请求会留控制台错误，而"无控制台错误"是硬指标）。`safeName()` 只取 basename 并清洗 `\ / : * ? " < > |`
- **落点**：桌面版 → 程序启动目录（`EVOSIM_SAVE_DIR` 可覆盖）；浏览器版 → 下载目录。落点在工坊状态栏与提示条明说

### 浮窗 / 交互
- **点击穿透**：`.inspector` / `.species-editor` / `.species-tree` 外层 `position:fixed;inset:0;pointer-events:none`，只有面板 `auto`。**不要用全屏 backdrop**——它会吃掉所有点击（含工具栏），这是"克隆选中"永远失败的根因
- **未锁定浮窗只被"点世界画布"关闭**，点工具栏/顶栏不关。判据 `event.target.closest('.canvas-wrap')`
- **图钉**：锁定后点画布不关。**物种树默认锁上**（选完物种还要去画布操作）；检视器默认不锁
- **关闭路径必须收敛到各自 `close()`**，`onClose`/`onOpenChanged` 只在这里触发，否则漏掉 `renderer.highlight` 与相机跟随清理。`close()` 加 `if (this.root.hidden) return;` 防重复
- **`ui/floating.js` 的 `enableDrag`** 是拖动唯一实现（检视器/工坊/物种树共用），`setPointerCapture` 要包 try/catch（合成事件 pointerId 会抛）
- **Esc 分层**：检视器 → 物种工坊 → 物种树 → 参数抽屉 → **退出放置模式**（`editor.cancelClone()`）。世界重建时检视器与物种树都要关
- **渲染循环与模拟解耦**：`running` 只控制"是否推进世界"，RAF 循环常驻。`_ensureLoop()` 在 `_rafId === null` 时挂帧，`_frame` 开头把 `_rafId = null`。**`pause()` 不能 `cancelAnimationFrame`**——否则 `onFrame` 不再触发，准星/高亮/浮窗冻在最后一帧。`_frame` 里模拟段条件 `this.running && this.speed > 0`，渲染段无条件
- **`engine.fps`/`tps` 暂停时是冻结旧读数**，`setSpeed()` 不启动引擎。测性能必须先 `engine.start()`

### 物种树 / 着色 / 高亮
- **只展开存活分支**（存活物种 + 祖先链）；列顺序**前序遍历**；谱系抽成纯函数 `buildLineage(list, now, maxNodes)`（不碰 DOM，可直接喂手工谱系测）
- **节点上限 240，存活物种一律保留**；祖先被裁时接到"最近的仍在展示中的祖先"
- **物种色由指纹哈希决定**（`ColorScheme.speciesColor/speciesRGB`），确定性不存表——这是"按物种着色"与物种树能互相对照的前提
- **`renderer.colorMode`**：`'type'` / `'species'`；地形两种模式都按类型；切换要 `requestFullRedraw()`；物种色 Uint32 有 `_speciesPal` 缓存
- **两种高亮都画在主画布上**（非世界状态）：`highlight`（单个体）/ `highlightFingerprint`（整物种，上限 4000）
- **相机跟随**由 `engine.onFrame` 消费 `inspector.following`——个体死亡/检视器关闭都要 `setFollow(false)`

### 细胞注册表（阶段 7）
- **`life/CellType.js` 的 `CELL_DEFS`**：每种类型一个定义对象（`id/key/name/label` + 结构标记 `isBodyCell/independent/paintable/visible/directional/flag/behavior` + 属性默认 `hp/cost/damage/senseRange/foodValue` + `colors` 三套配色）。**加细胞 = 加对象**（要新行为再往 `BodyCell.CELL_BEHAVIORS` 加同名函数）。`LIVING/PAINTABLE/INDEPENDENT/VISIBLE_CELL_TYPES`、`CELL_FLAG_NAMES`、配色、面板「细胞属性」分组**全由它派生**
- **id 一旦定下不能改**（写进存档 + 网格 Uint8Array）；**`VISIBLE_CELL_TYPES` 顺序不能动**（顺序即大脑输入槽）
- **Anatomy 标志位由 `def.flag` 推导**，`recompute()` 先清零再置位；`dir` 只在 `directional` 类型上参与指纹
- **运行时属性值在 `Config.cells[KEY][attr]`**，注册表只是出厂默认；`cellAttr()` 读不到回退默认
- **个体派生量全走属性表**（`maxHealth = Σhp`、`upkeepCost = Σcost × metabolismScale`、`foodValueTotal = ΣfoodValue`），**刻意不缓存**（属性可随时改）
- **默认值与旧行为逐位等价**（hp/foodValue/damage=1）→ 升级后生态不变，既有实测数据继续有效
- **代谢消耗默认关闭（`metabolismScale = 0`）**：生物能量长期贴近 0，任何消耗都让它们立刻饥饿。消耗×1 有 1/3 种子灭绝；可用组合「消耗×1 + 生产者造食率 15%」（稳定 ~2300）。**要让代谢成主玩法得先重构能量模型**（保留能量缓冲）
- **饿死有缓冲**：能量见底进入 `starveTicks` 计时，连续 `starvationTicks`（默认 60）没吃上才死（否则刚出生能量 0 的个体当 tick 暴毙）
- **死亡食物 = ΣfoodValue**：每格先变 1 份，超出部分由内向外投到周围空格
- **每只眼睛可有自己的视野**：`senseRange` 为 0 才回退全局 `sense.lookRange`
- **进食能量走 `life.foodEnergyScale`**（`BodyCell.foodEnergyGain()` 读，默认 1，非法值兜底成 1）。
  它**只影响嘴的进食**，不影响生产者造食、也不影响死亡掉落的食物份数——调的是"食物的热量"不是"数量"
- **加新细胞/地形的成本分档**（见 DESIGN.md §15）：不可见的无机像素 = 改 1 处 `CELL_DEFS`（存档兼容）；
  `visible: true` 是破坏性的（`VISUAL_SLOTS` 9 写死、`SENSE_DIM` 40 → 改了就作废旧存档大脑）；
  需要"进入某格有反应"的地形（岩浆/沼泽/水/辐射）要先加一个 `onEnter` 钩子——**一次投入解锁一整类**。
  另：`Sensors.VISION_SLOT` 是 `Int8Array(64)`，新类型 id 必须 < 64；`Toolbar.BRUSHES` 仍是手写的

### 物种工坊 / 生物库
- **生物库**（`io/CreatureLibrary.js`）存 `localStorage`（键 `evosim.creatures`）：紧凑蓝图 `[type,dx,dy,dir?]` + base64 大脑 + 名字。**读回统一走 `blueprintOf()`**，不要手写解析（曾把 `dx` 读成 `type`，蓝图整个错位）
- **两种投放**：「手动放置」（装进放置笔刷 + 收起工坊 + 点画布放置，Esc 取消）与「放到视野中心」；库条目同样有「放置」/「居中」
- **放置笔刷（`BRUSH_CLONE`）复用为手动放置**：`editor.setCloneBlueprint()` 装蓝图，`spawnFromBlueprint()` 投放。`editor.isPlacing` = `brushType==='clone' && !!cloneBlueprint`；`cancelClone()` 退出
- **放置反馈**：`Editor` 记 `_placed`/`_missed`，`pointerup` 时 `onPlaceFeedback(placed, missed)` 汇报

## 操作映射

左键拖=绘制 · 右键/中键/Alt+左键=平移 · 滚轮=以光标为锚缩放(0.5~40 px/格) · 空格=播放/暂停 · S=单步 · R=重置 · F=适应窗口 · P=参数抽屉 · T=物种树 · G=物种工坊 · I=切换检视工具 · Ctrl/Cmd+S=保存到文件 · Esc=按「检视器 → 物种工坊 → 物种树 → 参数抽屉 → 退出放置」逐层关

- 工具栏"检视"：左键单击生物开检视器（点空地不改选中）
- 检视器头部：保存生物 · 跟随 · 图钉 · ×
- 克隆笔刷：检视个体 → 点"克隆选中" → 画布上点即投放
- **橡皮擦**（`BRUSH_ERASER`）：只清 `owners < 0` 的地形格（墙/食物），**生物细胞一律不动**。
  和「空」笔刷的区别：「空」是无差别覆盖（会抹掉生物身体），橡皮擦是"只清障碍不伤生物"。
  判据用 owner 而不是类型枚举，将来加新地形不用改代码

## 工作约定

- 设计文档 `DESIGN.md`，**架构变更需同步更新**
- 开发用 `python -m http.server`（ES Modules 需 HTTP）；交付用 `dist/evosim.exe` 或 `dist/evosim.html`（双击即玩）
- 8 阶段路线，每阶段结束项目都应可运行、可观察

### 进度
- **1**（10-15）网格 + 主循环 + 离屏渲染 + 画笔 + 时间轴。200×150 / 60tps / 60fps
- **2**（10-15）9 种细胞 + `BodyCell` 行为 + `Anatomy` + `Organism` 两阶段生命周期 + `OrganismFactory` + 食物索引/腐败
- **3**（10-15）`Mutation` + `Species`/`SpeciesRegistry` + `FossilRecord` + `Charts`。72 项，0.28 ms/tick
- **4**（10-15）`Sensors` + `Brain` + `think()` + `Inspector`。76+51 项，2.60 ms/tick（种群 1500）；**杀手-移动器 5–6× 富集**，繁荣-崩溃周期。遗留：眼睛仅 0.0–0.36%
- **5**（10-15）`ControlPanel` + `SaveLoad` + `Editor` 克隆笔刷 + `main.js` 装配 + config 元数据表。64+60 项。交付单文件 + exe
- **6**（10-15）`SpeciesTree` + `Export` + 着色模式 + 相机跟随 + 整物种高亮。60+50 项。另修"缩小时世界重复平铺"
- **7**（10-16）细胞注册表 + 生命值体系 + 物种工坊 + 检视器非模态浮窗/图钉/保存生物。89+41 项，0.657 ms/tick
- **7 补充**（10-16）物种树浮窗化（修"占满空间、看不到高亮"）+ 渲染循环常驻（修"暂停后准星冻结"）+ 工坊手动放置 + 导出落程序目录（exe 写盘端点）。cdp-phase5 66 项 / phase6 61 项 / phase7 48 项；verify-bundle 32 项；verify-exe 17 项
- **7 补充②**（10-16）参数新增**食物量倍率** + 既有 `metabolismScale` 改名**全局消耗量倍率**；笔刷新增**橡皮擦**（只清地形）。phase7-test 98 项 / cdp-phase5 78 项 / verify-bundle 35 项；回归 phase3–6 与 cdp-phase3/4/6/7 全绿
- 下一步：阶段 8（性能剖析、TypedArray 调优、预设世界模板）+ 收尾"眼睛一直稀有"。**扩展建议见 DESIGN.md §15**（`onEnter` 钩子 + 岩浆/沼泽/辐射/腐殖质 → 储食/再生/刺细胞）

## 验证方式

- 逻辑层：`tools/phase3-test.mjs` … `phase7-test.mjs`（node 直接跑）
- 参数扫描：`phase3-sweep.mjs`（变异率）/ `phase7-sweep.mjs`（代谢消耗）
- 生态测量：`phase4-ecology.mjs`（非断言式，看富集倍数/变异系数/自相关）
- 端到端：`tools/cdp-phase3/4/5/6/7.mjs`（http server + 无头 Chrome + CDP）
- 产物验证：`verify-bundle.mjs [URL]`（不传参走 `file://` 开 dist/evosim.html，传 URL 开 exe 服务）；`verify-exe.mjs`（起真 exe，验 `/__evosim/ping` 与 `/__evosim/save` 真写盘 + 路径穿越清洗）
- 打包：`bundle.mjs`（→ dist/evosim.html）、`build-exe.mjs`（→ dist/evosim.exe，Node SEA）

### CDP 硬规矩
- **每次从干净 profile 开始**：`http.server` 不发 `Cache-Control`，Chrome 启发式缓存 JS，复用 profile 会拿到上一版代码。`resetProfile()` 重试 8 次（`Atomics.wait` 同步小睡），仍失败换带 pid 的新目录
- **断言不要硬编码阶段号**，统一 `/^阶段 \d/`（踩过三次）
- **必须自己管住 Chrome 进程**：`chrome.kill()` 只发 SIGTERM，Windows 上常被忽略、留子进程**继续占调试端口**。后果极隐蔽——新 Chrome 绑不上端口，`fetch('/json/list')` 连到**旧实例页面**，测试在别人的页面里跑。所以：① 启动前 `freePort(PORT)` 杀残留；② 结束用 `taskkill /F /T /PID`；③ 加"启动时没有预置状态"哨兵断言
- **不要返回带循环引用的对象**（如 `renderer.highlight` 是带 world/grid 的 Organism）——`returnByValue` 报 `Object reference chain is too long`，只返回 `id`
- **鼠标事件会被浮层拦截**：用 `document.elementFromPoint(x,y)` 确认点击目标
- **无头浏览器 `fps`/`tps` 会抖动**：多次采样取最优
- **读不了图片时**：Chrome 打开图片 → canvas → `getImageData` → 降采样成 ASCII 打印
- 调试入口 `window.evosim`：Config/world/camera/renderer/editor/engine/colorScheme/rng/record/charts/stats/inspector/toolbar/controlPanel/timeline/speciesTree/library/speciesEditor + `save/load/quickSave/quickLoad/quickInfo/storageAvailable/exportFossil/exportSpecies/saveText/hostSaveDir/toast`
