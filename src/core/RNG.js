/**
 * 可复现的伪随机数生成器（xorshift128）。
 *
 * 全部随机性都必须走这里——只要种子相同、调用序列相同，
 * 模拟结果就完全一致。这是"存档可复现"的基础。
 */

/** splitmix32：把单个 32 位种子扩展成 4 个状态字 */
function splitmix32(seed) {
  let a = seed >>> 0;
  return function next() {
    a = (a + 0x9e3779b9) >>> 0;
    let t = a ^ (a >>> 16);
    t = Math.imul(t, 0x21f0aaad);
    t = t ^ (t >>> 15);
    t = Math.imul(t, 0x735a2d97);
    return (t ^ (t >>> 15)) >>> 0;
  };
}

export class RNG {
  constructor(seed = 1) {
    this.seed(seed);
  }

  seed(seed) {
    this._seed = (seed >>> 0) || 1;
    const mix = splitmix32(this._seed);
    this.a = mix();
    this.b = mix();
    this.c = mix();
    this.d = mix();
    if ((this.a | this.b | this.c | this.d) === 0) this.a = 1;
    this._spare = null; // 高斯分布缓存
    return this;
  }

  /** 原始 32 位无符号整数 */
  nextUint32() {
    let t = this.d;
    const s = this.a;
    this.d = this.c;
    this.c = this.b;
    this.b = s;
    t ^= t << 11;
    t ^= t >>> 8;
    this.a = (t ^ s ^ (s >>> 19)) >>> 0;
    return this.a;
  }

  /** [0, 1) 浮点数 */
  float() {
    return this.nextUint32() / 4294967296;
  }

  /** [0, n) 整数 */
  int(n) {
    return (this.float() * n) | 0;
  }

  /** [min, max) 浮点数 */
  range(min, max) {
    return min + this.float() * (max - min);
  }

  /** [min, max] 整数 */
  intRange(min, max) {
    return min + this.int(max - min + 1);
  }

  /** 以 prob% 的概率返回 true */
  chance(probPercent) {
    return this.float() * 100 < probPercent;
  }

  /** 以 probability（0-1）的概率返回 true */
  bernoulli(probability) {
    return this.float() < probability;
  }

  /** 从数组中随机取一个元素 */
  pick(array) {
    return array[(this.float() * array.length) | 0];
  }

  /** 标准正态分布（Box-Muller，带缓存） */
  gaussian(mean = 0, sigma = 1) {
    if (this._spare !== null) {
      const v = this._spare;
      this._spare = null;
      return mean + sigma * v;
    }
    let u = 0;
    let v = 0;
    let s = 0;
    do {
      u = this.float() * 2 - 1;
      v = this.float() * 2 - 1;
      s = u * u + v * v;
    } while (s >= 1 || s === 0);
    const mul = Math.sqrt((-2 * Math.log(s)) / s);
    this._spare = v * mul;
    return mean + sigma * u * mul;
  }

  /** 原地洗牌（Fisher-Yates） */
  shuffle(array) {
    for (let i = array.length - 1; i > 0; i--) {
      const j = this.int(i + 1);
      const tmp = array[i];
      array[i] = array[j];
      array[j] = tmp;
    }
    return array;
  }

  /** 导出当前状态，用于存档 */
  getState() {
    return { seed: this._seed, a: this.a, b: this.b, c: this.c, d: this.d };
  }

  /** 从存档恢复状态 */
  setState(state) {
    this._seed = state.seed;
    this.a = state.a;
    this.b = state.b;
    this.c = state.c;
    this.d = state.d;
    this._spare = null;
  }
}
