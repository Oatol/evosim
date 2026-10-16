/**
 * 代谢消耗参数扫描。
 *
 * 引入"每 tick 能量消耗"后，生态能不能撑住取决于一个很窄的量级：
 * 生产者细胞的造食率只有 5%/tick，一个三细胞起源大约每秒（tick）只能拿到
 * 0.05 能量。所以细胞消耗必须远小于这个数，否则整个种群会集体饿死。
 *
 * 这个脚本用数据（而不是直觉）来定默认值——和阶段 2 的腐败率、
 * 阶段 3 的变异率一样。
 *
 * 用法：node tools/phase7-sweep.mjs
 */

import { Config, snapshotConfig, restoreConfig } from '../src/core/config.js';
import { RNG } from '../src/core/RNG.js';
import { World } from '../src/core/World.js';
import { generateDemoWorld } from '../src/core/WorldGen.js';
import { seedOriginOfLife, spawnPopulation } from '../src/life/OrganismFactory.js';

const TICKS = 12000;
const WARMUP = 2000;      // 跳过刚投放时的瞬态
const SETTLED = 6000;     // 后段用来算"稳定期"
const SEEDS = [1234, 5678, 9012];

/** 一套待扫描的消耗方案。基础向量按"三细胞起源的收支"反推：约 0.006/tick */
const BASE = { MOUTH: 0.002, PRODUCER: 0.002, MOVER: 0.008, KILLER: 0.006, ARMOR: 0.004, EYE: 0.006 };

const PLANS = [
  { name: '关闭消耗（scale=0）', costs: null, scale: 0 },
  { name: '消耗×1', costs: BASE, scale: 1 },
  { name: '消耗×1 + 造食15%', costs: BASE, scale: 1, prod: 15 },
  { name: '消耗×1 + 造食25%', costs: BASE, scale: 1, prod: 25 },
  { name: '消耗×2 + 造食25%', costs: BASE, scale: 2, prod: 25 },
  { name: '消耗×1 + 造食40%', costs: BASE, scale: 1, prod: 40 },
];

function runOne(plan, seed) {
  restoreConfig(snapshotConfig());          // 先恢复出厂，再改
  const cfg = snapshotConfig();
  cfg.life.metabolismScale = plan.scale;
  if (plan.prod !== undefined) cfg.life.foodProdProb = plan.prod;
  if (plan.costs) {
    for (const [k, v] of Object.entries(plan.costs)) cfg.cells[k].cost = v;
  }

  const rng = new RNG(seed);
  const world = new World(cfg, rng);
  generateDemoWorld(world);
  seedOriginOfLife(world);
  spawnPopulation(world, 6);

  let minPop = Infinity;
  let settledSum = 0;
  let settledN = 0;
  let peakSpecies = 0;

  for (let t = 1; t <= TICKS; t++) {
    world.step();
    const pop = world.organisms.length;
    if (t > WARMUP && pop < minPop) minPop = pop;
    if (t > SETTLED) { settledSum += pop; settledN++; }
    if (t % Config.stats.interval === 0) {
      world.species.update(world, t);
      world.species.prune();
      if (world.species.aliveCount > peakSpecies) peakSpecies = world.species.aliveCount;
    }
  }

  const bio = world.organismStats();
  return {
    pop: world.organisms.length,
    minPop: minPop === Infinity ? 0 : minPop,
    settled: settledN ? settledSum / settledN : 0,
    peakSpecies,
    avgCells: bio.avgCells,
    movers: bio.movers,
  };
}

console.log(`代谢消耗扫描：${TICKS} tick × ${SEEDS.length} 个种子\n`);
const header = ['方案', '末种群', '稳定期均值', '最低种群', '峰值物种', '平均体型', '会移动的'];
console.log(header.map((h, i) => (i === 0 ? h.padEnd(18) : h.padStart(11))).join(''));

for (const plan of PLANS) {
  const rows = SEEDS.map((s) => runOne(plan, s));
  const avg = (k) => rows.reduce((a, r) => a + r[k], 0) / rows.length;
  const extinct = rows.filter((r) => r.pop === 0).length;
  const label = plan.name + (extinct ? ` ⚠${extinct}/3 灭绝` : '');
  console.log([
    label.padEnd(18),
    avg('pop').toFixed(0).padStart(11),
    avg('settled').toFixed(0).padStart(11),
    avg('minPop').toFixed(0).padStart(11),
    avg('peakSpecies').toFixed(0).padStart(11),
    avg('avgCells').toFixed(2).padStart(11),
    avg('movers').toFixed(0).padStart(11),
  ].join(''));
}

restoreConfig(snapshotConfig());
