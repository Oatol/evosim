/**
 * 变异率扫描：不同变异率下的长期生态表现。
 *
 * 目的和阶段 2 的腐败率扫描一样——用数据而不是直觉来定默认参数。
 * 关注三件事：种群能不能稳住、会不会经历灭绝瓶颈、物种多样性有多少。
 */

import { snapshotConfig } from '../src/core/config.js';
import { RNG } from '../src/core/RNG.js';
import { World } from '../src/core/World.js';
import { generateDemoWorld } from '../src/core/WorldGen.js';
import { seedOriginOfLife, spawnPopulation } from '../src/life/OrganismFactory.js';

const TICKS = 12000;
const SEEDS = [20261015, 777, 31337];
const RATES = [0, 1, 2, 3, 5, 8, 12];

// 前 2000 tick 是初始瞬态：刚投放的 7 个生物还没繁殖开，
// 统计最小值时必须跳过，否则每个配置都会报"最低 7"。
const WARMUP = 2000;
// 后半段用来衡量"稳定期"的种群水平
const SETTLED = 6000;

console.log(`变异率扫描 · ${TICKS} tick × ${SEEDS.length} 个种子 · 200×150\n`);
console.log('  变异率  稳定期种群   最低种群   峰值物种   存活物种   平均体型   耗时');
console.log('  ' + '─'.repeat(70));

for (const rate of RATES) {
  let sumSettledPop = 0;
  let sumSettledSamples = 0;
  let worstMin = Infinity;
  let sumPeakSpecies = 0;
  let sumAliveSpecies = 0;
  let sumAvgCells = 0;
  let ms = 0;

  for (const seed of SEEDS) {
    const cfg = snapshotConfig();
    cfg.mutation.useGlobalMutability = true;
    cfg.mutation.globalMutability = rate;

    const rng = new RNG(seed);
    const world = new World(cfg, rng);
    generateDemoWorld(world);
    seedOriginOfLife(world);
    spawnPopulation(world, 6);

    const t0 = Date.now();
    let minPop = Infinity;
    let peakSpecies = 0;
    let settledPop = 0;
    let settledSamples = 0;

    for (let t = 0; t < TICKS; t++) {
      world.step();
      if (world.tick % cfg.stats.interval === 0) {
        world.species.update(world, world.tick);
        world.species.prune();

        const pop = world.organisms.length;
        if (world.tick > WARMUP && pop < minPop) minPop = pop;
        if (world.tick > SETTLED) { settledPop += pop; settledSamples++; }
        if (world.species.aliveCount > peakSpecies) peakSpecies = world.species.aliveCount;
      }
    }
    ms += Date.now() - t0;

    sumSettledPop += settledSamples > 0 ? settledPop / settledSamples : 0;
    sumSettledSamples++;
    if (minPop < worstMin) worstMin = minPop;
    sumPeakSpecies += peakSpecies;
    sumAliveSpecies += world.species.aliveCount;
    sumAvgCells += world.organismStats().avgCells;
  }

  const n = sumSettledSamples;
  const flag = worstMin === 0 ? '  ← 灭绝' : worstMin < 30 ? '  ← 瓶颈' : '';
  console.log(
    `  ${String(rate).padStart(4)}%  ` +
    `${(sumSettledPop / n).toFixed(0).padStart(9)}  ` +
    `${String(worstMin === Infinity ? 0 : worstMin).padStart(9)}  ` +
    `${(sumPeakSpecies / n).toFixed(0).padStart(8)}  ` +
    `${(sumAliveSpecies / n).toFixed(0).padStart(8)}  ` +
    `${(sumAvgCells / n).toFixed(2).padStart(8)}  ` +
    `${(ms / n).toFixed(0).padStart(4)}ms` + flag,
  );
}

console.log('\n注：最低种群是三个种子中最差的一个（跳过前 2000 tick 的初始瞬态）——它决定"会不会灭绝"。');
console.log('    稳定期种群是 tick > 6000 之后的采样平均。');
