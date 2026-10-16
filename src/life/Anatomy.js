import { CELL_FLAG_NAMES, cellDef, isDirectional } from './CellType.js';
import { BodyCell } from './BodyCell.js';

/**
 * 身体蓝图 —— 基因组 A。
 *
 * 一个细胞列表，每项记录「类型 + 相对中心枢轴的坐标」。
 * 中心格 (0,0) 是旋转枢轴，永远存在。
 *
 * 允许出现空洞和断裂的细胞——参考项目视为特性，我们沿用：
 * 一个断开的手臂依然是身体的一部分，也依然要消耗寿命预算。
 *
 * 能力标志位（hasMover / hasEye / ...）不是手写 switch，而是从注册表
 * 的定义对象里读 `flag` 字段——加一种"有它就置位"的新细胞类型不用改这里。
 */
export class Anatomy {
  constructor() {
    this.cells = [];
    this.birthDistance = 4;

    // 派生缓存，任何结构变化后重算
    for (const name of CELL_FLAG_NAMES) this[name] = false;
  }

  get cellCount() {
    return this.cells.length;
  }

  clear() {
    this.cells.length = 0;
    this.birthDistance = 4;
    for (const name of CELL_FLAG_NAMES) this[name] = false;
  }

  /**
   * 添加一个细胞。位置已被占用时返回 null。
   * @param {number} dir 仅带朝向的细胞需要（局部朝向 0-3）
   */
  addCell(type, dx, dy, dir = 0) {
    if (this.cellAt(dx, dy)) return null;

    const cell = new BodyCell(type, dx, dy, isDirectional(type) ? (dir & 3) : 0);
    this.cells.push(cell);

    // 出生距离：保证子代不会和母体重叠
    const distance = Math.max(Math.abs(dy) * 2 + 2, Math.abs(dx) * 2 + 2);
    if (distance > this.birthDistance) this.birthDistance = distance;

    this._applyFlag(type, true);
    return cell;
  }

  /** 按局部坐标查细胞 */
  cellAt(dx, dy) {
    const cells = this.cells;
    for (let i = 0; i < cells.length; i++) {
      if (cells[i].dx === dx && cells[i].dy === dy) return cells[i];
    }
    return null;
  }

  /** 按局部坐标移除细胞。中心格 (0,0) 不可移除。 */
  removeCellAt(dx, dy) {
    if (dx === 0 && dy === 0) return null;
    const cells = this.cells;
    for (let i = 0; i < cells.length; i++) {
      if (cells[i].dx === dx && cells[i].dy === dy) {
        const removed = cells.splice(i, 1)[0];
        this.recompute();
        return removed;
      }
    }
    return null;
  }

  /** 删除一个随机细胞（中心格除外，且至少保留一个细胞） */
  removeRandomCell(rng) {
    if (this.cells.length <= 1) return null;
    for (let attempt = 0; attempt < 8; attempt++) {
      const cell = this.cells[rng.int(this.cells.length)];
      if (cell.dx === 0 && cell.dy === 0) continue;
      return this.removeCellAt(cell.dx, cell.dy);
    }
    // 兜底：找第一个非中心格
    for (const cell of this.cells) {
      if (cell.dx !== 0 && cell.dy !== 0) return this.removeCellAt(cell.dx, cell.dy);
    }
    return null;
  }

  /** 随机取一个细胞 */
  randomCell(rng) {
    return this.cells[rng.int(this.cells.length)];
  }

  /** 重算派生缓存 */
  recompute() {
    for (const name of CELL_FLAG_NAMES) this[name] = false;

    let birth = 4;
    const cells = this.cells;
    for (let i = 0; i < cells.length; i++) {
      const cell = cells[i];
      this._applyFlag(cell.type, true);
      const distance = Math.max(Math.abs(cell.dy) * 2 + 2, Math.abs(cell.dx) * 2 + 2);
      if (distance > birth) birth = distance;
    }
    this.birthDistance = birth;
    return this;
  }

  /** 按类型置位它的能力标志（定义对象没写 flag 的类型什么也不做） */
  _applyFlag(type, value) {
    const d = cellDef(type);
    if (d && d.flag) this[d.flag] = value;
  }

  /** 深拷贝（繁殖时用） */
  copyFrom(other) {
    this.clear();
    this.birthDistance = other.birthDistance;
    const cells = other.cells;
    for (let i = 0; i < cells.length; i++) {
      const src = cells[i];
      this.cells.push(new BodyCell(src.type, src.dx, src.dy, src.dir));
    }
    this.recompute();
    return this;
  }

  /** 最大半径（用于出生位置计算与渲染高亮） */
  get radius() {
    let max = 0;
    for (const cell of this.cells) {
      const d = Math.max(Math.abs(cell.dx), Math.abs(cell.dy));
      if (d > max) max = d;
    }
    return max;
  }

  /**
   * 结构指纹 —— 物种判据。
   *
   * 与位置无关（同一种身体在不同地方旋转放置算同一物种），
   * 与细胞列表顺序无关（排序后拼接）。
   * 权重变异不改变指纹，所以只有身体变了才算新物种。
   */
  fingerprint() {
    const parts = new Array(this.cells.length);
    for (let i = 0; i < this.cells.length; i++) parts[i] = this.cells[i].signature();
    parts.sort();
    return hashString(parts.join('|'));
  }

  /** 可读的结构描述，例如 "嘴×1 生产者×2 移动器×1" */
  describe() {
    const counts = new Map();
    for (const cell of this.cells) {
      counts.set(cell.type, (counts.get(cell.type) || 0) + 1);
    }
    return [...counts.entries()]
      .sort((a, b) => a[0] - b[0])
      .map(([type, n]) => {
        const d = cellDef(type);
        return `${d ? d.label : type}×${n}`;
      })
      .join(' ');
  }
}

/** FNV-1a 32 位哈希，输出 8 位十六进制 */
export function hashString(str) {
  let hash = 0x811c9dc5;
  for (let i = 0; i < str.length; i++) {
    hash ^= str.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193);
  }
  return (hash >>> 0).toString(16).padStart(8, '0');
}
