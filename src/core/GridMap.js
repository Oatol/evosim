import { CellType, TERRAIN_MASK } from '../life/CellType.js';

/**
 * 网格数据结构：结构体数组（SoA）+ TypedArray。
 *
 * 不用对象数组——20 万格的对象开销太大。
 *   types  : Uint8Array   细胞类型枚举（**顶层**：可能被生物的身体盖住）
 *   owners : Int32Array   所属生物 id，-1 表示独立格
 *   floor  : Uint8Array   **地板层**：这一格的地形是什么
 *
 * 索引公式：idx = row * cols + col
 * 越界统一返回 -1 / EMPTY，边界处理全部收在这里。
 *
 * ── 为什么要单独一层 floor（阶段 8）──
 *
 * 生物写格子时会把 `types` 覆盖成自己的细胞类型。如果地形只存在 `types` 里，
 * 那么一只生物从岩浆上走过，岩浆就被"抹掉"了——地形会被生物啃光，
 * 岩浆/辐射带/腐殖质这类需要长期存在的地形根本立不住。
 *
 * 所以地形单独记在 `floor` 里：生物盖上去只改 `types`，`floor` 不动；
 * 生物离开时调用 `World.revealFloor()` 把地板重新铺回顶层。
 * **只有玩家显式画"空"/用橡皮擦才会真正清掉地板。**
 *
 * 食物**不在** floor 里——它是资源不是地形，被生物压过去就该消失（既有行为）。
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
    this.floor = new Uint8Array(this.length);
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

  /** 把整张网格填成同一种类型。`keepTerrain` 为真时保留 floor 层里的地形 */
  fill(type, keepTerrain = false) {
    if (!keepTerrain) {
      this.types.fill(type);
      this.owners.fill(-1);
      // 填成地形类型时地板跟着走，否则地板清空
      this.floor.fill(TERRAIN_MASK[type] ? type : CellType.EMPTY);
      return;
    }
    // 保留地形：地板层是权威，把它铺回顶层（顺便把被生物盖住的格子还原）
    for (let i = 0; i < this.length; i++) {
      const f = this.floor[i];
      this.types[i] = TERRAIN_MASK[f] ? f : type;
      this.owners[i] = -1;
    }
  }

  /** 统计各类型格子数量，返回 Uint32Array(按类型 id 索引) */
  countByType() {
    const counts = new Uint32Array(32);
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
