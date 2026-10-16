import { GridMap } from './GridMap.js';
import {
  CellType, CELL_TYPE_COUNT, LIVING_CELL_TYPES,
  TERRAIN_MASK, TICKS_TERRAIN, TICKING_TERRAIN_TYPES, cellDef,
} from '../life/CellType.js';
import { TERRAIN_TICK } from '../life/Terrain.js';
import { SpeciesRegistry } from '../stats/Species.js';

/**
 * 世界环境：网格 + 生物集合 + 环境规则。
 *
 * 主循环的两阶段更新（设计文档 §5.1）：
 *   阶段 A  think()  全体感知与决策 —— 只读世界，结果与遍历顺序无关
 *   阶段 B  act()    随机顺序结算   —— 写世界
 *
 * 这么拆是为了消除参考项目里"先被遍历到的生物先吃食物、先占位置"的顺序偏差。
 */
export class World {
  constructor(config, rng) {
    this.config = config;
    this.rng = rng;

    this.grid = new GridMap(config.world.cols, config.world.rows, config.world.wrapEdges);

    this.tick = 0;
    this.organisms = [];

    this.nextOrganismId = 1;
    this._organismsById = new Map();
    this._order = [];              // 阶段 B 的随机结算顺序，复用数组避免每 tick 分配

    // 食物索引：foodCells 存所有食物格的扁平索引，foodPos 存反向位置。
    // 删除用 swap-remove，所以增删都是 O(1)。
    // 有了它，食物腐败可以只抽样"存在的食物"而不是扫描整张网格，
    // 统计食物数量也变成 O(1)。
    this.foodCells = [];
    this.foodPos = new Int32Array(this.grid.length).fill(-1);
    this._initFoodIndex();

    // 需要"每 tick 更新"的地形（腐殖质…）的索引，和食物索引同一套 swap-remove。
    // 只有带 `tick` 行为的地形才建索引——其它地形一格代价都不付。
    // 索引跟着 **floor 层** 走，所以被生物踩在脚下的腐殖质照样会造食。
    this.terrainCells = new Map();   // typeId → number[]
    this.terrainPos = new Map();     // typeId → Int32Array（index → 在数组里的位置）
    this._initTerrainIndex();

    // 物种注册表：由 world.addOrganism 驱动，繁殖时顺带登记祖先关系
    this.species = new SpeciesRegistry(config);

    this._listeners = new Map();
  }

  // ────────────────────────────── 食物索引 ──────────────────────────────

  _initFoodIndex() {
    const types = this.grid.types;
    const n = types.length;
    if (this.foodPos.length !== n) this.foodPos = new Int32Array(n);
    this.foodPos.fill(-1);
    this.foodCells.length = 0;
    for (let i = 0; i < n; i++) {
      if (types[i] === CellType.FOOD) {
        this.foodPos[i] = this.foodCells.length;
        this.foodCells.push(i);
      }
    }
  }

  _addFood(index) {
    if (this.foodPos[index] >= 0) return;
    this.foodPos[index] = this.foodCells.length;
    this.foodCells.push(index);
  }

  _removeFood(index) {
    const p = this.foodPos[index];
    if (p < 0) return;
    const last = this.foodCells.pop();
    if (p < this.foodCells.length) {
      this.foodCells[p] = last;
      this.foodPos[last] = p;
    }
    this.foodPos[index] = -1;
  }

  // ────────────────────────────── 地形索引 ──────────────────────────────
  //
  // 只有"带 tick 行为"的地形才有索引（目前是腐殖质）。
  // 索引建在 floor 层上，所以地形被生物盖住时索引不变、照样会 tick。

  _initTerrainIndex() {
    const n = this.grid.length;
    this.terrainCells.clear();
    this.terrainPos.clear();
    for (const t of TICKING_TERRAIN_TYPES) {
      this.terrainCells.set(t, []);
      this.terrainPos.set(t, new Int32Array(n).fill(-1));
    }
    const floor = this.grid.floor;
    for (let i = 0; i < n; i++) {
      if (TICKS_TERRAIN[floor[i]]) this._addTerrainCell(i, floor[i]);
    }
  }

  _addTerrainCell(index, type) {
    const cells = this.terrainCells.get(type);
    if (!cells) return;
    const pos = this.terrainPos.get(type);
    if (pos[index] >= 0) return;
    pos[index] = cells.length;
    cells.push(index);
  }

  _removeTerrainCell(index, type) {
    const cells = this.terrainCells.get(type);
    if (!cells) return;
    const pos = this.terrainPos.get(type);
    const p = pos[index];
    if (p < 0) return;
    const last = cells.pop();
    if (p < cells.length) {
      cells[p] = last;
      pos[last] = p;
    }
    pos[index] = -1;
  }

  /** 地板类型变了 → 把索引从旧类型挪到新类型 */
  _retagTerrain(index, oldType, newType) {
    if (TICKS_TERRAIN[oldType]) this._removeTerrainCell(index, oldType);
    if (TICKS_TERRAIN[newType]) this._addTerrainCell(index, newType);
  }

  // ────────────────────────────── 事件 ──────────────────────────────

  /** 订阅事件：'cellChanged' | 'fullRedraw' | 'reset' | 'stepped' | 'organismAdded' */
  on(event, handler) {
    if (!this._listeners.has(event)) this._listeners.set(event, []);
    this._listeners.get(event).push(handler);
    return this;
  }

  off(event, handler) {
    const list = this._listeners.get(event);
    if (!list) return this;
    const i = list.indexOf(handler);
    if (i >= 0) list.splice(i, 1);
    return this;
  }

  emit(event, ...args) {
    const list = this._listeners.get(event);
    if (!list) return;
    for (let i = 0; i < list.length; i++) list[i](...args);
  }

  // ────────────────────────────── 格子读写 ──────────────────────────────

  /**
   * 写一个格子。这是唯一的写入口——所有变化都会通知渲染层。
   * 同值写入会被忽略，所以不会产生无意义的脏格。
   */
  setCell(col, row, type, ownerId = -1) {
    const index = this.grid.index(col, row);
    if (index < 0) return -1;
    this.setCellAt(index, type, ownerId);
    return index;
  }

  /**
   * 按扁平索引写格子。所有写入都收敛到这一个函数，食物索引才不会有偏差。
   *
   * 它同时维护 **地板层**（GridMap.floor）：
   *   · 写的是地形类型（墙/岩浆/…）→ 地板跟着更新
   *   · 写的是"无主的空" → 地板也清掉（这就是玩家画"空"/橡皮擦的语义）
   * 生物写自己的细胞、或写食物，都不动地板。
   * 生物**离开**时要调 `revealFloor()`，不能用这个函数写空——那样会把地形一起删掉。
   */
  setCellAt(index, type, ownerId = -1) {
    if (index < 0 || index >= this.grid.length) return;
    const types = this.grid.types;
    if (types[index] === type && this.grid.owners[index] === ownerId) return;

    const wasFood = types[index] === CellType.FOOD;

    this.grid.set(index, type, ownerId);

    const floor = this.grid.floor;
    if (TERRAIN_MASK[type]) {
      if (floor[index] !== type) {
        const old = floor[index];
        floor[index] = type;
        this._retagTerrain(index, old, type);
      }
    } else if (type === CellType.EMPTY && ownerId < 0 && floor[index] !== CellType.EMPTY) {
      const old = floor[index];
      floor[index] = CellType.EMPTY;
      this._retagTerrain(index, old, CellType.EMPTY);
    }

    if (type === CellType.FOOD) {
      if (!wasFood) this._addFood(index);
    } else if (wasFood) {
      this._removeFood(index);
    }

    this.emit('cellChanged', index);
  }

  /**
   * 把地板重新铺回顶层 —— 生物离开 / 食物被吃掉时用。
   *
   * 和"写空"的区别是这里**保留地形**：一只生物从岩浆上走过，
   * 岩浆只是被暂时盖住，它走了就该露出来。
   */
  revealFloor(index) {
    if (index < 0 || index >= this.grid.length) return;
    this.setCellAt(index, this.grid.floor[index], -1);
  }

  /** 这一格的地形是什么（不考虑压在上面的生物/食物） */
  floorAt(col, row) {
    const i = this.grid.index(col, row);
    return i < 0 ? CellType.EMPTY : this.grid.floor[i];
  }

  /**
   * 整张网格填成同一种类型。
   * 绕过 setCellAt 的批量操作必须走这里，否则食物索引会与网格脱节。
   * `keepTerrain` 为真时保留地板层里的地形（对应参数"重置时保留地形"）。
   */
  fillGrid(type = CellType.EMPTY, keepTerrain = false) {
    this.grid.fill(type, keepTerrain);
    this._initFoodIndex();
    this._initTerrainIndex();
    this.emit('fullRedraw');
  }

  typeAt(col, row) {
    return this.grid.typeAt(col, row);
  }

  requestFullRedraw() {
    this.emit('fullRedraw');
  }

  // ────────────────────────────── 生物集合 ──────────────────────────────

  addOrganism(organism) {
    organism.world = this;
    organism.id = this.nextOrganismId++;

    // 没有指纹的（画笔投放、初始种群）在这里补算，保证每个个体都归属一个物种
    if (!organism.speciesFingerprint) {
      organism.speciesFingerprint = organism.anatomy.fingerprint();
    }

    this._organismsById.set(organism.id, organism);
    this.organisms.push(organism);
    organism.writeGrid();

    this.species.observe(organism, organism.parentFingerprint, this.tick);
    this.emit('organismAdded', organism);
    return organism;
  }

  organismById(id) {
    return this._organismsById.get(id);
  }

  /**
   * 直接收编一个已构造好的个体，**不登记物种**。
   *
   * 只给读档用：读档时物种表是整体恢复的，再走一遍 observe() 会把
   * cumulativePop 重复累加。其余场合一律用 addOrganism()。
   */
  adoptOrganism(organism) {
    organism.world = this;
    if (!organism.speciesFingerprint) {
      organism.speciesFingerprint = organism.anatomy.fingerprint();
    }
    organism.id = this.nextOrganismId++;
    this._organismsById.set(organism.id, organism);
    this.organisms.push(organism);
    organism.writeGrid();
    return organism;
  }

  canAddOrganism() {
    const max = this.config.life.maxOrganisms;
    return max < 0 || this.organisms.length < max;
  }

  /** 把所有生物变成食物并清空 */
  clearOrganisms() {
    const list = this.organisms;
    for (let i = 0; i < list.length; i++) list[i].die();
    list.length = 0;
    this._organismsById.clear();
  }

  /** 原地压缩掉已死亡的生物（不分配新数组） */
  _compactOrganisms() {
    const list = this.organisms;
    let w = 0;
    for (let i = 0; i < list.length; i++) {
      const org = list[i];
      if (org.alive) {
        list[w++] = org;
      } else {
        this._organismsById.delete(org.id);
      }
    }
    list.length = w;
  }

  // ────────────────────────────── 环境规则 ──────────────────────────────

  /**
   * 食物腐败：每 tick 按比例移除一批存量食物。
   *
   * 为什么必须有它：生产者细胞会把食物造在嘴够不着的地方——
   * 比如生产者长在 (1,1)，它造出的食物会落在 (2,1) 或 (1,2)，
   * 而嘴在 (0,0) 只能吃到距离 1 的格子。这部分食物永远吃不到，
   * 会无限堆积，最终把繁殖空间堵死，形成正反馈把种群推向灭绝。
   *
   * 给食物一个半衰期，等于给"生产者必须长在嘴够得着的地方"施加了
   * 真实的选择压力——这也是后续变异演化要解决的问题。
   */
  applyFoodDecay() {
    const rate = this.config.world.foodDecayRate;
    if (rate <= 0) return;

    const cells = this.foodCells;
    const expected = (cells.length * rate) / 100;
    if (expected <= 0) return;

    // 概率性取整，避免存量少时被直接抹掉
    let n = Math.floor(expected);
    if (this.rng.float() < expected - n) n++;

    for (let i = 0; i < n && cells.length > 0; i++) {
      const index = cells[this.rng.int(cells.length)];
      this.revealFloor(index);          // 食物烂掉 → 露出底下的地形（如果本来有）
    }
  }

  /**
   * 地形自身的每 tick 行为（腐殖质造食）。
   *
   * 只遍历"带 tick 行为"的地形格，不扫全网格——代价与地形格数成正比。
   * 注意：这里的处理器**不应该改地形**（会打乱正在遍历的 swap-remove 索引）；
   * 目前的处理器（造食）只写食物，安全。
   */
  applyTerrainTick() {
    for (const type of TICKING_TERRAIN_TYPES) {
      const cells = this.terrainCells.get(type);
      if (!cells || cells.length === 0) continue;
      const d = cellDef(type);
      const fn = d && d.tick ? TERRAIN_TICK[d.tick] : null;
      if (!fn) continue;
      for (let i = 0; i < cells.length; i++) fn(this, cells[i]);
    }
  }

  applyFoodDrop() {
    const prob = this.config.world.foodDropProb;
    if (prob <= 0) return;

    const area = this.grid.length;
    const count = Math.max(Math.floor((area * prob) / 50000), 1);
    for (let i = 0; i < count; i++) {
      if (!this.rng.chance(prob)) continue;
      const col = this.rng.int(this.grid.cols);
      const row = this.rng.int(this.grid.rows);
      const index = this.grid.index(col, row);
      if (this.grid.types[index] === CellType.EMPTY) {
        this.setCellAt(index, CellType.FOOD, -1);
      }
    }
  }

  // ────────────────────────────── 主循环 ──────────────────────────────

  step() {
    this.tick++;
    const organisms = this.organisms;

    // ── 阶段 A：全体感知与决策（只读世界） ──
    for (let i = 0; i < organisms.length; i++) {
      const org = organisms[i];
      if (org.alive) org.think();
    }

    // ── 阶段 B：随机顺序结算（写世界） ──
    // 先快照——繁殖会往 organisms 里追加，新个体本 tick 不应行动。
    const order = this._order;
    order.length = 0;
    for (let i = 0; i < organisms.length; i++) order.push(organisms[i]);
    this.rng.shuffle(order);

    for (let i = 0; i < order.length; i++) {
      const org = order[i];
      if (org.alive) org.act();
    }

    this._compactOrganisms();

    // ── 环境更新 ──
    this.applyTerrainTick();     // 腐殖质等自己会"长"东西的地形
    this.applyFoodDecay();
    this.applyFoodDrop();

    this.emit('stepped', this.tick);
  }

  // ────────────────────────────── 重置 ──────────────────────────────

  reset(keepTerrain = false) {
    this.clearOrganisms();
    this.grid.wrap = this.config.world.wrapEdges;
    this.grid.fill(CellType.EMPTY, keepTerrain);
    this._initFoodIndex();
    this._initTerrainIndex();
    this.species = new SpeciesRegistry(this.config);
    this.nextOrganismId = 1;
    this.tick = 0;
    this.emit('fullRedraw');
    this.emit('reset');
  }

  resize(cols, rows) {
    this.config.world.cols = cols;
    this.config.world.rows = rows;
    this.organisms.length = 0;
    this._organismsById.clear();
    this.grid.resize(cols, rows, this.config.world.wrapEdges);
    this.foodPos = new Int32Array(this.grid.length);
    this._initFoodIndex();
    this._initTerrainIndex();
    this.species = new SpeciesRegistry(this.config);
    this.nextOrganismId = 1;
    this.tick = 0;
    this.emit('fullRedraw');
    this.emit('reset');
  }

  // ────────────────────────────── 统计 ──────────────────────────────

  snapshotStats() {
    const counts = this.grid.countByType();
    const food = this.foodCells.length;   // 索引维护着，不必再扫一遍

    const body = LIVING_CELL_TYPES.reduce((sum, t) => sum + counts[t], 0);

    return {
      tick: this.tick,
      cols: this.grid.cols,
      rows: this.grid.rows,
      cells: this.grid.length,
      nonEmpty: food + counts[CellType.WALL] + body,
      food,
      wall: counts[CellType.WALL],
      body,
      organisms: this.organisms.length,
    };
  }

  /** 生物的聚合读数，供 UI 展示演化趋势 */
  organismStats() {
    const list = this.organisms;
    if (list.length === 0) {
      return {
        count: 0, avgCells: 0, maxCells: 0, movers: 0, eyes: 0, killers: 0,
        avgAge: 0, avgMutability: 0,
      };
    }
    let cells = 0;
    let maxCells = 0;
    let movers = 0;
    let eyes = 0;
    let killers = 0;
    let age = 0;
    let mutability = 0;

    for (let i = 0; i < list.length; i++) {
      const org = list[i];
      const n = org.cellCount;
      cells += n;
      if (n > maxCells) maxCells = n;
      if (org.anatomy.hasMover) movers++;
      if (org.anatomy.hasEye) eyes++;
      if (org.anatomy.hasKiller) killers++;
      age += org.age;
      mutability += org.mutability;
    }

    return {
      count: list.length,
      avgCells: cells / list.length,
      maxCells,
      movers,
      eyes,
      killers,
      avgAge: age / list.length,
      avgMutability: mutability / list.length,
    };
  }

  /**
   * 更重的统计：在 organismStats 之上再统计各细胞类型的占比。
   * 只在化石记录采样时调用（每 statsInterval tick 一次），
   * 因为它要遍历每个生物的每个细胞。
   */
  detailedStats() {
    const base = this.organismStats();
    const shares = new Float64Array(CELL_TYPE_COUNT);
    const list = this.organisms;

    let total = 0;
    for (let i = 0; i < list.length; i++) {
      const cells = list[i].anatomy.cells;
      for (let j = 0; j < cells.length; j++) {
        shares[cells[j].type]++;
        total++;
      }
    }
    if (total > 0) {
      for (let t = 0; t < shares.length; t++) shares[t] /= total;
    }

    base.cellShares = shares;
    return base;
  }
}
