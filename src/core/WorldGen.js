import { CellType } from '../life/CellType.js';

/**
 * 世界生成器。
 *
 * 阶段 1 用来造出"有东西可看"的初始世界：墙组成的洞穴结构 + 成片的食物。
 * 所有随机都走传入的 RNG，保证同一 seed 生成同一世界。
 */

/** 画一个圆斑（支持边界环绕） */
function paintBlob(world, cx, cy, radius, type, jitter = 0) {
  const r2 = radius * radius;
  const c0 = Math.floor(cx - radius - 1);
  const c1 = Math.ceil(cx + radius + 1);
  const r0 = Math.floor(cy - radius - 1);
  const r1 = Math.ceil(cy + radius + 1);

  for (let row = r0; row <= r1; row++) {
    for (let col = c0; col <= c1; col++) {
      const dx = col - cx;
      const dy = row - cy;
      const d2 = dx * dx + dy * dy;
      if (d2 > r2) continue;
      // 边缘做一点抖动，让轮廓不那么规整
      if (jitter > 0 && d2 > r2 * (1 - jitter)) continue;
      const index = world.grid.index(col, row);
      if (index < 0) continue;
      world.setCellAt(index, type, -1);
    }
  }
}

/**
 * 生成一个示例世界。
 * @param {World} world
 * @param {object} options
 * @param {number} options.wallDensity  墙的密度系数，越大墙越多
 * @param {number} options.foodDensity  食物的密度系数
 */
export function generateDemoWorld(world, options = {}) {
  const { wallDensity = 1, foodDensity = 1 } = options;
  const rng = world.rng;
  const cols = world.grid.cols;
  const rows = world.grid.rows;
  const area = cols * rows;

  world.reset(false);
  const grid = world.grid;

  // ── 1. 洞穴状墙体：成簇生长 ──
  const wallSeeds = Math.max(6, Math.round((area / 2600) * wallDensity));
  for (let i = 0; i < wallSeeds; i++) {
    let cx = rng.range(0, cols);
    let cy = rng.range(0, rows);
    const steps = rng.intRange(10, 34);
    let radius = rng.range(1.6, 3.4);
    for (let s = 0; s < steps; s++) {
      paintBlob(world, cx, cy, radius, CellType.WALL, 0.45);
      cx += rng.gaussian(0, 2.1);
      cy += rng.gaussian(0, 2.1);
      radius = Math.max(1.1, radius + rng.gaussian(0, 0.25));
    }
  }

  // ── 2. 食物草甸：几片成规模的区域 ──
  const meadowCount = Math.max(4, Math.round((area / 4200) * foodDensity));
  for (let i = 0; i < meadowCount; i++) {
    const cx = rng.range(0, cols);
    const cy = rng.range(0, rows);
    const radius = rng.range(6, 15);
    const r2 = radius * radius;
    for (let row = Math.floor(cy - radius); row <= Math.ceil(cy + radius); row++) {
      for (let col = Math.floor(cx - radius); col <= Math.ceil(cx + radius); col++) {
        const dx = col - cx;
        const dy = row - cy;
        if (dx * dx + dy * dy > r2) continue;
        const index = grid.index(col, row);
        if (index < 0 || grid.types[index] !== CellType.EMPTY) continue;
        if (rng.float() < 0.34) world.setCellAt(index, CellType.FOOD, -1);
      }
    }
  }

  // ── 3. 零散食物 ──
  const scatter = Math.round((area / 220) * foodDensity);
  for (let i = 0; i < scatter; i++) {
    const col = rng.int(cols);
    const row = rng.int(rows);
    const index = grid.index(col, row);
    if (grid.types[index] === CellType.EMPTY) world.setCellAt(index, CellType.FOOD, -1);
  }

  world.requestFullRedraw();
  return world;
}

/** 只铺一层稀薄食物，用于测试画笔 */
export function generateEmptyWorld(world) {
  world.reset(false);
  world.requestFullRedraw();
  return world;
}
