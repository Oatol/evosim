import { CELL_TYPE_COUNT } from '../life/CellType.js';

/**
 * 化石记录 —— 设计文档 §7.3。
 *
 * 定长环形缓冲：写满之后新样本覆盖最旧的，内存恒定。
 * 每 statsInterval 采样一次，所以记录长度 2000 对应 20 万 tick 的历史。
 *
 * 用 Float64Array 而不是普通数组：采样点固定、无对象分配、
 * 而且 Charts 画曲线时可以直接按索引读，不必走对象属性。
 */
export class FossilRecord {
  constructor(size = 2000, maxPoints = 512) {
    this._maxPoints = maxPoints;
    this._bufT = new Float64Array(maxPoints);
    this._bufV = new Float64Array(maxPoints);
    this.allocate(size);
  }

  allocate(size) {
    this.size = Math.max(1, size | 0);
    this.head = 0;      // 最旧样本的物理位置
    this.count = 0;     // 有效样本数

    this.ticks = new Float64Array(this.size);
    this.populations = new Float64Array(this.size);
    this.species = new Float64Array(this.size);
    this.avgCells = new Float64Array(this.size);
    this.avgMutability = new Float64Array(this.size);

    // 每种细胞类型在生物体内的平均占比，布局为 [type * size + slot]
    this.shares = new Float64Array(CELL_TYPE_COUNT * this.size);
  }

  get length() {
    return this.count;
  }

  /** 逻辑索引（0 = 最旧）→ 物理索引 */
  _phys(i) {
    return (this.head + i) % this.size;
  }

  push(sample) {
    let p;
    if (this.count < this.size) {
      p = (this.head + this.count) % this.size;
      this.count++;
    } else {
      p = this.head;
      this.head = (this.head + 1) % this.size;
    }

    this.ticks[p] = sample.tick;
    this.populations[p] = sample.population;
    this.species[p] = sample.species;
    this.avgCells[p] = sample.avgCells;
    this.avgMutability[p] = sample.avgMutability;

    const shares = sample.cellShares;
    if (shares) {
      for (let t = 0; t < CELL_TYPE_COUNT; t++) {
        this.shares[t * this.size + p] = shares[t];
      }
    }

    return p;
  }

  /** 最新一个样本的某个字段 */
  latest(key) {
    if (this.count === 0) return 0;
    return this[key][this._phys(this.count - 1)];
  }

  /**
   * 取第 i 个样本（0 = 最旧）的完整快照，供导出/分析用。
   *
   * 这里返回的是新对象（含一份 shares 拷贝），与 extract() 的"复用缓冲"
   * 策略相反——导出是低频操作，可读性比零分配重要。
   */
  sampleAt(i) {
    if (i < 0 || i >= this.count) return null;
    const p = this._phys(i);
    const shares = new Float64Array(CELL_TYPE_COUNT);
    for (let t = 0; t < CELL_TYPE_COUNT; t++) shares[t] = this.shares[t * this.size + p];
    return {
      tick: this.ticks[p],
      population: this.populations[p],
      species: this.species[p],
      avgCells: this.avgCells[p],
      avgMutability: this.avgMutability[p],
      shares,
    };
  }

  /**
   * 取一条曲线（按时间顺序）。
   *
   * 采样点多于 maxPoints 时按 stride 抽稀——画在 200px 宽的 canvas 上，
   * 多出来的点只会落在同一个像素里，白画。
   * 无论怎么抽稀，**最后一个点一定保留**，否则读数会落后于真实值。
   *
   * 返回的缓冲是复用的，调用方不要长期持有。
   */
  extract(key, maxPoints = this._maxPoints) {
    const arr = this[key];
    const n = this.count;
    const cap = Math.min(maxPoints, this._maxPoints);
    if (n === 0 || !arr) return { ticks: this._bufT, values: this._bufV, n: 0 };

    const stride = Math.max(1, Math.ceil(n / cap));
    let m = 0;
    for (let i = 0; i < n; i += stride) {
      const p = (this.head + i) % this.size;
      this._bufT[m] = this.ticks[p];
      this._bufV[m] = arr[p];
      m++;
    }

    // 末尾对齐：把最后一个采样点补上（抽稀时它常常被跳过）
    const pLast = this._phys(n - 1);
    if (m === 0 || this._bufT[m - 1] !== this.ticks[pLast]) {
      if (m < cap) {
        this._bufT[m] = this.ticks[pLast];
        this._bufV[m] = arr[pLast];
        m++;
      } else {
        this._bufT[m - 1] = this.ticks[pLast];
        this._bufV[m - 1] = arr[pLast];
      }
    }

    return { ticks: this._bufT, values: this._bufV, n: m };
  }

  reset() {
    this.head = 0;
    this.count = 0;
    this.ticks.fill(0);
    this.populations.fill(0);
    this.species.fill(0);
    this.avgCells.fill(0);
    this.avgMutability.fill(0);
    this.shares.fill(0);
  }
}
