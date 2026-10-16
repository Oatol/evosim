import { CellType, DIR_VECTORS, cellAttr, isPassable } from './CellType.js';
import { Anatomy } from './Anatomy.js';
import { Brain } from './Brain.js';
import { Sensors } from './Sensors.js';
import { rotateCol, rotateRow, performCellFunction } from './BodyCell.js';
import { applyTerrainEnter, applyTerrainStand } from './Terrain.js';
import { mutateOrganism } from './Mutation.js';

/**
 * 生物个体：生命周期。
 *
 * 主循环把它拆成两半（设计文档 §5.1）：
 *   think()  阶段 A —— 只读世界，决定"本 tick 想做什么"
 *   act()    阶段 B —— 写世界，移动、进食、受伤、繁殖、死亡
 *
 * 阶段 2 的 think() 只是掷骰子（随机游走）；
 * 阶段 4 会把它换成神经网络，其余部分完全不动。
 */
export class Organism {
  constructor(world, col, row) {
    this.world = world;
    this.id = 0;                 // 由 world.addOrganism 分配

    this.col = col;
    this.row = row;
    this.normalizePosition();

    this.rotation = 0;           // 身体朝向（枢轴旋转）
    this.direction = 0;          // 移动方向

    this.age = 0;
    this.energy = 0;
    this.damage = 0;
    this.starveTicks = 0;        // 连续能量见底的 tick 数（代谢消耗引入的饿死计时）
    this.inRadiation = false;    // 这一 tick 是否站在辐射带里（Mutation 用它放大变异率）

    this.alive = true;
    this.anatomy = new Anatomy();

    // ── 遗传信息 ──
    this.mutability = world.config.mutation.initialMutability;  // 变异率本身也遗传、也变异
    this.speciesFingerprint = null;   // 结构指纹；world.addOrganism 会兜底填充
    this.parentFingerprint = null;    // 母体所属物种的指纹（物种树用）
    this.brain = new Brain(world.config, world.rng);            // 基因组 B：神经网络权重
    this.sensors = new Sensors(world.config.brain.inSize);      // 输入缓冲，复用不分配

    // 阶段 A 产出的意图，供阶段 B 消费
    this.planTurn = false;
    this.planDirection = 0;
    this.turnCooldown = 0;            // 刚转过身的冷却 tick 数，防止原地打转
  }

  // ────────────────────────────── 派生量 ──────────────────────────────

  get cellCount() {
    return this.anatomy.cells.length;
  }

  /** 繁殖所需的能量（**基础成本**：真正从能量里扣掉的就是这个数） */
  foodNeeded() {
    const extra = this.anatomy.hasMover ? this.world.config.life.extraMoverFoodCost : 0;
    return this.cellCount + extra;
  }

  /**
   * 触发繁殖所需的能量**阈值**。
   *
   * 没有储食细胞时它就等于 `foodNeeded()`（逐位相同，基线不受影响）。
   * 有储食细胞时阈值被抬高，但繁殖时扣掉的仍是**基础成本**——
   * 多出来的那部分就是"储备"，会留在能量里，供之后消耗/饥饿时烧。
   *
   * 这是让「全局消耗量倍率」变成主玩法的钥匙：生态一开消耗就崩，
   * 根因是生物的能量长期贴着 0（一攒够就繁殖掉），没有缓冲。
   */
  reproduceAt() {
    const base = this.foodNeeded();
    if (!this.anatomy.hasStorage) return base;
    const bonus = this.world.config.life.storageFoodBonus;
    if (!(bonus > 0)) return base;
    return base * (1 + this.storageCount() * bonus);
  }

  /** 储食细胞个数 */
  storageCount() {
    const cells = this.anatomy.cells;
    let n = 0;
    for (let i = 0; i < cells.length; i++) {
      if (cells[i].type === CellType.STORAGE) n++;
    }
    return n;
  }

  /**
   * 被攻击时反弹给攻击者的伤害 = 所有刺细胞的 `damage` 之和。
   *
   * 复用细胞的 `damage` 属性（杀手用的也是它），所以参数面板里能实时调。
   * **不吃装甲减免**——装甲挡的是"被咬"，挡不住"被扎"，
   * 这也是刺能对"装甲+杀手"这条路线形成制衡的原因。
   */
  spikeDamage() {
    if (!this.anatomy.hasSpike) return 0;
    const cfg = this.world.config;
    const cells = this.anatomy.cells;
    let d = 0;
    for (let i = 0; i < cells.length; i++) {
      if (cells[i].type === CellType.SPIKE) d += cellAttr(cfg, cells[i].type, 'damage');
    }
    return d;
  }

  lifespan() {
    return this.cellCount * this.world.config.life.lifespanMultiplier;
  }

  /**
   * 生命值上限 = 所有细胞的 `hp` 之和（细胞类型属性，可在参数面板实时调）。
   *
   * 默认每种细胞 hp = 1，所以它等于体型——保持与"装甲/杀手"那套旧逻辑一致；
   * 把某种细胞的 hp 调高，就等于给这种结构加血。
   */
  maxHealth() {
    const cfg = this.world.config;
    const cells = this.anatomy.cells;
    let hp = 0;
    for (let i = 0; i < cells.length; i++) hp += cellAttr(cfg, cells[i].type, 'hp');
    return hp;
  }

  /** 每 tick 代谢消耗 = 所有细胞的 `cost` 之和 × 全局倍率 */
  upkeepCost() {
    const cfg = this.world.config;
    const scale = cfg.life.metabolismScale;
    if (!scale || scale <= 0) return 0;
    const cells = this.anatomy.cells;
    let cost = 0;
    for (let i = 0; i < cells.length; i++) cost += cellAttr(cfg, cells[i].type, 'cost');
    return cost * scale;
  }

  /** 死亡时能转化出的食物总量（= 所有细胞 `foodValue` 之和） */
  foodValueTotal() {
    const cfg = this.world.config;
    const cells = this.anatomy.cells;
    let v = 0;
    for (let i = 0; i < cells.length; i++) v += cellAttr(cfg, cells[i].type, 'foodValue');
    return v;
  }

  /** 能量进度 0-1，供渲染与检视器使用 */
  get energyRatio() {
    const need = this.reproduceAt();
    return need <= 0 ? 0 : Math.min(this.energy / need, 1);
  }

  get ageRatio() {
    const life = this.lifespan();
    return life <= 0 ? 0 : Math.min(this.age / life, 1);
  }

  // ────────────────────────────── 坐标 ──────────────────────────────

  /**
   * 把中心坐标折回网格范围内。
   *
   * 环绕世界里坐标不做归一化的话，一路向左走的生物 col 会一直减小，
   * 虽然 grid.index 会正确折回、画面没问题，但坐标会无限漂移，
   * 让检视器和相机跟随拿到离谱的数值。
   *
   * 注意：只在"落位"之后调用。繁殖时子代位置必须先按未归一化的
   * 相对偏移算出来，否则 isStraightPath 会绕远路把繁殖判成受阻。
   */
  normalizePosition() {
    const grid = this.world.grid;
    if (!grid.wrap) return;
    const cols = grid.cols;
    const rows = grid.rows;
    this.col = ((this.col % cols) + cols) % cols;
    this.row = ((this.row % rows) + rows) % rows;
  }

  realCol(cell) {
    return this.col + rotateCol(cell.dx, cell.dy, this.rotation);
  }

  realRow(cell) {
    return this.row + rotateRow(cell.dx, cell.dy, this.rotation);
  }

  /**
   * 给定世界坐标，返回该处眼睛的绝对朝向；不是眼睛则返回 -1。
   * 渲染层用它画朝向细缝。
   */
  eyeDirectionAt(col, row) {
    const grid = this.world.grid;
    const target = grid.index(col, row);
    if (target < 0) return -1;
    const cells = this.anatomy.cells;
    for (let i = 0; i < cells.length; i++) {
      const cell = cells[i];
      if (cell.type !== CellType.EYE) continue;
      if (grid.index(this.realCol(cell), this.realRow(cell)) === target) {
        return (this.rotation + cell.dir) & 3;
      }
    }
    return -1;
  }

  // ────────────────────────────── 阶段 A：决策 ──────────────────────────────

  /**
   * 阶段 A：感知 → 决策。只读世界，不写任何网格。
   *
   * 网络输出 5 维的语义（设计文档 §6.2）：
   *   o[0..3]  四个**相对**方向的移动倾向，argmax 决定往哪走
   *   o[4]     转身倾向，若为 5 个输出里最大的则本 tick 原地旋转身体
   *
   * 用相对方向而不是绝对方向：身体转过去，"前"就跟着转过去，
   * 生物不必为四个绝对方向各学一套策略——转向和移动因此解耦。
   */
  think() {
    this.planTurn = false;

    // 感知 → 前向传播。**所有生物都算**，不只是会动的：
    // 这样检视器对任何个体都能显示激活值，而且大脑是一份"预备能力"——
    // 一旦后代变异出移动器，这套权重立刻就能用上。
    const inputs = this.sensors.sense(this);
    const out = this.brain.forward(inputs);

    if (!this.anatomy.hasMover) return;

    const OUT = this.brain.outSize;
    let best = 0;
    for (let i = 1; i < OUT; i++) {
      if (out[i] > out[best]) best = i;
    }

    if (best === OUT - 1 && this.turnCooldown <= 0) {
      this.planTurn = true;
      return;
    }

    // o[4] 最强但还在冷却里 → 退化成"按移动倾向里最强的那个走"
    let dir = best;
    if (dir === OUT - 1) {
      dir = 0;
      for (let i = 1; i < OUT - 1; i++) if (out[i] > out[dir]) dir = i;
    }
    this.planDirection = (this.rotation + dir) & 3;
  }

  // ────────────────────────────── 阶段 B：结算 ──────────────────────────────

  act() {
    this.age++;
    if (this.age > this.lifespan()) {
      this.die();
      return;
    }

    // 每 tick 重置"这一 tick 是否处于辐射中"——辐射的变异率加成只在
    // 当 tick 繁殖时生效，不该跨 tick 累积（否则会指数爆炸）
    this.inRadiation = false;

    // 代谢：所有细胞的消耗之和。饿死不是立刻的——能量见底后还能撑
    // `starvationTicks` 个 tick，给新生个体一点找食物的时间。
    const upkeep = this.upkeepCost();
    if (upkeep > 0) {
      this.energy -= upkeep;
      if (this.energy <= 0) {
        this.energy = 0;
        this.starveTicks++;
        if (this.starveTicks >= this.world.config.life.starvationTicks) {
          this.die();
          return;
        }
      } else {
        this.starveTicks = 0;
      }
    }

    if (this.anatomy.hasMover) this._actMove();
    if (!this.alive) return;          // 可能踏进岩浆被烫死

    // 驻留效果：站在辐射带/岩浆上每 tick 触发一次。
    // 放在移动**之后**，所以"这一 tick 走到哪儿"决定了受什么影响。
    this._applyTerrainStand();
    if (!this.alive) return;

    // 细胞功能：进食 / 造食 / 攻击。杀手可能中途把自己害死（一击必杀模式）
    const cells = this.anatomy.cells;
    for (let i = 0; i < cells.length; i++) {
      performCellFunction(this, cells[i]);
      if (!this.alive) return;
    }

    if (this.energy >= this.reproduceAt()) this.reproduce();
  }

  /**
   * 驻留在当前格的地形上，每 tick 一次。
   *
   * 读的是 **floor 层**——生物自己的细胞就压在枢轴格上，顶层 `types` 是它自己，
   * 只有地板层才知道它脚底下踩的是什么。
   */
  _applyTerrainStand() {
    const grid = this.world.grid;
    const index = grid.index(this.col, this.row);
    if (index < 0) return;
    const type = grid.floor[index];
    if (type === CellType.EMPTY) return;
    applyTerrainStand(this, type, this.col, this.row);
  }

  _actMove() {
    if (this.turnCooldown > 0) this.turnCooldown--;

    if (this.planTurn) {
      // 网络要求转身：换个身体朝向（会一并重新随机移动方向）
      if (this.attemptRotate()) {
        this.turnCooldown = this.world.config.life.turnCooldown;   // 防止原地打转
        return;
      }
      this.changeDirection(this.planDirection);
      return;
    }

    this.direction = this.planDirection;
    if (this.attemptMove()) return;

    // 前方被挡住：试着换个朝向再走
    if (this.attemptRotate()) {
      this.turnCooldown = this.world.config.life.turnCooldown;
    } else {
      this.changeDirection(this.planDirection);
    }
  }

  // ────────────────────────────── 移动与旋转 ──────────────────────────────

  attemptMove() {
    const v = DIR_VECTORS[this.direction];
    const newCol = this.col + v[0];
    const newRow = this.row + v[1];
    if (!this.isClear(newCol, newRow, this.rotation)) return false;

    // 目的地的**地形**要在 writeGrid 之前读——writeGrid 会用自己的细胞盖住顶层。
    // 读的是 floor 层而不是 types：万一那格上压着食物，地形仍然算数
    // （生物走过去会把食物顶掉，所以该烫还是得烫）。
    const destIndex = this.world.grid.index(newCol, newRow);
    const destType = destIndex < 0 ? CellType.EMPTY : this.world.grid.floor[destIndex];

    this.clearGrid();
    this.col = newCol;
    this.row = newRow;
    this.writeGrid();
    this.normalizePosition();

    // 踏入效果（岩浆烫、沼泽拖）。可能直接把这只生物弄死，调用方要复查 alive。
    if (destType !== CellType.EMPTY) {
      applyTerrainEnter(this, destType, this.col, this.row);
    }
    return true;
  }

  attemptRotate() {
    const newRotation = this.world.rng.int(4);
    if (!this.isClear(this.col, this.row, newRotation)) return false;

    this.clearGrid();
    this.rotation = newRotation;
    this.direction = this.world.rng.int(4);
    this.writeGrid();
    return true;
  }

  changeDirection(dir) {
    this.direction = dir & 3;
  }

  /**
   * 身体能否整体落在 (col, row) 且朝向 rotation。
   * 空地和自己的格子可以；食物看配置；其他生物和墙不行。
   */
  isClear(col, row, rotation = this.rotation) {
    const grid = this.world.grid;
    const blocksFood = this.world.config.combat.foodBlocksReproduction;
    const cells = this.anatomy.cells;

    for (let i = 0; i < cells.length; i++) {
      const cell = cells[i];
      const c = col + rotateCol(cell.dx, cell.dy, rotation);
      const r = row + rotateRow(cell.dx, cell.dy, rotation);
      const index = grid.index(c, r);
      if (index < 0) return false;              // 越界

      const type = grid.types[index];
      if (type === CellType.EMPTY) continue;
      if (grid.owners[index] === this.id) continue;
      if (type === CellType.FOOD && !blocksFood) continue;
      if (isPassable(type)) continue;           // 岩浆/沼泽/辐射带/腐殖质等可以走进去
      return false;
    }
    return true;
  }

  /**
   * 繁殖专用：给定蓝图能否落在 (col, row)。
   *
   * 和 isClear 的区别是**不允许与任何生物重叠，包括母体自己**。
   * 移动时"允许自己的格子"是必要的（身体本来就压在自己身上），
   * 但子代和母体压在一起会让 owners 表错乱——两个生物写同一格，
   * 之后谁 clearGrid 都会把对方的身体抹掉。
   *
   * 为什么需要单独一遍：变异可能给子代加了细胞，母体蓝图算出来的
   * 出生距离就不再保证安全了。
   */
  canPlaceAt(col, row, rotation, anatomy = this.anatomy) {
    const grid = this.world.grid;
    const blocksFood = this.world.config.combat.foodBlocksReproduction;
    const cells = anatomy.cells;

    for (let i = 0; i < cells.length; i++) {
      const cell = cells[i];
      const index = grid.index(
        col + rotateCol(cell.dx, cell.dy, rotation),
        row + rotateRow(cell.dx, cell.dy, rotation),
      );
      if (index < 0) return false;

      const type = grid.types[index];
      if (type === CellType.EMPTY) continue;
      if (type === CellType.FOOD && !blocksFood) continue;
      if (isPassable(type)) continue;           // 地形（岩浆/沼泽/…）可以落在上面
      return false;
    }
    return true;
  }

  /** 母体到出生点的直线路径是否畅通（假定 c1==c2 或 r1==r2） */  isStraightPath(c1, r1, c2, r2, parentId) {
    const grid = this.world.grid;

    if (c1 === c2) {
      const from = Math.min(r1, r2);
      const to = Math.max(r1, r2);
      for (let r = from; r <= to; r++) {
        if (!this._passable(grid.index(c1, r), parentId)) return false;
      }
    } else {
      const from = Math.min(c1, c2);
      const to = Math.max(c1, c2);
      for (let c = from; c <= to; c++) {
        if (!this._passable(grid.index(c, r1), parentId)) return false;
      }
    }
    return true;
  }

  _passable(index, parentId) {
    if (index < 0) return false;
    const grid = this.world.grid;
    const type = grid.types[index];
    if (type === CellType.EMPTY || type === CellType.FOOD) return true;
    if (isPassable(type)) return true;          // 地形不挡住"出生路径"
    const ownerId = grid.owners[index];
    return ownerId === this.id || ownerId === parentId;
  }

  // ────────────────────────────── 网格写入 ──────────────────────────────

  writeGrid() {
    const world = this.world;
    const grid = world.grid;
    const cells = this.anatomy.cells;
    for (let i = 0; i < cells.length; i++) {
      const cell = cells[i];
      const index = grid.index(this.realCol(cell), this.realRow(cell));
      if (index >= 0) world.setCellAt(index, cell.type, this.id);
    }
  }

  clearGrid() {
    const world = this.world;
    const grid = world.grid;
    const cells = this.anatomy.cells;
    for (let i = 0; i < cells.length; i++) {
      const cell = cells[i];
      const index = grid.index(this.realCol(cell), this.realRow(cell));
      // 用 revealFloor 而不是写空：地形（岩浆/腐殖质…）只是被身体暂时盖住，
      // 生物离开后要露出来。写空会把地形一起删掉。
      if (index >= 0) world.revealFloor(index);
    }
  }

  // ────────────────────────────── 繁殖 ──────────────────────────────

  reproduce() {
    const world = this.world;
    const rng = world.rng;
    // 扣的是**基础成本**而不是触发阈值：储食细胞抬高的那部分留在能量里当储备
    const need = this.foodNeeded();

    // 无论成败都要付出代价——惩罚"盲目繁殖"
    this.energy -= need;
    if (this.energy < 0) this.energy = 0;

    if (!world.canAddOrganism()) return null;

    const v = DIR_VECTORS[rng.int(4)];
    const distance = this.anatomy.birthDistance + rng.int(3);
    const newCol = this.col + v[0] * distance;
    const newRow = this.row + v[1] * distance;
    const rotation = rng.int(4);

    // 第一道预检用母体蓝图——便宜，能筛掉拥挤环境里绝大多数失败的繁殖，
    // 省掉大量无谓的对象分配与 GC 压力。
    // （变异之前子代蓝图就是母体蓝图，所以这一步等价；
    //   变异之后还要用 canPlaceAt 按变异后的蓝图严格复查。）
    if (!this.isClear(newCol, newRow, rotation)) return null;
    if (!this.isStraightPath(newCol, newRow, this.col, this.row, this.id)) return null;

    const child = new Organism(world, newCol, newRow);
    child.anatomy.copyFrom(this.anatomy);
    child.brain.copyFrom(this.brain);        // 大脑也遗传——权重变异在 mutateOrganism 里做
    child.rotation = rotation;
    child.direction = rng.int(4);

    // 变异：元变异 + 结构变异 + 权重变异（设计文档 §6.5）
    mutateOrganism(child, this, rng, world.config);

    // 变异可能给子代加了细胞，母体 birthDistance 算出的出生距离未必还够，
    // 所以用变异后的蓝图再严格检查一次（不允许与任何生物重叠，含母体）。
    if (!child.canPlaceAt(newCol, newRow, rotation, child.anatomy)) return null;

    return world.addOrganism(child);
  }

  // ────────────────────────────── 伤害与死亡 ──────────────────────────────

  /**
   * 受伤。
   *
   * `attacker` 只在**生物之间的近战**里传（目前只有杀手细胞）。
   * 传了它就会触发刺细胞的反伤；地形伤害（岩浆/辐射）不传，
   * 所以站在岩浆里不会去扎岩浆。
   *
   * 反伤时**不再往下传 attacker**——否则两只都带刺的生物会互相无限反伤。
   * 这一层截断让"甲刺乙、乙刺甲"最多各挨一下。
   */
  harm(amount = 1, attacker = null) {
    if (!this.alive) return;
    this.damage += amount;

    if (attacker && attacker !== this && attacker.alive) {
      const back = this.spikeDamage();
      if (back > 0) attacker.harm(back);        // 不带 attacker → 不再连锁反伤
    }

    if (this.world.config.combat.instaKill || this.damage >= this.maxHealth()) {
      this.die();
    }
  }

  /**
   * 死亡：身体占据的每一格变成食物——能量回流生态系统。
   *
   * 食物总量由细胞类型的 `foodValue` 决定：每格先变 1 份（尸体本身），
   * `foodValue` 之和超出格数的部分再投放到尸体周围的空格里。
   * 默认每种细胞 foodValue = 1，于是行为与"一格一食物"完全一致；
   * 把某种细胞的 foodValue 调高，尸体就更肥。
   */
  die() {
    if (!this.alive) return;
    this.alive = false;

    const world = this.world;
    const grid = world.grid;
    const cells = this.anatomy.cells;
    for (let i = 0; i < cells.length; i++) {
      const cell = cells[i];
      const index = grid.index(this.realCol(cell), this.realRow(cell));
      if (index >= 0) world.setCellAt(index, CellType.FOOD, -1);
    }

    const extra = Math.round(this.foodValueTotal()) - cells.length;
    if (extra > 0) dropExtraFood(world, this.col, this.row, extra);
  }
}

/**
 * 在 (col, row) 周围的空格里投放 `count` 份额外食物（尸体"超出血量"的部分）。
 * 由内向外一圈圈找空格，放满为止。
 */
function dropExtraFood(world, col, row, count) {
  const grid = world.grid;
  const maxR = Math.max(grid.cols, grid.rows);
  let placed = 0;
  for (let r = 1; r <= maxR && placed < count; r++) {
    for (let dy = -r; dy <= r && placed < count; dy++) {
      for (let dx = -r; dx <= r && placed < count; dx++) {
        // 只看这一圈的边框，圈内已经在更小的半径里处理过了
        if (Math.max(Math.abs(dx), Math.abs(dy)) !== r) continue;
        const index = grid.index(col + dx, row + dy);
        if (index < 0) continue;
        if (grid.types[index] !== CellType.EMPTY) continue;
        world.setCellAt(index, CellType.FOOD, -1);
        placed++;
      }
    }
  }
}
