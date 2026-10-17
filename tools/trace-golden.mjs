/**
 * 逐 tick 轨迹导出（定位 Rust 移植偏差用）。
 *
 *   node tools/trace-golden.mjs --seed 4242 --ticks 200
 *   node tools/trace-golden.mjs --seed 4242 --dump-orgs 137
 */

import { RNG } from '../src/core/RNG.js';
import { snapshotConfig } from '../src/core/config.js';
import { World } from '../src/core/World.js';
import { generateDemoWorld } from '../src/core/WorldGen.js';
import { seedOriginOfLife, spawnPopulation } from '../src/life/OrganismFactory.js';

function argValue(key, fallback) {
  const i = process.argv.indexOf(key);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}

const seed = Number(argValue('--seed', 4242));
const ticks = Number(argValue('--ticks', 200));
const dumpOrgs = argValue('--dump-orgs', null);

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

const world = new World(snapshotConfig(), new RNG(seed));
generateDemoWorld(world);
seedOriginOfLife(world);
spawnPopulation(world, 8, 3);

if (dumpOrgs) {
  const target = Number(dumpOrgs);
  while (world.tick < target) world.step();
  console.log(`tick ${world.tick} count ${world.organisms.length}`);
  for (const org of world.organisms) {
    const cells = org.anatomy.cells
      .map((c) => `${c.type}:${c.dx},${c.dy}:${c.dir},`)
      .join('');
    console.log(
      `org ${org.id} col ${org.col} row ${org.row} rot ${org.rotation} dir ${org.direction} ` +
      `age ${org.age} energy ${org.energy.toFixed(6)} damage ${org.damage.toFixed(6)} ` +
      `mut ${org.mutability.toFixed(4)} cells ${cells}`,
    );
  }
} else {
  for (let t = 1; t <= ticks; t++) {
    world.step();
    const s = world.snapshotStats();
    console.log(`tick ${world.tick} orgs ${s.organisms} food ${s.food} species ${world.species.size} body ${s.body} digest ${digest(world)}`);
  }
}
