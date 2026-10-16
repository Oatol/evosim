/**
 * 细胞类型注册表 —— 每种细胞 = 一个定义对象。
 *
 * 想加一种新细胞，只需要往 `CELL_DEFS` 里加一项（需要新行为的话，
 * 再在 BodyCell 的行为表里加一个同名函数）。网格读写、渲染配色、感知编码、
 * 结构变异、存档、参数面板全部自动认得它——因为它们都从这里读，不再各写一份。
 *
 * 定义对象里的字段分三类：
 *
 *   · 结构（决定"它是什么"）
 *       id        类型 id。**一旦定下就不能再改**：它写进存档，也写进网格的 Uint8
 *       key       稳定英文键（Config 里的属性表、存档迁移都用它）
 *       name      存档/CSV 里的英文名
 *       label     界面上的中文名
 *       isBodyCell   是不是生物身体的一部分
 *       independent  独立格（可被画笔绘制、可进存档）
 *       paintable    画笔可选
 *       visible      参与大脑的视觉输入（固定 8 类，改这个会让旧大脑不兼容）
 *       directional  带朝向（眼睛；朝向参与物种指纹）
 *       flag         身体上"有它就置 true"的能力标志位名
 *       behavior     BodyCell 行为表里的函数名
 *
 *   · 属性**默认值**（运行时以 Config.cells[key] 为准，这里只是出厂默认）
 *       hp         生命值。个体 maxHealth = 所有细胞 hp 之和
 *       cost       每 tick 能量消耗。个体 upkeep = 所有细胞 cost 之和
 *       damage     每 tick 对相邻其他生物造成的伤害
 *       senseRange 感知距离；0 表示用全局 sense.lookRange
 *       foodValue  死亡时该细胞转化出的食物量（>1 的部分投放到周围空格）
 *
 *   · 表现
 *       colors     三套配色各一个颜色
 */

export const CellType = Object.freeze({
  // 独立格
  EMPTY: 0,
  FOOD: 1,
  WALL: 2,

  // 生物细胞
  MOUTH: 3,      // 嘴：吃相邻食物
  PRODUCER: 4,   // 生产者：在相邻空格生成食物
  MOVER: 5,      // 移动器：赋予移动能力
  KILLER: 6,     // 杀手：伤害相邻其他生物
  ARMOR: 7,      // 装甲：免疫杀手伤害
  EYE: 8,        // 眼睛：赋予感知能力（带朝向）

  // 地形（阶段 8）
  LAVA: 9,       // 岩浆：踏入与驻留都灼伤
  SWAMP: 10,     // 沼泽：移动要额外耗能
  RAD: 11,       // 辐射带：驻留扣血 + 变异率飙升
  HUMUS: 12,     // 腐殖质：缓慢向周围释放食物

  // 身体细胞（阶段 9）
  STORAGE: 13,   // 储食：抬高繁殖阈值，多出来的能量变成储备
  REGEN: 14,     // 再生：每 tick 回一点血
  SPIKE: 15,     // 刺：被杀手攻击时反伤
});

/** 所有定义共用的默认值 */
const CELL_DEFAULTS = Object.freeze({
  isBodyCell: false,
  independent: false,
  paintable: false,
  visible: false,
  directional: false,
  flag: null,
  behavior: null,

  // 地形相关（阶段 8）
  terrain: false,     // 算不算"地形"：写进 floor 层、重置时可保留、橡皮擦的目标
  passable: false,    // 生物能否走进去
  enter: null,        // 踏入瞬间的行为名（见 Terrain.TERRAIN_ENTER）
  stand: null,        // 驻留每 tick 的行为名（见 Terrain.TERRAIN_STAND）
  tick: null,         // 世界级每 tick 的行为名（见 Terrain.TERRAIN_TICK，需索引）

  hp: 1,
  cost: 0,
  damage: 0,
  senseRange: 0,
  foodValue: 1,

  colors: null,
});

function def(spec) {
  return Object.freeze({ ...CELL_DEFAULTS, ...spec });
}

/**
 * 全部细胞类型。
 *
 * 数组下标 **不保证**等于 id（虽然目前是），一律用 CELL_DEF_BY_ID 查。
 */
export const CELL_DEFS = Object.freeze([
  def({
    id: CellType.EMPTY, key: 'EMPTY', name: 'empty', label: '空',
    independent: true, paintable: true,
    colors: { classic: '#0b1020', highContrast: '#000000', colorblindSafe: '#0d1117' },
  }),
  def({
    id: CellType.FOOD, key: 'FOOD', name: 'food', label: '食物',
    independent: true, paintable: true, visible: true,
    colors: { classic: '#7d93a8', highContrast: '#c8d4e0', colorblindSafe: '#9aa7b4' },
  }),
  def({
    id: CellType.WALL, key: 'WALL', name: 'wall', label: '墙',
    independent: true, paintable: true, visible: true, terrain: true,
    colors: { classic: '#3a4150', highContrast: '#5c6470', colorblindSafe: '#454c57' },
  }),
  def({
    id: CellType.MOUTH, key: 'MOUTH', name: 'mouth', label: '嘴',
    isBodyCell: true, paintable: true, visible: true,
    flag: 'hasMouth', behavior: 'mouth',
    cost: 0.002,
    colors: { classic: '#e07b39', highContrast: '#ff9500', colorblindSafe: '#e69f00' },
  }),
  def({
    id: CellType.PRODUCER, key: 'PRODUCER', name: 'producer', label: '生产者',
    isBodyCell: true, paintable: true, visible: true,
    flag: 'hasProducer', behavior: 'producer',
    cost: 0.002,
    colors: { classic: '#4caf50', highContrast: '#00e676', colorblindSafe: '#009e73' },
  }),
  def({
    id: CellType.MOVER, key: 'MOVER', name: 'mover', label: '移动器',
    isBodyCell: true, paintable: true, visible: true,
    flag: 'hasMover',
    cost: 0.008,
    colors: { classic: '#38bdf8', highContrast: '#00b0ff', colorblindSafe: '#56b4e9' },
  }),
  def({
    id: CellType.KILLER, key: 'KILLER', name: 'killer', label: '杀手',
    isBodyCell: true, paintable: true, visible: true,
    flag: 'hasKiller', behavior: 'killer',
    damage: 1,
    cost: 0.006,
    colors: { classic: '#ef4444', highContrast: '#ff1744', colorblindSafe: '#d55e00' },
  }),
  def({
    id: CellType.ARMOR, key: 'ARMOR', name: 'armor', label: '装甲',
    isBodyCell: true, paintable: true, visible: true,
    flag: 'hasArmor',
    cost: 0.004,
    colors: { classic: '#a855f7', highContrast: '#d500f9', colorblindSafe: '#cc79a7' },
  }),
  def({
    id: CellType.EYE, key: 'EYE', name: 'eye', label: '眼睛',
    isBodyCell: true, paintable: true, visible: true, directional: true,
    flag: 'hasEye', behavior: 'eye',
    cost: 0.006,
    colors: { classic: '#d8b4fe', highContrast: '#ffffff', colorblindSafe: '#f0e442' },
  }),

  // ── 身体细胞（阶段 9）──
  //
  // 这三种都**不参与视觉**（`visible` 默认 false）：感知扩容要动 VISUAL_SLOTS /
  // SENSE_DIM，会让旧存档大脑失效，所以留到最后一批统一做。
  // 在那之前，它们只是"身体的一部分"，别的生物看不见——但照样占格子、照样有成本。
  def({
    id: CellType.STORAGE, key: 'STORAGE', name: 'storage', label: '储食',
    isBodyCell: true, paintable: true,
    flag: 'hasStorage',
    cost: 0.004,
    // 脂肪是热量密度最高的组织：死掉之后尸体格外肥，这也是"养肥了再吃"的由来
    foodValue: 3,
    colors: { classic: '#fbbf24', highContrast: '#ffd600', colorblindSafe: '#f0c000' },
  }),
  def({
    id: CellType.REGEN, key: 'REGEN', name: 'regen', label: '再生',
    isBodyCell: true, paintable: true,
    flag: 'hasRegen', behavior: 'regen',
    cost: 0.005,
    colors: { classic: '#5eead4', highContrast: '#00ffc8', colorblindSafe: '#44d7b6' },
  }),
  def({
    id: CellType.SPIKE, key: 'SPIKE', name: 'spike', label: '刺',
    isBodyCell: true, paintable: true,
    flag: 'hasSpike',
    damage: 1,          // 被攻击时反弹给攻击者的伤害（和杀手共用 damage 属性）
    cost: 0.004,
    colors: { classic: '#94a3b8', highContrast: '#e0e6ee', colorblindSafe: '#b8c0cc' },
  }),

  // ── 地形（阶段 8）──
  //
  // 地形的四个关键点：
  //   1. `terrain: true` → 写进 GridMap 的 **floor 层**，所以生物从上面走过
  //      不会把它抹掉（见 GridMap.floor 的注释）
  //   2. `passable: true` → 生物能走进去；`false` 就是墙那样的实心障碍
  //   3. `enter` / `stand` → 生物踏入 / 驻留时触发 Terrain 表里的函数
  //   4. `visible` 默认 false：**这一批先不让生物"看见"地形**，
  //      感知扩容（VISUAL_SLOTS 9→N）留到最后一批，那时会一并作废旧存档
  def({
    id: CellType.LAVA, key: 'LAVA', name: 'lava', label: '岩浆',
    independent: true, paintable: true, terrain: true, passable: true,
    enter: 'burn', stand: 'burn',
    colors: { classic: '#ff6a00', highContrast: '#ff2d00', colorblindSafe: '#e05a00' },
  }),
  def({
    id: CellType.SWAMP, key: 'SWAMP', name: 'swamp', label: '沼泽',
    independent: true, paintable: true, terrain: true, passable: true,
    enter: 'bog',
    colors: { classic: '#3f5540', highContrast: '#5a7d5a', colorblindSafe: '#4d6b4d' },
  }),
  def({
    id: CellType.RAD, key: 'RAD', name: 'rad', label: '辐射带',
    independent: true, paintable: true, terrain: true, passable: true,
    stand: 'irradiate',
    colors: { classic: '#d4f000', highContrast: '#eeff00', colorblindSafe: '#c8d400' },
  }),
  def({
    id: CellType.HUMUS, key: 'HUMUS', name: 'humus', label: '腐殖质',
    independent: true, paintable: true, terrain: true, passable: true,
    tick: 'fertilize',
    colors: { classic: '#6b4f2a', highContrast: '#8d6e3a', colorblindSafe: '#7a5c33' },
  }),
]);

// ────────────────────────────── 索引 ──────────────────────────────

/** id → 定义（稀疏数组，按 id 直接查） */
export const CELL_DEF_BY_ID = (() => {
  const arr = [];
  for (const d of CELL_DEFS) arr[d.id] = d;
  return Object.freeze(arr);
})();

/** key → 定义 */
export const CELL_DEF_BY_KEY = Object.freeze(
  Object.fromEntries(CELL_DEFS.map((d) => [d.key, d])),
);

export const CELL_TYPE_COUNT = CELL_DEFS.length;

/** 兼容旧名：类型定义表（按 id 排列） */
export const CELL_TYPES = CELL_DEFS;
export const CELL_TYPE_BY_ID = CELL_DEF_BY_ID;

// ────────────────────────────── 派生列表 ──────────────────────────────

/** 可被随机生成的生物细胞类型（结构变异时用） */
export const LIVING_CELL_TYPES = Object.freeze(
  CELL_DEFS.filter((d) => d.isBodyCell).map((d) => d.id),
);

/** 独立格类型（可被画笔绘制、可被存档） */
export const INDEPENDENT_CELL_TYPES = Object.freeze(
  CELL_DEFS.filter((d) => d.independent).map((d) => d.id),
);

/** 可被画笔绘制的所有类型 */
export const PAINTABLE_CELL_TYPES = Object.freeze(
  CELL_DEFS.filter((d) => d.paintable).map((d) => d.id),
);

/** 参与大脑视觉输入的类型（顺序即输入槽顺序，**不要随意重排**） */
export const VISIBLE_CELL_TYPES = Object.freeze(
  CELL_DEFS.filter((d) => d.visible).map((d) => d.id),
);

/** 所有"能力标志位"的名字（Anatomy 用它初始化和清空） */
export const CELL_FLAG_NAMES = Object.freeze(
  CELL_DEFS.filter((d) => d.flag).map((d) => d.flag),
);

/** 带朝向的细胞类型（渲染画朝向细缝用） */
export const DIRECTIONAL_CELL_TYPES = Object.freeze(
  CELL_DEFS.filter((d) => d.directional).map((d) => d.id),
);

/** 地形类型（写进 floor 层；重置时可保留；橡皮擦的目标） */
export const TERRAIN_CELL_TYPES = Object.freeze(
  CELL_DEFS.filter((d) => d.terrain).map((d) => d.id),
);

/**
 * 可直接在世界画布上绘制的地形 —— 工具栏的笔刷列表由它派生。
 *
 * 注意不能用 `PAINTABLE_CELL_TYPES`：那个还包含身体细胞（嘴/生产者/…），
 * 而身体细胞只能在物种工坊的蓝图里画，不该出现在世界画笔里。
 */
export const TERRAIN_BRUSH_TYPES = Object.freeze(
  CELL_DEFS.filter((d) => d.independent && d.paintable).map((d) => d.id),
);

/** 有"世界级每 tick 行为"的地形（World 会给它们维护索引，避免每 tick 扫全网格） */
export const TICKING_TERRAIN_TYPES = Object.freeze(
  CELL_DEFS.filter((d) => d.tick).map((d) => d.id),
);

/** id → 是不是地形（热路径用，按 id 直接索引的查表） */
export const TERRAIN_MASK = (() => {
  const m = new Uint8Array(256);
  for (const d of CELL_DEFS) if (d.terrain) m[d.id] = 1;
  return m;
})();

/** id → 是不是"需要每 tick 更新"的地形 */
export const TICKS_TERRAIN = (() => {
  const m = new Uint8Array(256);
  for (const d of CELL_DEFS) if (d.tick) m[d.id] = 1;
  return m;
})();

// ────────────────────────────── 查询 ──────────────────────────────

export function cellDef(id) {
  return CELL_DEF_BY_ID[id] || null;
}

export function cellTypeLabel(id) {
  const d = CELL_DEF_BY_ID[id];
  return d ? d.label : '未知';
}

export function isBodyCell(id) {
  const d = CELL_DEF_BY_ID[id];
  return !!(d && d.isBodyCell);
}

export function isDirectional(id) {
  const d = CELL_DEF_BY_ID[id];
  return !!(d && d.directional);
}

/** 是不是地形（墙 / 岩浆 / 沼泽 / 辐射带 / 腐殖质…） */
export function isTerrain(id) {
  const d = CELL_DEF_BY_ID[id];
  return !!(d && d.terrain);
}

/**
 * 生物能否走进这一格。
 *
 * **只对地形生效**（`terrain: true` 的类型）。空地和食物不算地形，它们的通行性
 * 由调用方单独判断——食物是否阻挡取决于 `combat.foodBlocksReproduction`。
 * 这样"食物"这条特例只有一处管，不会和这里打架
 * （踩过的坑：一开始给 FOOD 也标了 `passable: true`，结果把
 *  `foodBlocksReproduction = true` 直接架空了，读档回归测试立刻分叉）。
 */
export function isPassable(id) {
  const d = CELL_DEF_BY_ID[id];
  return !!(d && d.terrain && d.passable);
}

/**
 * 读一个细胞类型的某项属性。
 *
 * 运行时值优先从 `config.cells[KEY][name]` 取（参数面板改的就是它），
 * 取不到才退回定义对象里的出厂默认——所以就算传进来一个没有 cells
 * 段的旧配置，也不会读出一个 undefined。
 */
export function cellAttr(config, typeId, name) {
  const d = CELL_DEF_BY_ID[typeId];
  if (!d) return 0;
  const table = config && config.cells;
  if (table) {
    const row = table[d.key];
    if (row && row[name] !== undefined) return row[name];
  }
  const v = d[name];
  return v === undefined ? 0 : v;
}

/** 某类型的属性默认值（config.js 用它初始化 Config.cells） */
export function cellAttrDefaults(id) {
  const d = CELL_DEF_BY_ID[id];
  if (!d) return null;
  return {
    hp: d.hp,
    cost: d.cost,
    damage: d.damage,
    senseRange: d.senseRange,
    foodValue: d.foodValue,
  };
}

/** 属性字段的元数据（参数面板与"恢复默认"共用一份定义） */
export const CELL_ATTR_FIELDS = Object.freeze([
  { name: 'hp', label: '生命值', min: 0, max: 20, step: 1, note: '个体生命值 = 所有细胞生命值之和' },
  { name: 'cost', label: '每tick消耗', min: 0, max: 1, step: 0.005 },
  { name: 'damage', label: '伤害', min: 0, max: 10, step: 1 },
  { name: 'senseRange', label: '感知距离', min: 0, max: 60, step: 1, note: '0 = 用全局视野范围' },
  { name: 'foodValue', label: '死亡食物值', min: 0, max: 20, step: 1, note: '死亡时该细胞转化出的食物量' },
]);

// ────────────────────────────── 方向 ──────────────────────────────

/** 方向：0=上 1=右 2=下 3=左 */
export const DIR = Object.freeze({ UP: 0, RIGHT: 1, DOWN: 2, LEFT: 3 });

/** 方向 → [列增量, 行增量] */
export const DIR_VECTORS = Object.freeze([
  [0, -1],
  [1, 0],
  [0, 1],
  [-1, 0],
]);

export function oppositeDir(dir) {
  return (dir + 2) & 3;
}

export function dirVector(dir) {
  return DIR_VECTORS[dir & 3];
}
