import { CellType } from '../life/CellType.js';

/**
 * 网格数据结构：结构体数组（SoA）+ TypedArray。
 *
 * 不用对象数组——20 万格的对象开销太大。
 *   types  : Uint8Array   细胞类型枚举
 *   owners : Int32Array   所属生物 id，-1 表示独立格
 *
 * 索引公式：idx = row * cols + col
 * 越界统一返回 -1 / EMPTY，边界处理全部收在这里。
 */
export class GridMap {
  constructor(cols, rows, wrap = true) {
    this.resize(cols, rows, wrap);
  }

  resize(cols, rows, wrap = this.wrap) {
    this.cols = cols;
    this.rows = rows;
    this.wrap = wrap;
    this.length = cols * rows;
    this.types = new Uint8Array(this.length);
    this.owners = new Int32Array(this.length).fill(-1);
  }

  /** 世界坐标 → 扁平索引；越界且不环绕时返回 -1 */
  index(col, row) {
    if (this.wrap) {
      col = ((col % this.cols) + this.cols) % this.cols;
      row = ((row % this.rows) + this.rows) % this.rows;
    } else if (col < 0 || row < 0 || col >= this.cols || row >= this.rows) {
      return -1;
    }
    return row * this.cols + col;
  }

  /** 扁平索引 → [col, row] */
  coords(index) {
    const col = index % this.cols;
    const row = (index / this.cols) | 0;
    return [col, row];
  }

  inBounds(col, row) {
    return col >= 0 && row >= 0 && col < this.cols && row < this.rows;
  }

  typeAt(col, row) {
    const i = this.index(col, row);
    return i < 0 ? CellType.EMPTY : this.types[i];
  }

  ownerAt(col, row) {
    const i = this.index(col, row);
    return i < 0 ? -1 : this.owners[i];
  }

  typeAtIndex(index) {
    return index < 0 ? CellType.EMPTY : this.types[index];
  }

  set(index, type, ownerId = -1) {
    if (index < 0) return false;
    this.types[index] = type;
    this.owners[index] = ownerId;
    return true;
  }

  /** 把整张网格填成同一种类型（保留墙体可选） */
  fill(type, keepWalls = false) {
    if (!keepWalls) {
      this.types.fill(type);
      this.owners.fill(-1);
      return;
    }
    for (let i = 0; i < this.length; i++) {
      if (this.types[i] !== CellType.WALL) {
        this.types[i] = type;
        this.owners[i] = -1;
      }
    }
  }

  /** 统计各类型格子数量，返回 Uint32Array(按类型 id 索引) */
  countByType() {
    const counts = new Uint32Array(16);
    for (let i = 0; i < this.length; i++) counts[this.types[i]]++;
    return counts;
  }

  /** 统计非空格子数量 */
  countNonEmpty() {
    let n = 0;
    for (let i = 0; i < this.length; i++) {
      if (this.types[i] !== CellType.EMPTY) n++;
    }
    return n;
  }

  /** 遍历某个矩形范围内的格子，回调 (col, row, index) */
  forEachInRect(c0, r0, c1, r1, callback) {
    for (let row = r0; row <= r1; row++) {
      for (let col = c0; col <= c1; col++) {
        const i = this.index(col, row);
        if (i >= 0) callback(col, row, i);
      }
    }
  }
}
