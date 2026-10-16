/**
 * 阶段 3 逻辑测试：变异算子 / 元变异 / 物种注册表 / 化石记录 / 繁殖集成 / 长程生态。
 * 纯 node，无 DOM 依赖。
 */

import { Config, snapshotConfig } from '../src/core/config.js';
import { RNG } from '../src/core/RNG.js';
import { World } from '../src/core/World.js';
import { CellType, LIVING_CELL_TYPES } from '../src/life/CellType.js';
import { Anatomy } from '../src/life/Anatomy.js';
import { Organism } from '../src/life/Organism.js';
import { mutateAnatomy, mutateOrganism } from '../src/life/Mutation.js';
import { MAX_SPECIES } from '../src/stats/Species.js';
import { FossilRecord } from '../src/stats/FossilRecord.js';
import { generateDemoWorld } from '../src/core/WorldGen.js';
import { seedOriginOfLife, spawnPopulation, createOriginOrganism } from '../src/life/OrganismFactory.js';

let pass = 0;
let fail = 0;

function ok(name, cond, extra = '') {
  if (cond) { pass++; console.log(`PASS  ${name}${extra ? '  → ' + extra : ''}`); }
  else { fail++; console.log(`FAIL  ${name}${extra ? '  → ' + extra : ''}`); }
}

function eq(name, actual, expected) {
  ok(name, actual === expected, `实际 ${actual}，期望 ${expected}`);
}

function section(title) {
  console.log(`\n=== ${title} ===`);
}

/** 造一个装了基础世界的测试环境 */
function makeWorld(seed = 12345) {
  const cfg = snapshotConfig();
  const rng = new RNG(seed);
  const world = new World(cfg, rng);
  // World 持有的是传入的 config 对象，这里直接返回便于修改
  return { world, rng, cfg };
}

/** 干净的 3 细胞蓝图：嘴 + 双生产者 */
function originAnatomy() {
  const a = new Anatomy();
  a.addCell(CellType.MOUTH, 0, 0);
  a.addCell(CellType.PRODUCER, 1, 1);
  a.addCell(CellType.PRODUCER, -1, -1);
  return a;
}

// ────────────────────────────────────────────────────────────
section('1. 结构变异算子');

{
  const { world, rng } = makeWorld();
  const base = originAnatomy();

  // 只开"加细胞"
  const mut = { addProb: 100, changeProb: 0, removeProb: 0 };
  const a1 = originAnatomy();
  const changed = mutateAnatomy(a1, rng, mut);
  ok('addProb=100 时结构发生变化', changed === true);
  eq('加细胞后细胞数 +1', a1.cells.length, base.cells.length + 1);

  // 只开"删细胞"
  const mut2 = { addProb: 0, changeProb: 0, removeProb: 100 };
  const a2 = originAnatomy();
  const changed2 = mutateAnatomy(a2, rng, mut2);
  ok('removeProb=100 时结构发生变化', changed2 === true);
  eq('删细胞后细胞数 -1', a2.cells.length, base.cells.length - 1);

  // 全关
  const mut3 = { addProb: 0, changeProb: 0, removeProb: 0 };
  const a3 = originAnatomy();
  const before = a3.fingerprint();
  const changed3 = mutateAnatomy(a3, rng, mut3);
  ok('全部概率为 0 时结构不变', changed3 === false);
  eq('全部概率为 0 时指纹不变', a3.fingerprint(), before);
}

{
  // 删到只剩中心格后不能再删
  const { rng } = makeWorld();
  const a = new Anatomy();
  a.addCell(CellType.MOUTH, 0, 0);
  const mut = { addProb: 0, changeProb: 0, removeProb: 100 };
  const changed = mutateAnatomy(a, rng, mut);
  ok('只剩中心格时删细胞无效', changed === false);
  eq('中心格仍在', a.cells.length, 1);
}

{
  // 改细胞：反复试，应当能观察到类型变化
  const { rng } = makeWorld();
  const mut = { addProb: 0, changeProb: 100, removeProb: 0 };
  let sawChange = false;
  for (let i = 0; i < 40 && !sawChange; i++) {
    const a = originAnatomy();
    const before = a.describe();
    mutateAnatomy(a, rng, mut);
    if (a.describe() !== before) sawChange = true;
  }
  ok('changeProb=100 时能观察到类型改变', sawChange);
}

{
  // 加细胞不会把格子叠在一起
  const { rng } = makeWorld();
  const mut = { addProb: 100, changeProb: 0, removeProb: 0 };
  let overlap = false;
  for (let i = 0; i < 60; i++) {
    const a = originAnatomy();
    for (let k = 0; k < 6; k++) mutateAnatomy(a, rng, mut);
    const seen = new Set();
    for (const c of a.cells) {
      const key = `${c.dx},${c.dy}`;
      if (seen.has(key)) overlap = true;
      seen.add(key);
    }
  }
  ok('连续加细胞不会产生重叠格', !overlap);
}

{
  // 所有新细胞都是合法的生物细胞类型
  const { rng } = makeWorld();
  const mut = { addProb: 100, changeProb: 0, removeProb: 0 };
  let bad = false;
  for (let i = 0; i < 60; i++) {
    const a = originAnatomy();
    for (let k = 0; k < 5; k++) mutateAnatomy(a, rng, mut);
    for (const c of a.cells) {
      if (!LIVING_CELL_TYPES.includes(c.type)) bad = true;
    }
  }
  ok('变异产生的细胞类型都合法', !bad);
}

// ────────────────────────────────────────────────────────────
section('2. 元变异');

{
  const { world, rng } = makeWorld();
  world.fillGrid(CellType.EMPTY);

  const parent = new Organism(world, 10, 10);
  parent.anatomy.copyFrom(originAnatomy());
  parent.mutability = 12;
  world.addOrganism(parent);

  // 变异率 0 → 子代继承母体的变异率，且结构不变
  const saved = { ...Config.mutation };
  Config.mutation.addProb = 0;
  Config.mutation.changeProb = 0;
  Config.mutation.removeProb = 0;
  Config.mutation.useGlobalMutability = false;

  const child = new Organism(world, 20, 20);
  child.anatomy.copyFrom(parent.anatomy);
  const changed = mutateOrganism(child, parent, rng, Config);
  eq('母体变异率 12 → 子代继承 12', child.mutability, 12);
  ok('结构未变', changed === false);
  eq('结构未变时指纹沿用母体', child.speciesFingerprint, parent.speciesFingerprint);
  eq('祖先指纹指向母体物种', child.parentFingerprint, parent.speciesFingerprint);

  // 变异率 100 → 结构必变，变异率漂移
  Config.mutation.addProb = 100;
  let drifted = false;
  for (let i = 0; i < 30; i++) {
    const c2 = new Organism(world, 30 + i, 30);
    c2.anatomy.copyFrom(parent.anatomy);
    mutateOrganism(c2, parent, rng, Config);
    if (c2.mutability !== 12) drifted = true;
  }
  ok('结构变异发生时变异率会漂移', drifted);

  Object.assign(Config.mutation, saved);
}

{
  // 全局变异率开关：开启后个体自身的 mutability 被忽略
  const { world, rng } = makeWorld();
  const saved = { ...Config.mutation };
  Config.mutation.useGlobalMutability = true;
  Config.mutation.globalMutability = 100;
  Config.mutation.addProb = 100;
  Config.mutation.changeProb = 0;
  Config.mutation.removeProb = 0;

  const parent = new Organism(world, 10, 10);
  parent.anatomy.copyFrom(originAnatomy());
  parent.mutability = 1;              // 个体变异率很低
  world.addOrganism(parent);

  const child = new Organism(world, 30, 30);
  child.anatomy.copyFrom(parent.anatomy);
  const changed = mutateOrganism(child, parent, rng, Config);
  ok('全局变异率开启时个体低变异率被覆盖', changed === true);

  Object.assign(Config.mutation, saved);
}

// ────────────────────────────────────────────────────────────
section('3. 物种注册表');

{
  const { world, rng } = makeWorld();
  world.fillGrid(CellType.EMPTY);

  const org = createOriginOrganism(world, 20, 20);
  world.addOrganism(org);
  eq('首次投放创建 1 个物种', world.species.list.length, 1);
  eq('该物种累计个体数 = 1', world.species.list[0].cumulativePop, 1);
  ok('起源物种没有祖先', world.species.list[0].ancestorId === null);
  ok('指纹已填充', typeof org.speciesFingerprint === 'string' && org.speciesFingerprint.length === 8);

  // 再投放一个同结构的生物 → 不产生新物种
  const org2 = createOriginOrganism(world, 40, 40);
  world.addOrganism(org2);
  eq('同结构再投放不产生新物种', world.species.list.length, 1);
  eq('累计个体数累加到 2', world.species.list[0].cumulativePop, 2);
  eq('两个个体指纹相同', org.speciesFingerprint, org2.speciesFingerprint);

  // 手动造一个结构不同的生物
  const mutant = new Organism(world, 60, 60);
  mutant.anatomy.copyFrom(org.anatomy);
  mutant.anatomy.addCell(CellType.MOVER, 0, 1);
  mutant.parentFingerprint = org.speciesFingerprint;
  world.addOrganism(mutant);
  eq('结构不同 → 产生第 2 个物种', world.species.list.length, 2);

  const sp2 = world.species.list[1];
  eq('新物种的祖先指向起源物种', sp2.ancestorId, world.species.list[0].id);
  ok('新物种描述非空', sp2.description.length > 0, sp2.description);
}

{
  // update：统计个体数与灭绝
  const { world } = makeWorld();
  world.fillGrid(CellType.EMPTY);

  const a = createOriginOrganism(world, 20, 20);
  world.addOrganism(a);
  const b = new Organism(world, 40, 40);
  b.anatomy.copyFrom(a.anatomy);
  b.anatomy.addCell(CellType.MOVER, 0, 1);
  b.parentFingerprint = a.speciesFingerprint;
  world.addOrganism(b);

  world.species.update(world, 100);
  eq('两个物种都存活', world.species.aliveCount, 2);
  eq('物种 1 个体数 = 1', world.species.list[0].population, 1);
  eq('物种 2 个体数 = 1', world.species.list[1].population, 1);
  eq('历史最高存活物种数 = 2', world.species.peakAliveCount, 2);

  // 移除第二个物种的个体 → 该物种应标记灭绝
  world.organisms.splice(1, 1);
  world.species.update(world, 200);
  eq('移除个体后存活物种数 = 1', world.species.aliveCount, 1);
  ok('物种 2 已标记灭绝', world.species.list[1].alive === false);
  eq('物种 2 灭绝时刻 = 200', world.species.list[1].endTick, 200);
  eq('累计灭绝数 = 1', world.species.totalExtinct, 1);
}

{
  // prune：丢弃"昙花一现"的灭绝物种
  const cfg = snapshotConfig();
  const rng = new RNG(1);
  const world = new World(cfg, rng);
  world.fillGrid(CellType.EMPTY);

  const base = createOriginOrganism(world, 20, 20);
  world.addOrganism(base);

  // 造 30 个各不相同的"畸形"物种，每个只出现 1 次（累计个体数 1 < minDiscard 10）
  // 偏移从 (2,1) 起，避开起源蓝图已经占用的 (1,1) 和 (-1,-1)
  for (let i = 0; i < 30; i++) {
    const m = new Organism(world, 40 + i, 40);
    m.anatomy.copyFrom(base.anatomy);
    m.anatomy.addCell(CellType.MOVER, (i % 5) + 2, Math.floor(i / 5) + 1);
    m.parentFingerprint = base.speciesFingerprint;
    world.addOrganism(m);
  }
  const total = world.species.list.length;
  ok('共创建 31 个物种', total === 31, `实际 ${total}`);

  // 除了起源物种，其余全部移除个体 → 灭绝
  world.organisms.length = 1;
  world.species.update(world, 50);

  const removed = world.species.prune(10);       // 上限压到 10，强制剪枝
  ok('剪枝丢弃了噪声物种', removed > 0, `丢弃 ${removed} 个`);
  ok('剪枝后物种数 <= 上限', world.species.list.length <= 10, `实际 ${world.species.list.length}`);
  ok('存活物种未被误删', world.species.byFingerprint(base.speciesFingerprint) !== null);
  ok('剪枝后 map 与 list 一致', world.species.map.size === world.species.list.length);
}

{
  // 物种数上限常量合理
  ok('MAX_SPECIES 是个正数上限', MAX_SPECIES > 0 && MAX_SPECIES <= 20000, String(MAX_SPECIES));
}

// ────────────────────────────────────────────────────────────
section('4. 化石记录（环形缓冲）');

{
  const rec = new FossilRecord(5);
  eq('初始为空', rec.length, 0);
  eq('空时 latest 返回 0', rec.latest('populations'), 0);

  for (let i = 0; i < 5; i++) {
    rec.push({ tick: i * 10, population: i, species: i + 1, avgCells: 3 + i * 0.1, avgMutability: 5 });
  }
  eq('写入 5 个样本', rec.length, 5);
  eq('latest 取到最新值', rec.latest('populations'), 4);

  // 再写 3 个，最旧的 3 个应被覆盖
  for (let i = 5; i < 8; i++) {
    rec.push({ tick: i * 10, population: i, species: i + 1, avgCells: 3 + i * 0.1, avgMutability: 5 });
  }
  eq('容量满后仍为 5', rec.length, 5);
  const data = rec.extract('populations', 100);
  eq('抽稀后点数 = 5', data.n, 5);
  eq('最旧的样本是 tick=30', data.ticks[0], 30);
  eq('最新的样本是 tick=70', data.ticks[data.n - 1], 70);
  eq('对应种群数 3..7', data.values[0], 3);
  eq('最新种群数 = 7', data.values[data.n - 1], 7);

  // 降采样时末点必须保留
  const rec2 = new FossilRecord(1000);
  for (let i = 0; i < 1000; i++) {
    rec2.push({ tick: i, population: i, species: 0, avgCells: 0, avgMutability: 0 });
  }
  const d2 = rec2.extract('populations', 50);
  ok('抽稀后点数 <= 50', d2.n <= 50, String(d2.n));
  eq('抽稀后末点仍是最后一个样本', d2.values[d2.n - 1], 999);

  // reset
  rec.reset();
  eq('reset 后长度归零', rec.length, 0);
  eq('reset 后 latest 归零', rec.latest('populations'), 0);
}

{
  // cellShares 按类型分开存
  const rec = new FossilRecord(4);
  const shares = new Float64Array(9);
  shares[CellType.MOUTH] = 0.5;
  shares[CellType.PRODUCER] = 0.5;
  rec.push({ tick: 0, population: 1, species: 1, avgCells: 2, avgMutability: 5, cellShares: shares });
  eq('shares 写入正确（嘴）', rec.shares[CellType.MOUTH * rec.size + 0], 0.5);
  eq('shares 写入正确（生产者）', rec.shares[CellType.PRODUCER * rec.size + 0], 0.5);
}

// ────────────────────────────────────────────────────────────
section('5. 繁殖集成');

{
  // 变异率 100% 时，子代指纹应当经常与母体不同
  const cfg = snapshotConfig();
  cfg.mutation.useGlobalMutability = true;
  cfg.mutation.globalMutability = 100;
  cfg.mutation.addProb = 100;
  cfg.mutation.changeProb = 100;
  cfg.mutation.removeProb = 0;

  const rng = new RNG(777);
  const world = new World(cfg, rng);
  world.fillGrid(CellType.EMPTY);

  const parent = createOriginOrganism(world, 50, 50);
  parent.energy = 1000;
  world.addOrganism(parent);

  let newSpecies = 0;
  let births = 0;
  // 只让母体繁殖，所以出生数不会很多——母体不移动，子代都堆在它周围
  // 半径 4-6 格的圆环上，能放下的个体有限。这里验证的是"能生出变异体"，
  // 不是"能生多少"。
  for (let i = 0; i < 200; i++) {
    const before = world.species.list.length;
    parent.energy = 1000;
    if (parent.reproduce()) {
      births++;
      if (world.species.list.length > before) newSpecies++;
    }
  }
  ok('高变异率下产生了子代', births >= 3, `出生 ${births}`);
  ok('高变异率下产生了新物种', newSpecies > 0, `新物种 ${newSpecies}`);
  ok('每个新物种都对应一次成功出生', newSpecies <= births);
}

{
  // 繁殖出的子代不能和母体重叠
  const cfg = snapshotConfig();
  cfg.mutation.useGlobalMutability = true;
  cfg.mutation.globalMutability = 100;
  cfg.mutation.addProb = 100;
  cfg.mutation.changeProb = 0;
  cfg.mutation.removeProb = 0;

  const rng = new RNG(4242);
  const world = new World(cfg, rng);
  world.fillGrid(CellType.EMPTY);

  const parent = createOriginOrganism(world, 60, 60);
  world.addOrganism(parent);

  let overlaps = 0;
  for (let i = 0; i < 150; i++) {
    parent.energy = 1000;
    parent.reproduce();
  }

  // 检查网格 owners 是否有冲突：同一个 index 被两个生物写
  const ownerOf = new Map();
  for (const org of world.organisms) {
    for (const cell of org.anatomy.cells) {
      const index = world.grid.index(org.realCol(cell), org.realRow(cell));
      if (ownerOf.has(index) && ownerOf.get(index) !== org.id) overlaps++;
      ownerOf.set(index, org.id);
    }
  }
  ok('大量变异繁殖后网格无重叠', overlaps === 0, `重叠 ${overlaps} 处`);
  ok('确实繁殖出了多个个体', world.organisms.length > 5, `${world.organisms.length} 个`);
}

{
  // 变异率为 0 时指纹完全不变（纯克隆）
  const cfg = snapshotConfig();
  cfg.mutation.useGlobalMutability = true;
  cfg.mutation.globalMutability = 0;

  const rng = new RNG(99);
  const world = new World(cfg, rng);
  world.fillGrid(CellType.EMPTY);

  const parent = createOriginOrganism(world, 60, 60);
  world.addOrganism(parent);
  for (let i = 0; i < 100; i++) {
    parent.energy = 1000;
    parent.reproduce();
  }
  eq('变异率为 0 时只有 1 个物种', world.species.list.length, 1);
}

// ────────────────────────────────────────────────────────────
section('6. 长程生态');

{
  const cfg = snapshotConfig();
  const rng = new RNG(20261015);
  const world = new World(cfg, rng);
  generateDemoWorld(world);
  seedOriginOfLife(world);
  spawnPopulation(world, 6);

  const record = new FossilRecord(cfg.stats.recordSize);
  const TICKS = 12000;
  const t0 = Date.now();
  let peakSpecies = 0;

  for (let t = 0; t < TICKS; t++) {
    world.step();
    if (world.tick % cfg.stats.interval === 0) {
      world.species.update(world, world.tick);
      world.species.prune();
      const bio = world.detailedStats();
      record.push({
        tick: world.tick,
        population: world.organisms.length,
        species: world.species.aliveCount,
        avgCells: bio.avgCells,
        avgMutability: bio.avgMutability,
        cellShares: bio.cellShares,
      });
      if (world.species.aliveCount > peakSpecies) peakSpecies = world.species.aliveCount;
    }
  }
  const elapsed = Date.now() - t0;

  const finalPop = world.organisms.length;
  const finalSpecies = world.species.aliveCount;
  const bio = world.organismStats();

  ok('12000 tick 后种群未灭绝', finalPop > 50, `${finalPop} 个生物`);
  ok('出现了多个物种（演化真的在发生）', peakSpecies > 1, `峰值 ${peakSpecies} 个物种`);
  ok('存活物种数合理', finalSpecies >= 1 && finalSpecies < 2000, `${finalSpecies} 个`);
  ok('物种记录未爆炸', world.species.list.length <= MAX_SPECIES, `${world.species.list.length} 条`);
  ok('曲线有足够多的采样点', record.length >= 100, `${record.length} 个样本`);
  ok('平均体型合理', bio.avgCells >= 1 && bio.avgCells < 60, `${bio.avgCells.toFixed(2)} 格`);
  ok('平均变异率在合法区间', bio.avgMutability >= 1 && bio.avgMutability <= 100, bio.avgMutability.toFixed(2));
  ok('单 tick 耗时仍在预算内', elapsed / TICKS < 8, `${(elapsed / TICKS).toFixed(3)} ms/tick`);

  const pops = record.extract('populations', 500);
  let minPop = Infinity;
  for (let i = 0; i < pops.n; i++) if (pops.values[i] < minPop) minPop = pops.values[i];
  ok('种群全程未归零', minPop > 0, `最低 ${minPop}`);

  console.log(`      ${TICKS} tick / ${elapsed} ms · 最终 ${finalPop} 生物 · ${finalSpecies} 存活物种 · 峰值 ${peakSpecies} 物种 · 物种记录 ${world.species.list.length}`);
  console.log(`      平均体型 ${bio.avgCells.toFixed(2)} 格 · 平均变异率 ${bio.avgMutability.toFixed(2)} · 灭绝 ${world.species.totalExtinct} 个物种`);
}

// ────────────────────────────────────────────────────────────
console.log(`\n${'='.repeat(52)}`);
if (fail === 0) console.log(`全部通过 ✓  (${pass} 项)`);
else console.log(`通过 ${pass} 项，失败 ${fail} 项 ✗`);
process.exit(fail === 0 ? 0 : 1);
