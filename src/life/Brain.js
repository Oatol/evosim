/**
 * 神经网络 —— 基因组 B。
 *
 * 固定拓扑前馈网络：输入 40 → 隐藏 12 → 输出 5。
 * **只变异权重，不改拓扑**——这是刻意的取舍（设计文档 §14）：
 * 可变拓扑（NEAT 式）会同时引入结构变异和权重变异两套搜索空间，
 * 在可玩沙盒的时间尺度上很难收敛出可解释的行为。
 *
 * 权重总数 557 个浮点数，这就是全部可遗传的"智力信息"。
 * 用 Float32Array 而非普通数组：内存紧凑、变异遍历快、便于整体序列化。
 *
 * 注意：**权重变异不改变物种指纹**。身体形态才是"物种"的判据，
 * 大脑是个体差异——这样物种树不会被权重漂移搅乱（§7.1）。
 */

export class Brain {
  constructor(config, rng) {
    const brain = config.brain;
    this.inSize = brain.inSize;
    this.hiddenSize = brain.hiddenSize;
    this.outSize = brain.outSize;

    this.w1 = new Float32Array(this.inSize * this.hiddenSize);   // 40 × 12 = 480
    this.b1 = new Float32Array(this.hiddenSize);                 // 12
    this.w2 = new Float32Array(this.hiddenSize * this.outSize);  // 12 × 5  = 60
    this.b2 = new Float32Array(this.outSize);                    // 5

    // 复用缓冲——每 tick 每个生物都要前向传播，绝不能每次分配
    this._hidden = new Float32Array(this.hiddenSize);
    this._output = new Float32Array(this.outSize);

    /** 最后一次前向传播的输入（引用 Sensors 的缓冲），检视器读它显示激活值 */
    this.lastInput = null;

    if (rng) this.randomize(rng);
  }

  /** 可遗传的权重总数 */
  get weightCount() {
    return this.w1.length + this.b1.length + this.w2.length + this.b2.length;
  }

  /**
   * 随机初始化。
   *
   * w1 / w2 用 He 初始化的量级（σ = √(1/扇入)），这样隐藏层不会一上来就 tanh 饱和。
   * b2 特意给了 σ = 0.5 的随机值：没有眼睛的生物视觉输入全 0，
   * 此时输出完全由 b2 决定——b2 若都是 0，所有个体都会朝同一个方向走，
   * 种群会整整齐齐地平移，看不出"个体差异"。
   */
  randomize(rng) {
    const s1 = Math.sqrt(1 / this.inSize);
    const s2 = Math.sqrt(1 / this.hiddenSize);

    for (let i = 0; i < this.w1.length; i++) this.w1[i] = rng.gaussian(0, s1);
    for (let i = 0; i < this.b1.length; i++) this.b1[i] = 0;
    for (let i = 0; i < this.w2.length; i++) this.w2[i] = rng.gaussian(0, s2);
    for (let i = 0; i < this.b2.length; i++) this.b2[i] = rng.gaussian(0, 0.5);
    return this;
  }

  /**
   * 前向传播。
   *
   *   h[j] = tanh( Σ_i in[i]·w1[j][i] + b1[j] )     // 12 维
   *   o[k] =        Σ_j h[j]·w2[j][k] + b2[k]       // 5 维，线性输出
   *
   * 输出层不做激活：我们要的是"哪个方向最强"（argmax），
   * 加个 tanh 只会压缩差异、让 argmax 更难分辨。
   *
   * @param {Float32Array} inputs 长度 = inSize
   * @returns {Float32Array} 复用的输出缓冲（5 维），调用方不要长期持有
   */
  forward(inputs) {
    const IN = this.inSize;
    const HID = this.hiddenSize;
    const OUT = this.outSize;
    const w1 = this.w1;
    const b1 = this.b1;
    const w2 = this.w2;
    const b2 = this.b2;
    const h = this._hidden;
    const out = this._output;

    // 防御：运行中改过隐藏层尺寸的话，新旧大脑的 inSize 可能不一致
    const n = inputs.length < IN ? inputs.length : IN;

    for (let j = 0; j < HID; j++) {
      let sum = b1[j];
      const base = j * IN;                 // 按隐藏神经元分组 → 连续内存访问
      for (let i = 0; i < n; i++) sum += inputs[i] * w1[base + i];
      h[j] = Math.tanh(sum);
    }

    for (let k = 0; k < OUT; k++) {
      let sum = b2[k];
      for (let j = 0; j < HID; j++) sum += h[j] * w2[j * OUT + k];
      out[k] = sum;
    }

    this.lastInput = inputs;
    return out;
  }

  /**
   * 深拷贝权重（繁殖时用）。
   * 拓扑不一致时（运行中改过隐藏层尺寸）返回 false，保留自己随机初始化的权重——
   * 宁可让子代是个"新大脑"，也不要按错误的步长复制出乱码。
   */
  copyFrom(other) {
    if (other.inSize !== this.inSize
      || other.hiddenSize !== this.hiddenSize
      || other.outSize !== this.outSize) {
      return false;
    }
    this.w1.set(other.w1);
    this.b1.set(other.b1);
    this.w2.set(other.w2);
    this.b2.set(other.b2);
    return true;
  }

  /** 隐藏层激活值（复用缓冲） */
  get hiddenActivations() {
    return this._hidden;
  }

  /** 输出层值（复用缓冲） */
  get outputActivations() {
    return this._output;
  }

  /** 权重绝对值最大的那个，供检视器定位"最强连接" */
  strongestWeight() {
    const arrays = [this.w1, this.w2];
    let best = 0;
    for (const arr of arrays) {
      for (let i = 0; i < arr.length; i++) {
        const a = arr[i] < 0 ? -arr[i] : arr[i];
        if (a > best) best = a;
      }
    }
    return best;
  }
}
