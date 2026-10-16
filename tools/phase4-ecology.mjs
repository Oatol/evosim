/**
 * 阶段 4 生态观察：神经网络接上之后，"捕食者-猎物"到底有没有自发涌现？
 *
 * 这不是断言式测试，而是**测量**——验收标准里的"捕食者-猎物行为自发涌现"
 * 没法用一条 assert 判定，只能用几个代理指标去看：
 *
 *   1. 杀手细胞是否富集在移动者身上   P(杀手 | 会移动) vs P(杀手)
 *      随机组合的话两者应该接近；明显更高说明"杀手+移动器"是被选择出来的组合，
 *      也就是"会主动追上去杀"而不是"守株待兔"。
 *   2. 眼睛是否富集在"会移动的杀手"身上   追击需要方向信息，没有眼睛只能乱撞。
 *   3. 稳定期种群是否有周期性波动   捕食者-猎物循环的典型特征是种群振荡，
 *      而不是单调收敛到一个平衡点。用变异系数 + 自相关来粗看。
 *
 * 纯 node，无 DOM 依赖。
 */

import { snapshotConfig } from '../src/core/config.js';
import { RNG } from '../src/core/RNG.js';
import { World } from '../src/core/World.js';
import { generateDemoWorld } from '../src/core/WorldGen.js';
import { seedOriginOfLife, spawnPopulation } from '../src/life/OrganismFactory.js';

const TICKS = 20000;
const SAMPLE_EVERY = 100;
const WARMUP = 3000;          // 跳过初始瞬态
const SEEDS = [12345, 777, 20261015];

const BLOCKS = '▁▂▃▄▅▆▇█';

function sparkline(values) {
  if (values.length === 0) return '';
  let lo = Infinity;
  let hi = -Infinity;
  for (const v of values) { if (v < lo) lo = v; if (v > hi) hi = v; }
  const span = hi - lo || 1;
  let out = '';
  for (const v of values) {
    const idx = Math.min(BLOCKS.length - 1, Math.floor(((v - lo) / span) * BLOCKS.length));
    out += BLOCKS[idx];
  }
  return out;
}

function mean(a) { return a.reduce((s, v) => s + v, 0) / a.length; }
function stdev(a) {
  const m = mean(a);
  return Math.sqrt(a.reduce((s, v) => s + (v - m) ** 2, 0) / a.length);
}

/** 滞后 k 的自相关系数（衡量周期性） */
function autocorr(a, lag) {
  const m = mean(a);
  let num = 0;
  let den = 0;
  for (let i = 0; i < a.length; i++) den += (a[i] - m) ** 2;
  for (let i = 0; i + lag < a.length; i++) num += (a[i] - m) * (a[i + lag] - m);
  return den === 0 ? 0 : num / den;
}

function run(seed) {
  const cfg = snapshotConfig();
  const rng = new RNG(seed);
  const world = new World(cfg, rng);
  generateDemoWorld(world);
  seedOriginOfLife(world);
  spawnPopulation(world, 6);

  const popSeries = [];
  const speciesSeries = [];
  const cellsSeries = [];
  const moverSeries = [];
  const killerSeries = [];
  const eyeSeries = [];

  // 富集统计：在稳定期累计
  let totalOrgs = 0;
  let withKiller = 0;
  let withMover = 0;
  let moverAndKiller = 0;
  let moverWithEye = 0;
  let killerWithEye = 0;

  for (let t = 1; t <= TICKS; t++) {
    world.step();
    if (t % SAMPLE_EVERY !== 0) continue;

    const bio = world.organismStats();
    popSeries.push(world.organisms.length);
    speciesSeries.push(world.species.aliveCount);
    cellsSeries.push(bio.avgCells);
    moverSeries.push(bio.movers);
    killerSeries.push(bio.killers);
    eyeSeries.push(bio.eyes);

    if (t > WARMUP) {
      for (const o of world.organisms) {
        const mv = o.anatomy.hasMover;
        const kl = o.anatomy.hasKiller;
        const ey = o.anatomy.hasEye;
        totalOrgs++;
        if (kl) withKiller++;
        if (mv) withMover++;
        if (mv && kl) moverAndKiller++;
        if (mv && ey) moverWithEye++;
        if (kl && ey) killerWithEye++;
      }
      world.species.update(world, t);
    }
  }

  const settled = popSeries.filter((_, i) => i * SAMPLE_EVERY > WARMUP);

  return {
    seed,
    popSeries,
    settled,
    speciesSeries,
    cellsSeries,
    moverSeries,
    killerSeries,
    eyeSeries,
    finalPop: world.organisms.length,
    finalSpecies: world.species.aliveCount,
    totalSpecies: world.species.list.length,
    peakSpecies: world.species.peakAliveCount,
    totalOrgs,
    withKiller,
    withMover,
    moverAndKiller,
    moverWithEye,
    killerWithEye,
    avgCells: cellsSeries[cellsSeries.length - 1],
  };
}

console.log(`阶段 4 生态观察 · ${TICKS} tick × ${SEEDS.length} 个种子\n`);

const results = [];
for (const seed of SEEDS) {
  process.stdout.write(`  跑种子 ${seed} ... `);
  const t0 = Date.now();
  const r = run(seed);
  console.log(`${((Date.now() - t0) / 1000).toFixed(1)}s`);
  results.push(r);
}

console.log(`\n${'='.repeat(66)}`);
console.log('种群曲线（每 100 tick 一个采样点，越低越高）');
console.log(`${'='.repeat(66)}`);
for (const r of results) {
  console.log(`seed ${String(r.seed).padEnd(9)} ${sparkline(r.popSeries)}`);
}

console.log(`\n${'='.repeat(66)}`);
console.log('代理指标 1 · 杀手是否富集在移动者身上');
console.log(`${'='.repeat(66)}`);
console.log('  P(杀手)           = 全种群中带杀手的比例');
console.log('  P(杀手|会移动)     = 会移动的个体里带杀手的比例');
console.log('  富集倍数          = 两者之比。≈1 说明杀手与移动器是独立随机组合的；');
console.log('                     明显 >1 说明"杀手+移动器"是被选择出来的搭配。\n');
console.log('  seed      P(杀手)  P(杀手|移动)  富集倍数   移动者占比');
for (const r of results) {
  const pKiller = r.withKiller / r.totalOrgs;
  const pMover = r.withMover / r.totalOrgs;
  const pKillerGivenMover = r.moverAndKiller / Math.max(1, r.withMover);
  const enrich = pKillerGivenMover / Math.max(1e-9, pKiller);
  console.log(`  ${String(r.seed).padEnd(9)} ${(pKiller * 100).toFixed(2).padStart(6)}% ` +
    `${(pKillerGivenMover * 100).toFixed(2).padStart(10)}% ` +
    `${enrich.toFixed(2).padStart(9)}× ` +
    `${(pMover * 100).toFixed(2).padStart(9)}%`);
}

console.log(`\n${'='.repeat(66)}`);
console.log('代理指标 2 · 眼睛是否富集在"会移动的个体"身上（追击需要方向信息）');
console.log(`${'='.repeat(66)}`);
console.log('  seed      P(眼|移动)  P(眼|杀手)   P(眼)');
for (const r of results) {
  const pEyeGivenMover = r.moverWithEye / Math.max(1, r.withMover);
  const pEyeGivenKiller = r.killerWithEye / Math.max(1, r.withKiller);
  console.log(`  ${String(r.seed).padEnd(9)} ${(pEyeGivenMover * 100).toFixed(2).padStart(9)}% ` +
    `${(pEyeGivenKiller * 100).toFixed(2).padStart(10)}% ` +
    `${(r.eyeSeries[r.eyeSeries.length - 1] / Math.max(1, r.finalPop) * 100).toFixed(2).padStart(8)}%`);
}

console.log(`\n${'='.repeat(66)}`);
console.log('代理指标 3 · 稳定期种群是否有周期性波动');
console.log(`${'='.repeat(66)}`);
console.log('  变异系数 = 标准差 / 均值。越小越接近"卡在一个平衡点"。');
console.log('  自相关(lag=5) 越接近 1 越像平滑趋势，出现明显负值说明有周期。\n');
console.log('  seed      均值    标准差  变异系数   自相关(lag5)  自相关(lag20)');
for (const r of results) {
  const m = mean(r.settled);
  const sd = stdev(r.settled);
  console.log(`  ${String(r.seed).padEnd(9)} ${m.toFixed(0).padStart(6)} ${sd.toFixed(0).padStart(8)} ` +
    `${(sd / m).toFixed(3).padStart(9)} ${autocorr(r.settled, 5).toFixed(3).padStart(13)} ` +
    `${autocorr(r.settled, 20).toFixed(3).padStart(14)}`);
}

console.log(`\n${'='.repeat(66)}`);
console.log('生态终态');
console.log(`${'='.repeat(66)}`);
console.log('  seed      最终种群  存活物种  累计物种  峰值物种  平均体型');
for (const r of results) {
  console.log(`  ${String(r.seed).padEnd(9)} ${String(r.finalPop).padStart(8)} ` +
    `${String(r.finalSpecies).padStart(9)} ${String(r.totalSpecies).padStart(9)} ` +
    `${String(r.peakSpecies).padStart(9)} ${r.avgCells.toFixed(2).padStart(9)}`);
}

console.log(`\n${'='.repeat(66)}`);
console.log('会移动 / 有眼睛 / 有杀手 三条曲线的走势（第一个种子）');
console.log(`${'='.repeat(66)}`);
const r0 = results[0];
console.log(`  会移动  ${sparkline(r0.moverSeries)}`);
console.log(`  有眼睛  ${sparkline(r0.eyeSeries)}`);
console.log(`  有杀手  ${sparkline(r0.killerSeries)}`);
console.log(`  平均体型 ${sparkline(r0.cellsSeries)}`);
console.log(`  存活物种 ${sparkline(r0.speciesSeries)}`);
