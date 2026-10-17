/**
 * 黄金快照对照 —— JS 版 vs Rust 版。
 *
 * 同一个种子、同一套初始世界（generateDemoWorld + 生命起源 + 8 只随机生物），
 * 两边跑 N tick，每 50 tick 比一次状态摘要。**摘要一致 = 行为逐位一致**。
 *
 *   node tools/parity-check.mjs                 # 默认 5 个种子 × 2000 tick
 *   node tools/parity-check.mjs --ticks 5000 --seeds 1001,4242
 *
 * 退出码 0 = 全通过。
 */

import { spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { RNG } from '../src/core/RNG.js';
import { snapshotConfig } from '../src/core/config.js';
import { World } from '../src/core/World.js';
import { generateDemoWorld } from '../src/core/WorldGen.js';
import { seedOriginOfLife, spawnPopulation } from '../src/life/OrganismFactory.js';

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = join(HERE, '..');
const BIN = join(ROOT, 'rust', 'target', 'release', 'evosim.exe');
const BIN_DEBUG = join(ROOT, 'rust', 'target', 'debug', 'evosim.exe');

function argValue(key, fallback) {
  const i = process.argv.indexOf(key);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}

const TICKS = Number(argValue('--ticks', 2000));
const SEEDS = argValue('--seeds', '1001,4242,7777,31415,271828')
  .split(',')
  .map((s) => Number(s.trim()));
const STEP = Number(argValue('--step', 50));

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

function runJs(seed) {
  const world = new World(snapshotConfig(), new RNG(seed));
  generateDemoWorld(world);
  seedOriginOfLife(world);
  spawnPopulation(world, 8, 3);

  const out = new Map();
  for (let t = 1; t <= TICKS; t++) {
    world.step();
    if (t % STEP === 0) {
      const s = world.snapshotStats();
      out.set(t, `orgs ${s.organisms} food ${s.food} species ${world.species.size} body ${s.body} digest ${digest(world)}`);
    }
  }
  return out;
}

function runRust(seed) {
  const bin = existsSync(BIN) ? BIN : BIN_DEBUG;
  if (!existsSync(bin)) {
    throw new Error(`找不到 Rust 可执行文件：${bin}（先 cd rust && cargo build --release）`);
  }
  const res = spawnSync(bin, ['--sim', '--seed', String(seed), '--ticks', String(TICKS), '--every'], {
    encoding: 'utf8',
    maxBuffer: 512 * 1024 * 1024,
  });
  if (res.error) throw res.error;
  const out = new Map();
  for (const line of res.stdout.split('\n')) {
    const m = line.match(/^tick (\d+) orgs (\d+) food (\d+) species (\d+) body (\d+) digest (\w+)/);
    if (!m) continue;
    const t = Number(m[1]);
    if (t % STEP !== 0) continue;
    out.set(t, `orgs ${m[2]} food ${m[3]} species ${m[4]} body ${m[5]} digest ${m[6]}`);
  }
  return out;
}

let failures = 0;
let checks = 0;

console.log(`黄金快照对照：${SEEDS.length} 个种子 × ${TICKS} tick，每 ${STEP} tick 比一次\n`);

for (const seed of SEEDS) {
  const js = runJs(seed);
  const rs = runRust(seed);
  let firstBad = null;
  let compared = 0;

  for (let t = STEP; t <= TICKS; t += STEP) {
    const a = js.get(t);
    const b = rs.get(t);
    compared++;
    if (a !== b) {
      if (firstBad === null) firstBad = { t, a, b };
      failures++;
    }
  }
  checks += compared;

  if (firstBad) {
    console.log(`FAIL  seed ${seed}：第 ${firstBad.t} tick 起分歧（比较了 ${compared} 个检查点）`);
    console.log(`   JS   ${firstBad.a}`);
    console.log(`   Rust ${firstBad.b}`);
  } else {
    console.log(`PASS  seed ${seed}：${compared} 个检查点全部一致（${TICKS} tick）`);
  }
}

console.log(`\n合计 ${checks} 个检查点，${failures} 个不一致`);
process.exit(failures === 0 ? 0 : 1);
