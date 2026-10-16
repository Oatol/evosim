import { CellType, cellDef, isDirectional, cellAttr } from './CellType.js';

/**
 * 身体细胞：记录它在母体蓝图里的相对位置，并定义它的被动行为。
 *
 * 注意「被动」二字——细胞不思考。它们只要存在就会持续起作用：
 * 嘴碰到食物就吃，杀手碰到别人就伤。大脑（阶段 4）只决定"去哪儿"。
 *
 * 行为本身不写死在 switch 里：注册表的定义对象带一个 `behavior` 名，
 * 这里用一张表把它映射到函数。加新细胞类型时，只需要在这张表里补一项
 * （复用已有行为的话，连这一项都不用加）。
 */

/** 四邻域：上 右 下 左 */
export const NEIGHBORS_4 = Object.freeze([[0, -1], [1, 0], [0, 1], [-1, 0]]);

/** 八邻域 */
export const NEIGHBORS_8 = Object.freeze([
  [0, -1], [1, 0], [0, 1], [-1, 0],
  [-1, -1], [1, 1], [-1, 1], [1, -1],
]);

export class BodyCell {
  constructor(type, dx, dy, dir = 0) {
    this.type = type;
    this.dx = dx;
    this.dy = dy;
    this.dir = dir;      // 局部朝向 0-3，仅带朝向的细胞（眼睛）使用
  }

  clone() {
    return new BodyCell(this.type, this.dx, this.dy, this.dir);
  }

  /** 用于物种指纹：类型 + 位置（朝向参与，因为眼睛朝向是结构的一部分） */
  signature() {
    return isDirectional(this.type)
      ? `${this.type}:${this.dx},${this.dy},${this.dir}`
      : `${this.type}:${this.dx},${this.dy}`;
  }
}

// ────────────────────────────── 旋转 ──────────────────────────────
//
// 局部坐标 (dx, dy) 绕中心枢轴顺时针旋转 rotation × 90°。
// 屏幕坐标 y 轴向下，所以顺时针 90° 的变换是 (x, y) → (-y, x)。
//
//   rotation 0 (上)  → ( dx,  dy)
//   rotation 1 (右)  → (-dy,  dx)
//   rotation 2 (下)  → (-dx, -dy)
//   rotation 3 (左)  → ( dy, -dx)

export function rotateCol(dx, dy, rotation) {
  switch (rotation & 3) {
    case 0: return dx;
    case 1: return -dy;
    case 2: return -dx;
    default: return dy;
  }
}

export function rotateRow(dx, dy, rotation) {
  switch (rotation & 3) {
    case 0: return dy;
    case 1: return dx;
    case 2: return -dy;
    default: return -dx;
  }
}

// ────────────────────────────── 行为 ──────────────────────────────

/** 行为名 → 函数。注册表的 `behavior` 字段指向这里的键。 */
export const CELL_BEHAVIORS = {
  mouth: mouthFunction,
  producer: producerFunction,
  killer: killerFunction,
  eye: eyeFunction,
};

/**
 * 执行一个细胞的被动行为。
 * 没有 `behavior` 的类型（移动器、装甲）什么都不做——
 * 移动器只是"允许移动"的标记，装甲只是"打不动"的格子。
 */
export function performCellFunction(organism, cell) {
  const d = cellDef(cell.type);
  if (!d || !d.behavior) return;
  const fn = CELL_BEHAVIORS[d.behavior];
  if (fn) fn(organism, cell);
}

/** 嘴：吃掉四邻域里的所有食物 */
function mouthFunction(organism, cell) {
  const world = organism.world;
  const grid = world.grid;
  const col = organism.realCol(cell);
  const row = organism.realRow(cell);

  // 一份食物能提供的能量由全局「食物量倍率」决定（默认 1）。
  // 这个数影响的是"吃进来多少"，不是"长出来多少"——所以它只在这里出现，
  // 生产者造食、死亡掉落都不受影响。
  const gain = foodEnergyGain(world.config);

  for (let i = 0; i < 4; i++) {
    const index = grid.index(col + NEIGHBORS_4[i][0], row + NEIGHBORS_4[i][1]);
    if (index < 0) continue;
    if (grid.types[index] === CellType.FOOD) {
      world.revealFloor(index);     // 吃掉的只是食物；底下有地形就露出来
      organism.energy += gain;
    }
  }
}

/**
 * 读「食物量倍率」。
 *
 * 兜底成 1：传进来的可能是没有这段的旧配置（存档迁移），
 * 也可能是被面板写成空值——任何一种都不该让进食变成 0 能量。
 */
export function foodEnergyGain(config) {
  const v = config && config.life ? config.life.foodEnergyScale : 1;
  return Number.isFinite(v) && v > 0 ? v : 1;
}

/** 生产者：以一定概率在某个相邻空格生成食物 */
function producerFunction(organism, cell) {
  const world = organism.world;
  const cfg = world.config.life;

  // 会动的生物默认不能用生产者细胞——这是"移动"要付的代价，
  // 也是生态系统里"生产者 vs 掠食者"分化的来源。
  if (organism.anatomy.hasMover && !cfg.moversCanProduce) return;

  const prob = cfg.foodProdProb;
  if (prob <= 0) return;
  if (!world.rng.chance(prob)) return;

  const grid = world.grid;
  const col = organism.realCol(cell);
  const row = organism.realRow(cell);
  const n = NEIGHBORS_4[world.rng.int(4)];
  const index = grid.index(col + n[0], row + n[1]);
  if (index < 0) return;
  if (grid.types[index] === CellType.EMPTY) {
    world.setCellAt(index, CellType.FOOD, -1);
  }
}

/**
 * 杀手：伤害四邻域里属于其他生物的身体细胞（装甲格免疫）。
 *
 * 伤害值来自该细胞类型的 `damage` 属性（可在参数面板里实时调），
 * 而不是写死的 1。
 */
function killerFunction(organism, cell) {
  const world = organism.world;
  const grid = world.grid;
  const damage = cellAttr(world.config, cell.type, 'damage');
  if (damage <= 0) return;

  const col = organism.realCol(cell);
  const row = organism.realRow(cell);

  for (let i = 0; i < 4; i++) {
    const index = grid.index(col + NEIGHBORS_4[i][0], row + NEIGHBORS_4[i][1]);
    if (index < 0) continue;

    const ownerId = grid.owners[index];
    if (ownerId < 0 || ownerId === organism.id) continue;
    if (grid.types[index] === CellType.ARMOR) continue;   // 装甲格本身打不动

    const target = world.organismById(ownerId);
    if (!target || !target.alive) continue;
    target.harm(damage);
  }
}

/**
 * 眼睛：感知能力在 Sensors 里实现（阶段 A 的 think() 统一采样），
 * 这里不需要做任何事——眼睛是**结构性能力**，不是"每 tick 的动作"。
 *
 * 保留这个分发分支，是为了让"眼睛也是一种身体细胞"这件事在代码里显式可见，
 * 而不是让它悄悄消失在 default 里。
 */
function eyeFunction(organism, cell) {
  // 感知在 Sensors.sense() 里做
}
