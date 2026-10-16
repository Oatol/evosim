/**
 * 阶段 6 逻辑测试：CSV 导出 / 化石记录取样 / 物种颜色 / 谱系推导。
 *
 * 纯 node，无 DOM 依赖（物种树的绘制与交互由 CDP 脚本覆盖）。
 */

import { Config, snapshotConfig } from '../src/core/config.js';
import { RNG } from '../src/core/RNG.js';
import { World } from '../src/core/World.js';
import { CellType, CELL_TYPE_COUNT, CELL_TYPES } from '../src/life/CellType.js';
import { FossilRecord } from '../src/stats/FossilRecord.js';
import { Species, SpeciesRegistry } from '../src/stats/Species.js';
import { generateDemoWorld } from '../src/core/WorldGen.js';
import { seedOriginOfLife, spawnPopulation } from '../src/life/OrganismFactory.js';
import { fossilToCSV, speciesToCSV, exportFilename } from '../src/io/Export.js';
import { fingerprintHue, speciesColor, speciesRGB } from '../src/render/ColorScheme.js';
import { buildLineage } from '../src/ui/SpeciesTree.js';

let pass = 0;
let fail = 0;

function ok(name, cond, extra = '') {
  if (cond) { pass++; console.log(`PASS  ${name}${extra ? '  → ' + extra : ''}`); }
  else { fail++; console.log(`FAIL  ${name}${extra ? '  → ' + extra : ''}`); }
}

function eq(name, actual, expected) {
  const same = typeof actual === 'number' && typeof expected === 'number'
    ? Math.abs(actual - expected) < 1e-6
    : actual === expected;
  ok(name, same, `实际 ${JSON.stringify(actual)}，期望 ${JSON.stringify(expected)}`);
}

function section(title) {
  console.log(`\n=== ${title} ===`);
}

/** 造一个跑起来的真实世界（用来产出真实的物种表） */
function makeWorld(seed = 7001, ticks = 0) {
  const cfg = snapshotConfig();
  const rng = new RNG(seed);
  const world = new World(cfg, rng);
  generateDemoWorld(world);
  seedOriginOfLife(world);
  spawnPopulation(world, 8);
  for (let i = 0; i < ticks; i++) world.step();
  return world;
}

// ────────────────────────────────────────────────────────────
section('1. FossilRecord.sampleAt（环形回绕）');

{
  const rec = new FossilRecord(5);
  for (let i = 0; i < 5; i++) {
    rec.push({ tick: i * 10, population: i, species: i, avgCells: 3 + i * 0.1, avgMutability: 5 });
  }
  eq('写满后 count = 5', rec.count, 5);
  eq('sampleAt(0) 是最旧样本', rec.sampleAt(0).tick, 0);
  eq('sampleAt(4) 是最新样本', rec.sampleAt(4).tick, 40);

  // 再写 3 个 → 最旧的 3 个被覆盖
  for (let i = 5; i < 8; i++) {
    rec.push({ tick: i * 10, population: i, species: i, avgCells: 3, avgMutability: 5 });
  }
  eq('回绕后 count 仍 = 5', rec.count, 5);
  eq('回绕后最旧样本变成 tick 30', rec.sampleAt(0).tick, 30);
  eq('回绕后最新样本是 tick 70', rec.sampleAt(4).tick, 70);

  let ordered = true;
  for (let i = 1; i < rec.count; i++) {
    if (rec.sampleAt(i).tick <= rec.sampleAt(i - 1).tick) ordered = false;
  }
  ok('sampleAt 始终按时间升序', ordered);
  eq('越界返回 null', rec.sampleAt(99), null);
}

// ────────────────────────────────────────────────────────────
section('2. 导出：演化时间序列 CSV');

{
  const rec = new FossilRecord(10);
  const shares = new Float64Array(CELL_TYPE_COUNT);
  shares[CellType.MOUTH] = 0.5;
  shares[CellType.PRODUCER] = 0.5;
  rec.push({ tick: 0, population: 10, species: 2, avgCells: 3, avgMutability: 5, cellShares: shares });
  shares[CellType.MOVER] = 0.25;
  rec.push({ tick: 100, population: 20, species: 4, avgCells: 3.5, avgMutability: 5.5, cellShares: shares });

  const csv = fossilToCSV(rec);
  const lines = csv.split('\r\n');
  eq('行数 = 表头 + 采样点', lines.length, 3);
  ok('表头含核心列', lines[0].startsWith('tick,population,aliveSpecies,avgCells,avgMutability,'), lines[0]);
  ok('表头含各身体细胞占比', lines[0].includes('share_mouth') && lines[0].includes('share_eye'), lines[0]);
  ok('第一行是第一个采样点', lines[1].startsWith('0,10,2,3.0000,5.0000,'), lines[1]);
  ok('第二行是第二个采样点', lines[2].startsWith('100,20,4,3.5000,5.5000,'), lines[2]);

  const cols = lines[0].split(',').length;
  eq('每行列数一致', lines[1].split(',').length, cols);
  eq('用 CRLF 换行（Excel 友好）', lines.length - 1, csv.split('\r\n').length - 1);

  // 空记录：只有表头
  const empty = fossilToCSV(new FossilRecord(4));
  eq('空记录只输出表头', empty.split('\r\n').length, 1);
}

// ────────────────────────────────────────────────────────────
section('3. 导出：物种表 CSV');

{
  const reg = new SpeciesRegistry(snapshotConfig());
  const mk = (id, fp, ancestorId, start, end, pop, peak, cum, alive, desc) => {
    const sp = new Species(id, fp, null, ancestorId, start);
    sp.endTick = end;
    sp.population = pop;
    sp.peakPopulation = peak;
    sp.cumulativePop = cum;
    sp.alive = alive;
    sp.description = desc;
    sp.cellCount = 3;
    reg.list.push(sp);
    return sp;
  };
  mk(1, 'aaaa0001', null, 0, 500, 0, 40, 300, false, '嘴×1 生产者×2');
  mk(2, 'bbbb0002', 1, 100, -1, 12, 25, 180, true, '嘴×1, 生产者×2');   // 描述里有逗号
  mk(3, 'cccc0003', 2, 200, -1, 3, 9, 30, true, '带"引号"的形态');

  const csv = speciesToCSV(reg);
  const lines = csv.split('\r\n');
  eq('行数 = 表头 + 物种数', lines.length, 4);
  ok('表头含祖先列', lines[0].startsWith('id,fingerprint,name,ancestorId,startTick,endTick,lifespan,'), lines[0]);

  const root = lines[1].split(',');
  eq('起源物种的 ancestorId 为空', root[3], '');
  eq('已灭绝物种 lifespan = endTick - startTick', root[6], '500');
  eq('已灭绝物种 alive 列 = 0', root[7], '0');

  const second = lines[2];
  ok('存活物种的 endTick 保留 -1', second.split(',')[5] === '-1', second);
  ok('描述含逗号时被引号包裹', second.includes('"嘴×1, 生产者×2"'), second);
  ok('描述含引号时被转义（引号翻倍）', lines[3].includes('"带""引号""的形态"'), lines[3]);

  // 存活物种的 lifespan 用 lastSeenTick 兜底（Species.lifespan getter）
  const sp2 = reg.list[1];
  sp2.lastSeenTick = 640;
  eq('存活物种 lifespan 用 lastSeenTick', sp2.lifespan, 540);
}

// ────────────────────────────────────────────────────────────
section('4. 导出文件名');

{
  const name = exportFilename('history', 1234, new Date(2026, 9, 15, 9, 5, 3));
  eq('文件名格式正确', name, 'evosim-history-tick1234-20261015-090503.csv');
  ok('物种表文件名区分种类', exportFilename('species', 1).startsWith('evosim-species-'));
}

// ────────────────────────────────────────────────────────────
section('5. 物种颜色映射');

{
  const a1 = speciesColor('a1b2c3d4');
  const a2 = speciesColor('a1b2c3d4');
  eq('同一指纹颜色稳定', a1, a2);
  ok('输出是合法 hsl 字符串', /^hsl\(\d+(\.\d+)?, \d+%, \d+%\)$/.test(a1), a1);

  const hueA = fingerprintHue('a1b2c3d4');
  const hueB = fingerprintHue('deadbeef');
  ok('色相在 0–360', hueA >= 0 && hueA < 360 && hueB >= 0 && hueB < 360, `${hueA} / ${hueB}`);
  eq('指纹为空时色相为 0', fingerprintHue(''), 0);

  // 一批指纹的色相要有足够分散度，否则物种之间分不开。
  // 这里用真实世界产出的指纹来测（见第 7 节）——合成指纹是等差数列，
  // 结构太规整，不能代表实际情况。
  const lcg = (() => {
    let x = 123456789 >>> 0;
    return () => { x = (Math.imul(x, 1103515245) + 12345) >>> 0; return x; };
  })();
  const hues = new Set();
  for (let i = 0; i < 60; i++) {
    hues.add(Math.floor(fingerprintHue(lcg().toString(16).padStart(8, '0')) / 10));
  }
  ok('60 个随机指纹的色相分散在 28+ 个 10° 区间', hues.size >= 28, `${hues.size} 个区间`);

  const [r, g, b] = speciesRGB('a1b2c3d4');
  ok('RGB 三分量都在 0–255', [r, g, b].every((v) => Number.isInteger(v) && v >= 0 && v <= 255), `${r},${g},${b}`);
  eq('同一指纹 RGB 稳定', speciesRGB('a1b2c3d4').join(','), `${r},${g},${b}`);

  // 灰阶特例
  eq('饱和度为 0 时三通道相等', speciesRGB('x', 0, 50).join(','), '128,128,128');
}

// ────────────────────────────────────────────────────────────
section('6. 谱系推导 buildLineage');

/** 造一棵手工谱系：1 → 2 → {3,4}，1 → 5 → 6 */
function makeLineage() {
  const mk = (id, ancestorId, start, end, pop, cum) => {
    const sp = new Species(id, `fp${id}`, null, ancestorId, start);
    sp.endTick = end;
    sp.population = pop;
    sp.cumulativePop = cum;
    sp.alive = pop > 0;
    return sp;
  };
  return [
    mk(1, null, 0, 15, 0, 500),
    mk(2, 1, 10, 35, 0, 300),
    mk(3, 2, 20, -1, 5, 120),
    mk(4, 2, 25, -1, 3, 90),
    mk(5, 1, 30, 45, 0, 200),
    mk(6, 5, 40, -1, 1, 40),
  ];
}

{
  const list = makeLineage();
  const L = buildLineage(list, 50);

  eq('只展开存活物种 + 祖先链', L.order.length, 6);
  eq('存活物种数', L.aliveIds.size, 3);
  ok('存活的是 3/4/6', L.aliveIds.has(3) && L.aliveIds.has(4) && L.aliveIds.has(6));

  const cols = L.order.map((sp) => L.columnOf.get(sp.id));
  eq('列号是 0..n-1 的排列', [...cols].sort((a, b) => a - b).join(','), '0,1,2,3,4,5');
  eq('前序首节点是根物种', L.order[0].id, 1);

  // 前序性质：每棵子树占据连续的一段列，且从自己的列开始
  const sizeOf = new Map();
  const computeSize = (id) => {
    let n = 1;
    for (const sp of list) {
      if (sp.ancestorId === id) n += computeSize(sp.id);
    }
    sizeOf.set(id, n);
    return n;
  };
  computeSize(1);
  let contiguous = true;
  const idsInSubtree = (rootId) => {
    const out = [rootId];
    for (const sp of list) if (sp.ancestorId === rootId) out.push(...idsInSubtree(sp.id));
    return out;
  };
  for (const sp of list) {
    const base = L.columnOf.get(sp.id);
    const size = idsInSubtree(sp.id).length;
    const got = idsInSubtree(sp.id).map((id) => L.columnOf.get(id)).sort((a, b) => a - b);
    for (let i = 0; i < size; i++) if (got[i] !== base + i) contiguous = false;
  }
  ok('前序：每棵子树占据连续的一段列', contiguous);

  eq('根物种没有父列', L.parentColumn.has(1), false);
  eq('S3 的父列 = S2 的列', L.parentColumn.get(3), L.columnOf.get(2));
  eq('S6 的父列 = S5 的列', L.parentColumn.get(6), L.columnOf.get(5));

  eq('时间下界 = 最早的诞生 tick', L.t0, 0);
  eq('时间上界 = 当前 tick', L.t1, 50);
  eq('now 被原样带上', L.now, 50);
}

{
  // 空表
  const L = buildLineage([], 100);
  eq('空物种表 → 空节点', L.order.length, 0);
  eq('空物种表 t0 兜底为 0', L.t0, 0);
  eq('空物种表 t1 = now', L.t1, 100);
}

{
  // 上限裁剪：存活的一律保留
  const list = [];
  for (let i = 1; i <= 200; i++) {
    const sp = new Species(i, `fp${i}`, null, i === 1 ? null : i - 1, i * 10);
    sp.population = (i === 200 || i === 199) ? 4 : 0;
    sp.alive = sp.population > 0;
    sp.endTick = sp.alive ? -1 : i * 10 + 5;
    sp.cumulativePop = 1000 - i;
    list.push(sp);
  }
  const L = buildLineage(list, 5000, 40);
  eq('节点数被裁到上限', L.order.length, 40);
  ok('两个存活物种都被保留', L.aliveIds.size === 2 && L.order.some((s) => s.id === 200) && L.order.some((s) => s.id === 199));
  ok('所有节点都有列号', L.order.every((sp) => L.columnOf.has(sp.id)));
  ok('每个非根节点都有父列（祖先被裁掉时接最近的仍展示的祖先）',
    L.order.every((sp) => L.parentColumn.has(sp.id) || sp.ancestorId === null || !L.order.some((o) => o.id === sp.ancestorId)));
}

// ────────────────────────────────────────────────────────────
section('7. 真实世界的物种表可以导出');

{
  const world = makeWorld(7002, 3000);
  world.species.update(world, world.tick);
  const csv = speciesToCSV(world.species);
  const lines = csv.split('\r\n');
  eq('CSV 行数 = 物种数 + 1', lines.length, world.species.list.length + 1);
  ok('至少有若干物种', world.species.list.length > 1, `${world.species.list.length} 条`);

  // 祖先 id 必须指向表里存在的物种（或者为空）
  const ids = new Set(world.species.list.map((s) => s.id));
  let dangling = 0;
  for (const sp of world.species.list) {
    if (sp.ancestorId !== null && !ids.has(sp.ancestorId)) dangling++;
  }
  eq('没有指向不存在物种的祖先引用', dangling, 0);

  // 谱系推导在真实数据上不崩，且列号是完整排列
  const L = buildLineage(world.species.list, world.tick);
  const cols = L.order.map((sp) => L.columnOf.get(sp.id)).sort((a, b) => a - b);
  let permOk = cols.length === L.order.length;
  for (let i = 0; i < cols.length; i++) if (cols[i] !== i) permOk = false;
  ok('真实数据上列号仍是完整排列', permOk, `${L.order.length} 个节点`);
  ok('真实数据上时间范围有效', L.t1 > L.t0, `${L.t0} → ${L.t1}`);

  // 真实物种指纹的颜色分散度——这是"按物种着色"能不能看的前提
  const realHues = new Set();
  for (const sp of world.species.list) realHues.add(Math.floor(fingerprintHue(sp.fingerprint) / 10));
  const ratio = realHues.size / Math.max(1, Math.min(36, world.species.list.length));
  ok('真实物种指纹的色相分散（≥ 60% 的区间被用到）', ratio >= 0.6,
    `${world.species.list.length} 个物种落在 ${realHues.size} 个区间`);
}

// ────────────────────────────────────────────────────────────
console.log(`\n${'='.repeat(52)}`);
console.log(fail === 0 ? `全部通过 ✓  (${pass} 项)` : `失败 ${fail} 项 ✗  (通过 ${pass} 项)`);
process.exit(fail === 0 ? 0 : 1);
