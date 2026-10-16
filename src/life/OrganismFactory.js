import { CellType, LIVING_CELL_TYPES } from './CellType.js';
import { Organism } from './Organism.js';
import { NEIGHBORS_4 } from './BodyCell.js';

/**
 * 生物工厂：造出可用的起始生物。
 *
 * 阶段 3 起，变异会从这些模板出发自行探索形态空间；
 * 这里只负责给出"能活下去"的起点。
 */

/**
 * 生命起源 —— 参考项目的经典配方：
 * 一个嘴在中心，两个生产者在对角。
 * 不移动，靠生产者造食物、嘴吃掉，稳定地繁衍。
 */
export function createOriginOrganism(world, col, row) {
  const org = new Organism(world, col, row);
  org.anatomy.addCell(CellType.MOUTH, 0, 0);
  org.anatomy.addCell(CellType.PRODUCER, 1, 1);
  org.anatomy.addCell(CellType.PRODUCER, -1, -1);
  return org;
}

/**
 * 随机小生物：从中心向邻域随机生长 2-6 个细胞。
 * 中心格保证是嘴或生产者——否则它一出生就没法获取能量。
 */
export function createRandomOrganism(world, col, row) {
  const rng = world.rng;
  const org = new Organism(world, col, row);
  org.rotation = rng.int(4);
  org.direction = rng.int(4);

  org.anatomy.addCell(rng.chance(50) ? CellType.MOUTH : CellType.PRODUCER, 0, 0);

  const growth = rng.intRange(2, 6);
  for (let i = 0; i < growth; i++) {
    const base = org.anatomy.randomCell(rng);
    const n = NEIGHBORS_4[rng.int(4)];
    const dx = base.dx + n[0];
    const dy = base.dy + n[1];
    if (Math.abs(dx) > 3 || Math.abs(dy) > 3) continue;
    if (org.anatomy.cellAt(dx, dy)) continue;

    const type = rng.pick(LIVING_CELL_TYPES);
    org.anatomy.addCell(type, dx, dy, type === CellType.EYE ? rng.int(4) : 0);
  }

  return org;
}

/** 找一个能放下给定半径生物的空位，找不到返回 null */
export function findClearSpot(world, rng, radius, attempts = 40) {
  const cols = world.grid.cols;
  const rows = world.grid.rows;
  for (let i = 0; i < attempts; i++) {
    const col = rng.int(cols);
    const row = rng.int(rows);
    if (isAreaEmpty(world, col, row, radius)) return [col, row];
  }
  return null;
}

/** 以 (col,row) 为中心的方形区域是否全空 */
export function isAreaEmpty(world, col, row, radius) {
  const grid = world.grid;
  for (let dr = -radius; dr <= radius; dr++) {
    for (let dc = -radius; dc <= radius; dc++) {
      const index = grid.index(col + dc, row + dr);
      if (index < 0) return false;
      if (grid.types[index] !== CellType.EMPTY) return false;
    }
  }
  return true;
}

/** 在世界中央投放生命起源 */
export function seedOriginOfLife(world) {
  const col = (world.grid.cols / 2) | 0;
  const row = (world.grid.rows / 2) | 0;

  // 先把中心一小块清空，保证能放下
  const grid = world.grid;
  for (let dr = -3; dr <= 3; dr++) {
    for (let dc = -3; dc <= 3; dc++) {
      const index = grid.index(col + dc, row + dr);
      if (index >= 0) world.setCellAt(index, CellType.EMPTY, -1);
    }
  }

  const org = createOriginOrganism(world, col, row);
  return world.addOrganism(org);
}

/**
 * 在指定位置投放一个随机生物。
 * @returns {Organism|null} 位置被占时返回 null
 */
export function spawnRandomOrganism(world, col, row) {
  const org = createRandomOrganism(world, col, row);
  if (!org.isClear(col, row, org.rotation)) return null;
  return world.addOrganism(org);
}

/** 撒一片随机生物，返回成功投放的数量 */
export function spawnPopulation(world, count, radius = 3) {
  const rng = world.rng;
  let spawned = 0;
  for (let i = 0; i < count; i++) {
    const spot = findClearSpot(world, rng, radius);
    if (!spot) break;
    if (spawnRandomOrganism(world, spot[0], spot[1])) spawned++;
  }
  return spawned;
}

// ────────────────────────────── 克隆 ──────────────────────────────

/**
 * 从一个存活个体身上取一份"可反复投放"的蓝图。
 *
 * 蓝图存的是**蓝图副本 + 大脑引用**：投放时把大脑权重 copyFrom 过去。
 * 不深拷贝大脑是因为权重有 557 个，而且投放出去的个体之后会自己变异，
 * 引用不会互相污染——copyFrom 是单向的读操作。
 */
export function makeBlueprint(organism) {
  const cells = organism.anatomy.cells.map((c) => ({
    type: c.type, dx: c.dx, dy: c.dy, dir: c.dir,
  }));
  return {
    cells,
    brain: organism.brain,
    label: organism.anatomy.describe(),
    fingerprint: organism.speciesFingerprint,
  };
}

/**
 * 按蓝图投放一个个体（克隆笔刷用）。
 *
 * 投放出来的个体和源个体**同物种**（蓝图一致 → 指纹一致），
 * 但大脑是当时那份权重的拷贝——所以"投放一个已知物种的个体"
 * 在物种树上是加一个人口，而不是开一个新分支。
 *
 * @returns {Organism|null} 位置放不下时返回 null
 */
export function spawnFromBlueprint(world, col, row, blueprint) {
  if (!blueprint || !blueprint.cells.length) return null;

  const rng = world.rng;
  const org = new Organism(world, col, row);
  for (const c of blueprint.cells) {
    org.anatomy.addCell(c.type, c.dx, c.dy, c.dir);
  }
  org.anatomy.recompute();

  if (blueprint.brain) org.brain.copyFrom(blueprint.brain);

  org.rotation = rng.int(4);
  org.direction = rng.int(4);

  if (!org.isClear(col, row, org.rotation)) return null;
  return world.addOrganism(org);
}
