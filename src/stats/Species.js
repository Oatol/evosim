/**
 * 物种与物种注册表 —— 设计文档 §7。
 *
 * 物种的判据是**身体结构指纹**：只有结构变异才产生新物种，
 * 权重变异不改变指纹。这样"物种"对应的是形态，而大脑是个体差异，
 * 物种树不会被权重漂移搅乱。
 */

/**
 * 一个物种。
 *
 * 刻意**不持有 Anatomy 引用**——物种是"历史上出现过的形态"的记录，
 * 不该跟着某个存活个体一起被修改。需要的信息在创建时快照成字符串。
 */
export class Species {
  constructor(id, fingerprint, anatomy, ancestorId, tick) {
    this.id = id;
    this.fingerprint = fingerprint;
    this.name = `S${id}`;
    this.description = anatomy ? anatomy.describe() : '';
    this.cellCount = anatomy ? anatomy.cellCount : 0;

    this.ancestorId = ancestorId;     // 物种 id；null = 起源物种
    this.startTick = tick;
    this.endTick = -1;                // 灭绝时刻，-1 = 尚未灭绝

    this.population = 0;              // 上次采样时的个体数
    this.peakPopulation = 0;
    this.cumulativePop = 0;           // 累计产生过的个体数（过滤"昙花一现"用）
    this.alive = true;
    this.lastSeenTick = tick;
  }

  get lifespan() {
    return (this.endTick >= 0 ? this.endTick : this.lastSeenTick) - this.startTick;
  }
}

/** 物种数上限——超过就剪枝，防止长期运行把内存吃光 */
export const MAX_SPECIES = 4000;

export class SpeciesRegistry {
  constructor(config) {
    this.config = config;
    this.map = new Map();        // fingerprint -> Species
    this.list = [];              // 全部物种（含已灭绝），按创建顺序 —— 也是化石记录的索引
    this.nextId = 1;

    this.aliveCount = 0;
    this.peakAliveCount = 0;
    this.totalCreated = 0;
    this.totalExtinct = 0;
  }

  get size() {
    return this.list.length;
  }

  byFingerprint(fingerprint) {
    return this.map.get(fingerprint) || null;
  }

  /**
   * 登记一个个体。繁殖与投放都会走这里。
   * 指纹没见过就现场建一个物种，祖先指向母体所属的物种。
   */
  observe(organism, parentFingerprint, tick) {
    const fp = organism.speciesFingerprint;
    let sp = this.map.get(fp);

    if (!sp) {
      const parent = parentFingerprint ? this.map.get(parentFingerprint) : null;
      sp = new Species(this.nextId++, fp, organism.anatomy, parent ? parent.id : null, tick);
      this.map.set(fp, sp);
      this.list.push(sp);
      this.totalCreated++;
    }

    sp.cumulativePop++;
    sp.lastSeenTick = tick;
    return sp;
  }

  /**
   * 重新统计每个物种的当前个体数，并标记灭绝。
   * 按 statsInterval 调用，不每 tick 跑——遍历物种列表是 O(物种数)，
   * 而物种数在长期运行后可以到几千。
   */
  update(world, tick) {
    const list = this.list;
    for (let i = 0; i < list.length; i++) list[i].population = 0;

    const organisms = world.organisms;
    const map = this.map;
    for (let i = 0; i < organisms.length; i++) {
      const sp = map.get(organisms[i].speciesFingerprint);
      if (sp) sp.population++;
    }

    let alive = 0;
    for (let i = 0; i < list.length; i++) {
      const sp = list[i];
      if (sp.population > 0) {
        sp.alive = true;
        sp.lastSeenTick = tick;
        if (sp.population > sp.peakPopulation) sp.peakPopulation = sp.population;
        alive++;
      } else if (sp.alive) {
        sp.alive = false;
        sp.endTick = tick;
        this.totalExtinct++;
      }
    }

    this.aliveCount = alive;
    if (alive > this.peakAliveCount) this.peakAliveCount = alive;
    return alive;
  }

  /** 当前存活物种，按个体数降序——物种树与"最常见物种"列表用 */
  topAlive(limit = 20) {
    const out = [];
    const list = this.list;
    for (let i = 0; i < list.length; i++) {
      if (list[i].population > 0) out.push(list[i]);
    }
    out.sort((a, b) => b.population - a.population);
    return out.length > limit ? out.slice(0, limit) : out;
  }

  /**
   * 剪枝：物种数超过上限时丢弃一批。
   *
   * 两轮筛选：
   *   第一轮丢"已灭绝 且 累计个体数 < minDiscard"的——这些多半是刚变异出来
   *   就死掉的畸形，属于噪声，丢了不影响化石记录的可读性。
   *   第二轮还不够就从最老的灭绝物种丢起（兜底，尽量少用）。
   */
  prune(max = MAX_SPECIES) {
    const list = this.list;
    const over = list.length - max;
    if (over <= 0) return 0;

    const minDiscard = this.config.stats.minDiscard;
    let removed = 0;
    let w = 0;

    for (let i = 0; i < list.length; i++) {
      const sp = list[i];
      if (removed < over && !sp.alive && sp.cumulativePop < minDiscard) {
        this.map.delete(sp.fingerprint);
        removed++;
        continue;
      }
      list[w++] = sp;
    }
    list.length = w;

    if (removed < over) {
      w = 0;
      for (let i = 0; i < list.length; i++) {
        const sp = list[i];
        if (removed < over && !sp.alive) {
          this.map.delete(sp.fingerprint);
          removed++;
          continue;
        }
        list[w++] = sp;
      }
      list.length = w;
    }

    return removed;
  }
}
