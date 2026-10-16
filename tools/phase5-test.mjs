/**
 * 阶段 5 逻辑测试：参数面板元数据 / 存档读档往返 / 存档后继续演化可复现 / 克隆蓝图。
 *
 * 纯 node，无 DOM 依赖。参数面板的实际交互、存档下载与文件读取由 CDP 脚本覆盖。
 */

import { Config, CONFIG_SECTIONS, CONFIG_FIELDS, DEFAULT_CONFIG, snapshotConfig, restoreConfig } from '../src/core/config.js';
import { RNG } from '../src/core/RNG.js';
import { World } from '../src/core/World.js';
import { CellType } from '../src/life/CellType.js';
import { Organism } from '../src/life/Organism.js';
import { generateDemoWorld } from '../src/core/WorldGen.js';
import { seedOriginOfLife, spawnPopulation, makeBlueprint, spawnFromBlueprint, createOriginOrganism } from '../src/life/OrganismFactory.js';
import {
  serializeWorld, deserializeWorld, validateSave,
  SAVE_FORMAT, SAVE_VERSION, saveFilename,
} from '../src/io/SaveLoad.js';

let pass = 0;
let fail = 0;

function ok(name, cond, extra = '') {
  if (cond) { pass++; console.log(`PASS  ${name}${extra ? '  → ' + extra : ''}`); }
  else { fail++; console.log(`FAIL  ${name}${extra ? '  → ' + extra : ''}`); }
}

function eq(name, actual, expected) {
  const same = typeof actual === 'number' && typeof expected === 'number'
    ? Math.abs(actual - expected) < 1e-4
    : actual === expected;
  ok(name, same, `实际 ${actual}，期望 ${expected}`);
}

function section(title) {
  console.log(`\n=== ${title} ===`);
}

function makeWorld(seed = 12345) {
  const cfg = snapshotConfig();
  const rng = new RNG(seed);
  return { world: new World(cfg, rng), rng, cfg };
}

/** 造一个跑起来的小生态 */
function makePopulatedWorld(seed = 4242, ticks = 0) {
  const ctx = makeWorld(seed);
  const { world } = ctx;
  generateDemoWorld(world);
  seedOriginOfLife(world);
  spawnPopulation(world, 8);
  for (let i = 0; i < ticks; i++) world.step();
  return ctx;
}

const snapshot = (world) => ({
  tick: world.tick,
  types: Array.from(world.grid.types),
  owners: Array.from(world.grid.owners),
  organisms: world.organisms.map((o) => ({
    col: o.col, row: o.row, rot: o.rotation, dir: o.direction,
    energy: o.energy, age: o.age, mut: o.mutability,
    fp: o.speciesFingerprint,
    cells: o.anatomy.cells.map((c) => `${c.type}:${c.dx},${c.dy},${c.dir}`).sort().join('|'),
    w1: Array.from(o.brain.w1),
    b1: Array.from(o.brain.b1),
    w2: Array.from(o.brain.w2),
    b2: Array.from(o.brain.b2),
  })),
});

function sameFloatArrays(a, b) {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
  return true;
}

/** 两个等长数组的逐位差异个数；长度不等返回 -1。避免失败时打印整张网格 */
function diffCount(a, b) {
  if (a.length !== b.length) return -1;
  let n = 0;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) n++;
  return n;
}

/** 字段键支持一层点号路径（细胞属性表是嵌套的，如 MOUTH.hp） */
function readPath(obj, key) {
  const dot = key.indexOf('.');
  return dot < 0 ? obj[key] : obj[key.slice(0, dot)][key.slice(dot + 1)];
}

// ────────────────────────────────────────────────────────────
section('1. 参数面板元数据（CONFIG_FIELDS / CONFIG_SECTIONS）');

{
  let missing = 0;
  let orphan = 0;
  for (const sec of CONFIG_SECTIONS) {
    if (!CONFIG_FIELDS[sec.key]) missing++;
    if (!Config[sec.key]) missing++;
  }
  for (const key of Object.keys(CONFIG_FIELDS)) {
    if (!CONFIG_SECTIONS.some((s) => s.key === key)) orphan++;
  }
  eq('每个分组都有字段表与对应配置', missing, 0);
  eq('字段表没有游离分组', orphan, 0);

  // 每个字段都要能在 Config 里找到真实的值
  let dangling = 0;
  let badRange = 0;
  let outOfRange = 0;
  let fieldCount = 0;
  for (const [sectionKey, fields] of Object.entries(CONFIG_FIELDS)) {
    for (const [key, meta] of Object.entries(fields)) {
      fieldCount++;
      const value = readPath(Config[sectionKey], key);
      if (value === undefined) { dangling++; continue; }
      if (meta.type === 'bool') continue;
      if (meta.readonly) continue;
      if (!(meta.min < meta.max) || !(meta.step > 0)) { badRange++; continue; }
      if (typeof value === 'number' && (value < meta.min || value > meta.max)) outOfRange++;
    }
  }
  ok('字段总数合理（> 35）', fieldCount > 35, `${fieldCount} 项`);
  eq('没有指向不存在配置项的字段', dangling, 0);
  eq('所有滑块的取值范围合法', badRange, 0);
  eq('所有默认值都在滑块范围内', outOfRange, 0);

  // bool 字段必须真的是布尔
  let badBool = 0;
  for (const [sectionKey, fields] of Object.entries(CONFIG_FIELDS)) {
    for (const [key, meta] of Object.entries(fields)) {
      if (meta.type === 'bool' && typeof readPath(Config[sectionKey], key) !== 'boolean') badBool++;
    }
  }
  eq('所有开关字段默认值都是布尔', badBool, 0);
}

{
  // DEFAULT_CONFIG 是深拷贝：改 Config 不应影响它
  const before = DEFAULT_CONFIG.world.cols;
  Config.world.cols = 999;
  eq('DEFAULT_CONFIG 与 Config 相互独立', DEFAULT_CONFIG.world.cols, before);
  Config.world.cols = before;

  // restoreConfig 能整体恢复
  const snap = snapshotConfig();
  Config.life.foodProdProb = 77;
  Config.sense.lookRange = 3;
  restoreConfig(snap);
  eq('restoreConfig 恢复整数值', Config.life.foodProdProb, snap.life.foodProdProb);
  eq('restoreConfig 恢复另一分组', Config.sense.lookRange, snap.sense.lookRange);
}

{
  // 延迟生效标记：网格尺寸走"重建"路径，种子走"下次重置生效"路径
  const deferred = [];
  const resize = [];
  for (const [sectionKey, fields] of Object.entries(CONFIG_FIELDS)) {
    for (const [key, meta] of Object.entries(fields)) {
      if (meta.deferred) deferred.push(`${sectionKey}.${key}`);
      if (meta.deferred === 'resize') resize.push(`${sectionKey}.${key}`);
    }
  }
  ok('只有网格尺寸走"重建世界"路径', resize.length === 2
    && resize.includes('world.cols') && resize.includes('world.rows'), resize.join(', '));
  ok('延迟生效参数集合稳定', deferred.length === 3
    && deferred.includes('world.cols') && deferred.includes('world.rows') && deferred.includes('sim.seed'),
    deferred.join(', '));
}

// ────────────────────────────────────────────────────────────
section('2. 存档序列化结构');

{
  const { world, rng } = makePopulatedWorld(1001, 60);
  const data = serializeWorld(world, rng, { config: snapshotConfig() });

  eq('format 正确', data.format, SAVE_FORMAT);
  eq('version 正确', data.version, SAVE_VERSION);
  eq('tick 被记录', data.tick, world.tick);
  ok('savedAt 是 ISO 时间戳', typeof data.savedAt === 'string' && !Number.isNaN(Date.parse(data.savedAt)), data.savedAt);
  ok('网格尺寸被记录', data.grid.cols === world.grid.cols && data.grid.rows === world.grid.rows,
    `${data.grid.cols}×${data.grid.rows}`);
  ok('个体列表非空', data.organisms.length > 0, `${data.organisms.length} 个`);
  ok('物种表非空', data.species.list.length > 0, `${data.species.list.length} 条`);
  ok('RNG 状态被记录', data.rng && typeof data.rng.a === 'number');
  ok('配置快照被记录', data.config && data.config.world && data.config.life);

  // 地形只存非空格
  let nonEmpty = 0;
  const types = world.grid.types;
  for (let i = 0; i < types.length; i++) if (types[i] !== CellType.EMPTY) nonEmpty++;
  const terrainKeys = Object.keys(data.grid.terrain || {});
  let storedTerrain = data.grid.food.length;
  for (const k of terrainKeys) storedTerrain += data.grid.terrain[k].length;
  ok('地形只存非空格（不含生物身体）', storedTerrain <= nonEmpty,
    `存 ${storedTerrain} / 非空 ${nonEmpty}`);

  // 扁平索引必须落在网格范围内
  let badIndex = 0;
  for (const k of terrainKeys) {
    for (const i of data.grid.terrain[k]) if (i < 0 || i >= types.length) badIndex++;
  }
  for (const i of data.grid.food) if (i < 0 || i >= types.length) badIndex++;
  eq('地形索引都在网格范围内', badIndex, 0);

  // 权重是 base64 字符串
  const b = data.organisms[0].brain;
  ok('大脑权重编码为 base64 字符串',
    typeof b.w1 === 'string' && typeof b.b1 === 'string' && typeof b.w2 === 'string' && typeof b.b2 === 'string');
  eq('w1 编码长度 = 480×4 字节的 base64', b.w1.length, Math.ceil(480 * 4 / 3) * 4);

  // 只有眼睛带 dir。
  // 用一个**专门装了眼睛**的小世界来验，而不是指望演化出来的种群里有眼睛——
  // 眼睛在演化里一直很稀有，加/删细胞类型都会改变变异候选池、进而改变演化轨迹，
  // 把断言绑在"60 tick 内自然长出眼睛"上会随机翻车。
  const eyeCtx = makeWorld(777);
  eyeCtx.world.fillGrid(CellType.EMPTY);
  const eyeOrg = new Organism(eyeCtx.world, 20, 20);
  eyeOrg.anatomy.addCell(CellType.MOUTH, 0, 0);
  eyeOrg.anatomy.addCell(CellType.EYE, 0, -1, 2);
  eyeCtx.world.addOrganism(eyeOrg);
  const eyeData = serializeWorld(eyeCtx.world, eyeCtx.rng, { config: snapshotConfig() });

  let eyeWithDir = 0;
  let nonEyeWithDir = 0;
  for (const org of eyeData.organisms) {
    for (const c of org.cells) {
      if (c[0] === CellType.EYE) { if (c.length === 4) eyeWithDir++; }
      else if (c.length !== 3) nonEyeWithDir++;
    }
  }
  ok('眼睛细胞带朝向', eyeWithDir > 0, `${eyeWithDir} 个`);
  eq('非眼睛细胞不带朝向', nonEyeWithDir, 0);
}

// ────────────────────────────────────────────────────────────
section('3. 存档往返一致性');

{
  const src = makePopulatedWorld(2002, 120);
  const data = serializeWorld(src.world, src.rng, { config: snapshotConfig() });
  const before = snapshot(src.world);

  // 读到一个全新世界
  const dst = makeWorld(99999);
  const info = deserializeWorld(dst.world, dst.rng, data);
  const after = snapshot(dst.world);

  eq('生物数量一致', after.organisms.length, before.organisms.length);
  eq('tick 一致', after.tick, before.tick);
  eq('地形类型逐格一致（差异格数）', diffCount(after.types, before.types), 0);
  eq('地形归属逐格一致（差异格数）', diffCount(after.owners, before.owners), 0);
  // 食物索引的**顺序**必须原样保留——腐败抽样依赖数组位置，顺序错了演化会分叉
  eq('食物索引顺序原样保留',
    diffCount(Array.from(dst.world.foodCells), Array.from(src.world.foodCells)), 0);
  eq('食物反向索引表一致',
    diffCount(Array.from(dst.world.foodPos), Array.from(src.world.foodPos)), 0);
  ok('读档报告没有大脑被跳过', info.brainSkipped === 0, `${info.brainSkipped} 个`);
  eq('物种表条数一致', dst.world.species.list.length, src.world.species.list.length);
  eq('物种 nextId 一致', dst.world.species.nextId, src.world.species.nextId);

  let mismatch = 0;
  let weightMismatch = 0;
  for (let i = 0; i < before.organisms.length; i++) {
    const a = before.organisms[i];
    const b = after.organisms[i];
    if (a.col !== b.col || a.row !== b.row || a.rot !== b.rot || a.dir !== b.dir
      || a.age !== b.age || a.fp !== b.fp || a.cells !== b.cells
      || Math.abs(a.energy - b.energy) > 1e-4 || Math.abs(a.mut - b.mut) > 1e-4) {
      mismatch++;
    }
    if (!sameFloatArrays(a.w1, b.w1) || !sameFloatArrays(a.b1, b.b1)
      || !sameFloatArrays(a.w2, b.w2) || !sameFloatArrays(a.b2, b.b2)) {
      weightMismatch++;
    }
  }
  eq('每个个体的位置/朝向/年龄/能量/变异率/蓝图一致', mismatch, 0);
  eq('每个个体的大脑权重逐位一致', weightMismatch, 0);

  // 祖先链也要原样保留
  const srcAlive = src.world.species.list.filter((s) => s.ancestorId >= 0).length;
  const dstAlive = dst.world.species.list.filter((s) => s.ancestorId >= 0).length;
  eq('带祖先关系的物种数量一致', dstAlive, srcAlive);
}

// ────────────────────────────────────────────────────────────
section('4. 读档后继续演化可复现（RNG 状态恢复）');

{
  const a = makePopulatedWorld(3003, 150);
  const data = serializeWorld(a.world, a.rng, { config: snapshotConfig() });

  const b = makeWorld(1);
  deserializeWorld(b.world, b.rng, data);

  // 两边各跑同样多的 tick
  const STEPS = 250;
  for (let i = 0; i < STEPS; i++) a.world.step();
  for (let i = 0; i < STEPS; i++) b.world.step();

  const sa = snapshot(a.world);
  const sb = snapshot(b.world);

  eq('继续演化后 tick 仍一致', sb.tick, sa.tick);
  eq('继续演化后生物数量一致', sb.organisms.length, sa.organisms.length);
  eq('继续演化后网格类型逐格一致（差异格数）', diffCount(sb.types, sa.types), 0);

  let posMismatch = 0;
  let weightMismatch = 0;
  for (let i = 0; i < Math.min(sa.organisms.length, sb.organisms.length); i++) {
    const x = sa.organisms[i];
    const y = sb.organisms[i];
    if (x.col !== y.col || x.row !== y.row || x.age !== y.age) posMismatch++;
    if (!sameFloatArrays(x.w1, y.w1) || !sameFloatArrays(x.b2, y.b2)) weightMismatch++;
  }
  eq('继续演化后每个个体位置一致', posMismatch, 0);
  eq('继续演化后大脑权重仍一致', weightMismatch, 0);
}

// ────────────────────────────────────────────────────────────
section('5. 存档校验与容错');

{
  const bad = [
    ['null', null],
    ['非对象', 42],
    ['format 不符', { format: 'other', version: 1, grid: {}, organisms: [] }],
    ['缺网格', { format: SAVE_FORMAT, version: 1, organisms: [], species: { list: [] } }],
    ['缺个体列表', { format: SAVE_FORMAT, version: 1, grid: { cols: 10, rows: 10 }, species: { list: [] } }],
    ['缺物种表', { format: SAVE_FORMAT, version: 1, grid: { cols: 10, rows: 10 }, organisms: [] }],
  ];
  let caught = 0;
  for (const [, data] of bad) {
    try { validateSave(data); } catch { caught++; }
  }
  eq('六种坏存档都被拒绝', caught, bad.length);

  // 版本过新
  let versionRejected = false;
  try {
    validateSave({ format: SAVE_FORMAT, version: SAVE_VERSION + 1, grid: { cols: 10, rows: 10 }, organisms: [], species: { list: [] } });
  } catch { versionRejected = true; }
  ok('高版本存档被拒绝', versionRejected);

  // 合法存档通过
  let okPass = true;
  try {
    validateSave({ format: SAVE_FORMAT, version: SAVE_VERSION, grid: { cols: 10, rows: 10 }, organisms: [], species: { list: [] } });
  } catch { okPass = false; }
  ok('合法存档通过校验', okPass);
}

{
  // 大脑拓扑不符时应被跳过而不是硬塞乱码
  const src = makePopulatedWorld(4004, 30);
  const data = serializeWorld(src.world, src.rng, { config: snapshotConfig() });
  data.organisms[0].brain.hiddenSize = 99;    // 假装来自改过隐藏层尺寸的会话

  const dst = makeWorld(7);
  const info = deserializeWorld(dst.world, dst.rng, data);
  eq('拓扑不符的大脑被跳过计数', info.brainSkipped, 1);
  eq('其余个体大脑正常恢复', dst.world.organisms.length - info.brainSkipped, src.world.organisms.length - 1);
  // 被跳过的那个个体的 w1 长度仍应是新的合法长度（构造函数随机出来的），不是乱码
  eq('被跳过个体的权重仍是合法长度', dst.world.organisms[0].brain.w1.length, 480);
}

{
  // 存档文件名含 tick 与时间戳
  const { world } = makePopulatedWorld(5005, 0);
  const name = saveFilename(world, new Date(2026, 9, 15, 9, 5, 3));
  ok('存档文件名含 tick', name.includes(`tick${world.tick}`), name);
  ok('存档文件名含时间戳', name.includes('20261015-090503'), name);
  ok('存档文件名以 .json 结尾', name.endsWith('.json'));
}

// ────────────────────────────────────────────────────────────
section('6. 克隆蓝图');

{
  const { world } = makeWorld(6006);
  const org = createOriginOrganism(world, 60, 60);
  org.anatomy.addCell(CellType.MOVER, 2, 0);
  org.anatomy.recompute();
  world.addOrganism(org);

  const bp = makeBlueprint(org);
  eq('蓝图细胞数与源一致', bp.cells.length, org.anatomy.cells.length);
  eq('蓝图指纹与源一致', bp.fingerprint, org.speciesFingerprint);
  ok('蓝图大脑就是源大脑引用', bp.brain === org.brain);
  ok('蓝图有可读标签', typeof bp.label === 'string' && bp.label.length > 0, bp.label);

  // 蓝图是细胞副本：改源细胞列表不应影响蓝图
  const lenBefore = bp.cells.length;
  org.anatomy.addCell(CellType.KILLER, -2, 0);
  eq('蓝图细胞是副本（源加细胞不影响它）', bp.cells.length, lenBefore);

  // 投放克隆体：应同物种、位置合法
  const clone = spawnFromBlueprint(world, 100, 100, bp);
  ok('克隆体被成功投放', !!clone);
  eq('克隆体与源同物种（指纹一致）', clone.speciesFingerprint, org.speciesFingerprint);
  eq('克隆体细胞数与蓝图一致', clone.anatomy.cells.length, bp.cells.length);

  // 克隆体大脑是拷贝：权重相等但不是同一个对象
  let weightsEqual = true;
  for (let i = 0; i < clone.brain.w1.length; i++) {
    if (clone.brain.w1[i] !== org.brain.w1[i]) { weightsEqual = false; break; }
  }
  ok('克隆体大脑权重与源相等', weightsEqual);
  ok('克隆体大脑不是同一对象（copyFrom 拷贝）', clone.brain !== org.brain);

  // 位置被占时投放失败
  const blocked = spawnFromBlueprint(world, clone.col, clone.row, bp);
  ok('位置被占时投放返回 null', blocked === null);
}

// ────────────────────────────────────────────────────────────
console.log(`\n${'='.repeat(52)}`);
console.log(fail === 0 ? `全部通过 ✓  (${pass} 项)` : `失败 ${fail} 项 ✗  (通过 ${pass} 项)`);
process.exit(fail === 0 ? 0 : 1);
