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
    independent: true, paintable: true, visible: true,
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
