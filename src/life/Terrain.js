import { CellType, cellDef } from './CellType.js';
import { NEIGHBORS_4 } from './BodyCell.js';

/**
 * 地形效果表 —— "生物踩到地形上会发生什么"。
 *
 * 注册表里的定义对象用三个字段指向这里的函数：
 *
 *   `enter`  踏入瞬间（移动成功的那一下）—— 岩浆烫一下、沼泽拖一下
 *   `stand`  驻留每 tick                    —— 辐射带持续照射
 *   `tick`   世界级每 tick（和生物无关）      —— 腐殖质自己往外冒食物
 *
 * 前两个由 **Organism** 调用（作用对象是"生物"），第三个由 **World** 调用
 * （作用对象是"格子"）。分开是因为腐殖质不需要有生物在场也会造食。
 *
 * 数值全部走 `Config.terrain`，可以在参数面板里实时调——和细胞属性同一套思路：
 * 效果写在这里，数值写在配置里。
 *
 * ── 为什么 enter 和 stand 要分开 ──
 *
 * 只做 enter 的话，"待在辐射带里持续掉血"就没法表达——它不是"进入时发生一次"，
 * 而是"待在里面每 tick 都发生"。反过来只做 stand 的话，"踏进岩浆被烫一下"
 * 的瞬时冲击感也会丢掉。两个都需要，所以拆开。
 */

// ────────────────────────────── 数值读取 ──────────────────────────────

/** 读一个地形数值，缺失/非法时回退到默认值 */
function num(config, key, fallback) {
  const t = config && config.terrain;
  const v = t ? t[key] : undefined;
  return Number.isFinite(v) ? v : fallback;
}

// ────────────────────────────── 踏入 ──────────────────────────────

export const TERRAIN_ENTER = {
  /** 岩浆：踏进去先挨一记冲击伤害 */
  burn(organism) {
    const dmg = num(organism.world.config, 'lavaEnterDamage', 2);
    if (dmg > 0) organism.harm(dmg);
  },

  /** 沼泽：移动本身要额外耗能（能量不够就落到 0，进入饥饿计时） */
  bog(organism) {
    const cost = num(organism.world.config, 'swampMoveCost', 0.5);
    if (cost <= 0) return;
    organism.energy -= cost;
    if (organism.energy < 0) organism.energy = 0;
  },
};

// ────────────────────────────── 驻留 ──────────────────────────────

export const TERRAIN_STAND = {
  /** 岩浆：待在里面每 tick 继续烧 */
  burn(organism) {
    const dmg = num(organism.world.config, 'lavaStandDamage', 1);
    if (dmg > 0) organism.harm(dmg);
  },

  /**
   * 辐射带：每 tick 扣一点血，同时**大幅提高变异率**。
   *
   * 变异率不是直接改 `organism.mutability`——那会指数爆炸（5% ×3 ×3 ×3… 三 tick 就封顶）。
   * 而是打一个"这一 tick 处于辐射中"的标记，由 Mutation 在**繁殖那一步**
   * 临时乘上倍率。这样"在辐射里待过的个体，后代更容易突变"才是准确的语义。
   */
  irradiate(organism) {
    const dmg = num(organism.world.config, 'radDamage', 0.25);
    if (dmg > 0) organism.harm(dmg);
    if (organism.alive) organism.inRadiation = true;
  },
};

// ────────────────────────────── 世界级每 tick ──────────────────────────────

export const TERRAIN_TICK = {
  /** 腐殖质：以一定概率往相邻空格冒一份食物 */
  fertilize(world, index) {
    const prob = num(world.config, 'humusFoodProb', 0.5);
    if (prob <= 0) return;
    if (!world.rng.chance(prob)) return;

    const grid = world.grid;
    const [col, row] = grid.coords(index);
    const n = NEIGHBORS_4[world.rng.int(4)];
    const target = grid.index(col + n[0], row + n[1]);
    if (target < 0) return;
    if (grid.types[target] !== CellType.EMPTY) return;
    world.setCellAt(target, CellType.FOOD, -1);
  },
};

// ────────────────────────────── 分发 ──────────────────────────────

/** 生物踏入 (col,row) 那格地形时触发。`type` 是**进入前**那格的类型 */
export function applyTerrainEnter(organism, type, col, row) {
  const d = cellDef(type);
  if (!d || !d.enter) return;
  const fn = TERRAIN_ENTER[d.enter];
  if (fn) fn(organism, col, row);
}

/** 生物驻留在 (col,row) 时每 tick 触发 */
export function applyTerrainStand(organism, type, col, row) {
  const d = cellDef(type);
  if (!d || !d.stand) return;
  const fn = TERRAIN_STAND[d.stand];
  if (fn) fn(organism, col, row);
}
