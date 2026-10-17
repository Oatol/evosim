/**
 * 导出黄金快照 —— Rust 版移植的对照基线。
 *
 * 三份数据：
 *   golden/rng-u32.txt   RNG 前 200 个 u32（十六进制）
 *   golden/gauss.txt     前 200 个高斯样本（f64 位模式，十六进制）
 *   golden/world.txt     3 个种子的世界状态摘要（每 1/10/50/100/200/500 tick 一行）
 *
 * 摘要算法必须与 Rust 的 `World::state_digest()` 完全一致：
 * FNV-1a 依次混入 网格(types/owners/floor) → 每个个体(id/col/row/rot/dir/age/energy/damage/细胞…)。
 *
 * 用法：node tools/export-golden.mjs
 */

import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { RNG } from '../src/core/RNG.js';
import { snapshotConfig } from '../src/core/config.js';
import { World } from '../src/core/World.js';
import { generateDemoWorld } from '../src/core/WorldGen.js';
import { seedOriginOfLife, spawnPopulation } from '../src/life/OrganismFactory.js';

const HERE = dirname(fileURLToPath(import.meta.url));
const OUT = join(HERE, '..', 'rust', 'golden');
mkdirSync(OUT, { recursive: true });

// ── 1. RNG 序列 ──
const SEED = 12345;
{
  const rng = new RNG(SEED);
  const lines = [];
  for (let i = 0; i < 200; i++) lines.push(rng.nextUint32().toString(16).padStart(8, '0'));
  writeFileSync(join(OUT, 'rng-u32.txt'), lines.join('\n') + '\n');
  console.log('rng-u32.txt 200 个 u32');
}

// ── 2. 高斯样本（存 f64 位模式，才能逐位比对）──
{
  const rng = new RNG(SEED);
  const buf = new ArrayBuffer(8);
  const dv = new DataView(buf);
  const lines = [];
  for (let i = 0; i < 200; i++) {
    dv.setFloat64(0, rng.gaussian(0, 1));
    lines.push(dv.getBigUint64(0).toString(16).padStart(16, '0'));
  }
  writeFileSync(join(OUT, 'gauss.txt'), lines.join('\n') + '\n');
  console.log('gauss.txt 200 个高斯样本');
}

// ── 3. 世界摘要 ──
function digest(world) {
  let h = 0x811c9dc5;
  const mix = (v) => {
    h = (h ^ (v >>> 0)) >>> 0;
    h = Math.imul(h, 0x01000193) >>> 0;
  };
  const g = world.grid;
  for (let i = 0; i < g.length; i++) {
    mix(g.types[i]);
    mix(g.owners[i] >>> 0);
    mix(g.floor[i]);
  }
  for (const org of world.organisms) {
    mix(org.id);
    mix(org.col >>> 0);
    mix(org.row >>> 0);
    mix(org.rotation);
    mix(org.direction);
    mix(org.age >>> 0);
    mix(Math.round(org.energy * 1000) | 0);
    mix(org.damage | 0);
    mix(org.anatomy.cells.length);
    for (const c of org.anatomy.cells) {
      mix(c.type);
      mix(c.dx >>> 0);
      mix(c.dy >>> 0);
      mix(c.dir);
    }
  }
  return h.toString(16).padStart(8, '0');
}

const CHECKPOINTS = [1, 10, 50, 100, 200, 500];
const SEEDS = [1001, 4242, 7777];
const lines = [];

for (const seed of SEEDS) {
  const cfg = snapshotConfig();
  const rng = new RNG(seed);
  const world = new World(cfg, rng);
  generateDemoWorld(world);
  seedOriginOfLife(world);
  spawnPopulation(world, 8, 3);

  let cp = 0;
  for (let t = 1; t <= 500; t++) {
    world.step();
    if (cp < CHECKPOINTS.length && world.tick === CHECKPOINTS[cp]) {
      const s = world.snapshotStats();
      lines.push(`seed ${seed} tick ${world.tick} orgs ${s.organisms} food ${s.food} species ${world.species.size} body ${s.body} digest ${digest(world)}`);
      cp++;
    }
  }
  console.log(`seed ${seed} 完成 500 tick`);
}

writeFileSync(join(OUT, 'world.txt'), lines.join('\n') + '\n');
console.log('world.txt 写入完成：' + lines.length + ' 行');
