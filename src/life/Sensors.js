import { CellType, DIR_VECTORS, VISIBLE_CELL_TYPES, cellDef, isDirectional, cellAttr } from './CellType.js';

/**
 * 感知编码 —— 世界 → 输入向量（设计文档 §6.1）。
 *
 * 输入向量 40 维：
 *
 *   [ 0 .. 35]  视觉：4 个相对方向 × 9 维
 *                 每方向 = 8 种细胞类型 one-hot + 1 个归一化距离
 *   [36]        能量比例 = min(energy / foodNeeded, 1)
 *   [37]        年龄比例 = min(age / lifespan, 1)
 *   [38]        移动能力 = hasMover ? 1 : 0
 *   [39]        偏置常量 = 1
 *
 * **眼睛决定"能看哪个方向"**：眼睛细胞的 dir 指向某个相对方向，
 * 只有那个方向的 9 个槽会被填充。没有眼睛指向的方向全 0。
 * 因此"长眼睛"是一项有实际收益的投资——身体结构与智力在这里耦合。
 */

export const VISION_DIRS = 4;         // 四个相对方向
export const VISUAL_SLOTS = 9;        // 每方向：8 类型 one-hot + 1 距离
export const VISION_DIM = VISION_DIRS * VISUAL_SLOTS;   // 36
export const STATE_BASE = VISION_DIM;                   // 36
export const SENSE_DIM = 40;

/** 相对方向名（0 是身体正面） */
export const DIR_LABELS = ['前', '右', '后', '左'];

/**
 * 参与视觉的类型（顺序即输入槽顺序）。
 * 由注册表里 `visible: true` 的定义决定——**不要随意重排**，
 * 顺序一变，所有旧大脑的输入语义就跟着错位。
 */
export const SEEN_TYPES = VISIBLE_CELL_TYPES;

export const SEEN_TYPE_LABELS = SEEN_TYPES.map((t) => cellDef(t).label);

/** 细胞类型 id → 视觉槽序号（-1 = 不参与视觉） */
const VISION_SLOT = (() => {
  const arr = new Int8Array(64).fill(-1);
  for (let i = 0; i < SEEN_TYPES.length; i++) arr[SEEN_TYPES[i]] = i;
  return arr;
})();

/** 40 个输入槽的可读名，供检视器显示 */
export const INPUT_LABELS = (() => {
  const labels = [];
  for (let d = 0; d < VISION_DIRS; d++) {
    for (let t = 0; t < SEEN_TYPES.length; t++) labels.push(`${DIR_LABELS[d]}·${SEEN_TYPE_LABELS[t]}`);
    labels.push(`${DIR_LABELS[d]}·距离`);
  }
  labels.push('能量', '年龄', '会移动', '偏置');
  return labels;
})();

export class Sensors {
  constructor(size = SENSE_DIM) {
    this.size = size;
    this.values = new Float32Array(size);
  }

  /**
   * 填充输入向量。就地改写并返回内部缓冲（复用，不分配）。
   */
  sense(organism) {
    const values = this.values;
    values.fill(0);

    const world = organism.world;
    const grid = world.grid;
    const config = world.config;
    const globalRange = config.sense.lookRange;
    const seeThroughSelf = config.sense.seeThroughSelf;
    const rotation = organism.rotation;
    const cells = organism.anatomy.cells;

    for (let i = 0; i < cells.length; i++) {
      const cell = cells[i];
      if (!isDirectional(cell.type)) continue;

      // 每个眼睛可以有自己的视野（细胞类型属性，0 表示用全局值）
      const range = cellAttr(config, cell.type, 'senseRange') || globalRange;
      if (range <= 0) continue;

      const localDir = cell.dir & 3;
      const base = localDir * VISUAL_SLOTS;
      if (base + VISUAL_SLOTS > this.size) continue;

      const v = DIR_VECTORS[(rotation + localDir) & 3];
      let col = organism.realCol(cell);
      let row = organism.realRow(cell);

      let seenType = -1;
      let seenDist = 0;

      for (let step = 1; step <= range; step++) {
        col += v[0];
        row += v[1];
        const index = grid.index(col, row);
        if (index < 0) break;                       // 越界（非环绕世界）

        const type = grid.types[index];
        if (type === CellType.EMPTY) continue;      // 空气，继续看

        if (grid.owners[index] === organism.id) {
          // 看到的是自己的身体
          if (seeThroughSelf) continue;             // 透视自身 → 穿过去继续看
          break;                                    // 否则被自己挡住，什么都看不到
        }

        seenType = type;
        seenDist = step;
        break;                                      // 第一个非空格就是观测目标
      }

      if (seenType < 0) continue;
      const slot = seenType < VISION_SLOT.length ? VISION_SLOT[seenType] : -1;
      if (slot < 0) continue;                       // 该类型不参与视觉，当作没看见

      // 同一个方向可能有多个眼睛。取"更近的那个"——更近的观测信息量更大，
      // 而且让多只眼睛指向同方向是有意义的冗余（看得更全）。
      const closeness = 1 - seenDist / range;
      if (closeness > values[base + VISUAL_SLOTS - 1]) {
        for (let t = 0; t < SEEN_TYPES.length; t++) values[base + t] = 0;
        values[base + slot] = 1;
        values[base + VISUAL_SLOTS - 1] = closeness;
      }
    }

    // ── 自身状态 ──
    if (this.size > STATE_BASE) values[STATE_BASE] = organism.energyRatio;
    if (this.size > STATE_BASE + 1) values[STATE_BASE + 1] = organism.ageRatio;
    if (this.size > STATE_BASE + 2) values[STATE_BASE + 2] = organism.anatomy.hasMover ? 1 : 0;
    if (this.size > STATE_BASE + 3) values[STATE_BASE + 3] = 1;

    return values;
  }
}
