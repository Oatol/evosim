/**
 * 阶段 7 逻辑测试：细胞类型注册表 / 细胞属性与生命值 / 生物库。
 * 纯 node，无 DOM 依赖（物种工坊的界面由 cdp-phase7.mjs 覆盖）。
 */

import { Config, snapshotConfig } from '../src/core/config.js';
import { RNG } from '../src/core/RNG.js';
import { World } from '../src/core/World.js';
import {
  CellType, CELL_DEFS, CELL_DEF_BY_ID, CELL_DEF_BY_KEY, CELL_TYPE_COUNT,
  LIVING_CELL_TYPES, PAINTABLE_CELL_TYPES, INDEPENDENT_CELL_TYPES,
  VISIBLE_CELL_TYPES, CELL_FLAG_NAMES, cellDef, cellAttr, cellAttrDefaults,
  cellTypeLabel, isBodyCell, isDirectional,
} from '../src/life/CellType.js';
import { CELL_BEHAVIORS, performCellFunction, foodEnergyGain } from '../src/life/BodyCell.js';
import { Anatomy } from '../src/life/Anatomy.js';
import { Organism } from '../src/life/Organism.js';
import { Brain } from '../src/life/Brain.js';
import { Sensors, SEEN_TYPES, SEEN_TYPE_LABELS, VISUAL_SLOTS } from '../src/life/Sensors.js';
import { COLOR_SCHEMES, ColorScheme } from '../src/render/ColorScheme.js';
import { CreatureLibrary } from '../src/io/CreatureLibrary.js';
import { generateDemoWorld } from '../src/core/WorldGen.js';
import { seedOriginOfLife, spawnPopulation, spawnFromBlueprint } from '../src/life/OrganismFactory.js';

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
  ok(name, same, `实际 ${JSON.stringify(actual)}，期望 ${JSON.stringify(expected)}`);
}

function section(title) {
  console.log(`\n=== ${title} ===`);
}

function makeWorld(seed = 4242, mutate = null) {
  const cfg = snapshotConfig();
  if (mutate) mutate(cfg);
  const rng = new RNG(seed);
  const world = new World(cfg, rng);
  return { world, rng, cfg };
}

function sameFloat(a, b) {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
  return true;
}

// ────────────────────────────────────────────────────────────
section('1. 细胞类型注册表');

{
  // id 唯一、与 CellType 常量一致
  const ids = CELL_DEFS.map((d) => d.id);
  eq('id 无重复', new Set(ids).size, ids.length);
  let mismatch = 0;
  for (const [key, value] of Object.entries(CellType)) {
    const d = CELL_DEF_BY_KEY[key];
    if (!d || d.id !== value) mismatch++;
  }
  eq('每个 CellType 常量都能在注册表里找到同 id 的定义', mismatch, 0);
  eq('注册表大小与 CELL_TYPE_COUNT 一致', CELL_TYPE_COUNT, CELL_DEFS.length);

  // 索引表
  let badIndex = 0;
  for (const d of CELL_DEFS) {
    if (CELL_DEF_BY_ID[d.id] !== d) badIndex++;
    if (CELL_DEF_BY_KEY[d.key] !== d) badIndex++;
  }
  eq('CELL_DEF_BY_ID / CELL_DEF_BY_KEY 指向同一批对象', badIndex, 0);

  // 每个定义都有完整的展示字段与三套颜色
  let missingField = 0;
  let missingColor = 0;
  for (const d of CELL_DEFS) {
    if (!d.key || !d.name || !d.label) missingField++;
    for (const scheme of Object.keys(COLOR_SCHEMES)) {
      if (!d.colors || !d.colors[scheme]) missingColor++;
    }
  }
  eq('每个定义都有 key / name / label', missingField, 0);
  eq('每个定义在三套配色里都有颜色', missingColor, 0);

  // 派生列表由标记决定（"加一个定义对象就够"的契约）
  eq('LIVING = 身体细胞',
    LIVING_CELL_TYPES.join(','), CELL_DEFS.filter((d) => d.isBodyCell).map((d) => d.id).join(','));
  eq('PAINTABLE = paintable 标记',
    PAINTABLE_CELL_TYPES.join(','), CELL_DEFS.filter((d) => d.paintable).map((d) => d.id).join(','));
  eq('INDEPENDENT = independent 标记',
    INDEPENDENT_CELL_TYPES.join(','), CELL_DEFS.filter((d) => d.independent).map((d) => d.id).join(','));
  eq('VISIBLE = visible 标记',
    VISIBLE_CELL_TYPES.join(','), CELL_DEFS.filter((d) => d.visible).map((d) => d.id).join(','));

  // 每个 behavior 名都要在行为表里有实现
  let badBehavior = 0;
  for (const d of CELL_DEFS) {
    if (d.behavior && typeof CELL_BEHAVIORS[d.behavior] !== 'function') badBehavior++;
  }
  eq('每个 behavior 都能在 CELL_BEHAVIORS 里找到', badBehavior, 0);

  // 每个 flag 都要真的是 Anatomy 上的字段
  const probe = new Anatomy();
  let badFlag = 0;
  for (const d of CELL_DEFS) {
    if (d.flag && !(d.flag in probe)) badFlag++;
  }
  eq('每个能力标志位都是 Anatomy 的真实字段', badFlag, 0);
  eq('CELL_FLAG_NAMES 与定义里的 flag 一致',
    CELL_FLAG_NAMES.join(','), CELL_DEFS.filter((d) => d.flag).map((d) => d.flag).join(','));

  // 查询函数
  eq('cellTypeLabel 走注册表', cellTypeLabel(CellType.KILLER), '杀手');
  eq('未知 id 的 label 兜底', cellTypeLabel(99), '未知');
  ok('isBodyCell 对独立格返回 false', isBodyCell(CellType.WALL) === false);
  ok('isBodyCell 对身体细胞返回 true', isBodyCell(CellType.MOUTH) === true);
  ok('isDirectional 只对眼睛为真',
    isDirectional(CellType.EYE) === true && isDirectional(CellType.MOVER) === false);
}

// ────────────────────────────────────────────────────────────
section('2. 模块化的"加一种新细胞"契约');

{
  // 注册表是唯一事实来源：Anatomy 的标志位、describe、颜色、可绘制列表
  // 全都由定义对象推导。这里用现有类型验证这条链路是通的——
  // 真加新类型时，同样的链路会自动多认一个。
  const a = new Anatomy();
  a.addCell(CellType.MOUTH, 0, 0);
  a.addCell(CellType.PRODUCER, 1, 1);
  a.addCell(CellType.EYE, -1, 1, 2);
  a.recompute();

  eq('Anatomy 按 flag 自动置位（嘴）', a.hasMouth, true);
  eq('Anatomy 按 flag 自动置位（生产者）', a.hasProducer, true);
  eq('Anatomy 按 flag 自动置位（眼睛）', a.hasEye, true);
  eq('没有该类型时标志位为 false', a.hasKiller, false);
  eq('describe() 用注册表里的中文标签', a.describe(), '嘴×1 生产者×1 眼睛×1');

  const a2 = new Anatomy();
  a2.addCell(CellType.MOUTH, 0, 0);
  a2.recompute();
  ok('recompute 会把已移除类型的标志位清掉（先有眼睛）', a2.hasEye === false);

  // 带朝向的类型：dir 参与指纹，不带朝向的类型忽略 dir
  const e1 = new Anatomy(); e1.addCell(CellType.EYE, 0, 0, 1); e1.addCell(CellType.MOUTH, 0, 1);
  const e2 = new Anatomy(); e2.addCell(CellType.EYE, 0, 0, 2); e2.addCell(CellType.MOUTH, 0, 1);
  ok('眼睛朝向不同 → 指纹不同', e1.fingerprint() !== e2.fingerprint());
  const m1 = new Anatomy(); m1.addCell(CellType.MOUTH, 0, 0); m1.addCell(CellType.MOVER, 1, 0, 3);
  const m2 = new Anatomy(); m2.addCell(CellType.MOUTH, 0, 0); m2.addCell(CellType.MOVER, 1, 0, 0);
  eq('非朝向类型的 dir 不影响指纹', m1.fingerprint(), m2.fingerprint());
}

// ────────────────────────────────────────────────────────────
section('3. 配色由注册表派生');

{
  for (const name of Object.keys(COLOR_SCHEMES)) {
    const cs = new ColorScheme(name);
    let bad = 0;
    for (const d of CELL_DEFS) {
      if (cs.colorOf(d.id) !== d.colors[name]) bad++;
    }
    eq(`「${COLOR_SCHEMES[name].label}」配色与注册表一致`, bad, 0);
  }
  const cs = new ColorScheme('classic');
  eq('未知类型返回兜底色', cs.colorOf(77), '#ff00ff');
}

// ────────────────────────────────────────────────────────────
section('4. 细胞属性读取（Config.cells 优先，注册表兜底）');

{
  const { world, cfg } = makeWorld();
  eq('默认生命值来自注册表', cellAttr(cfg, CellType.MOUTH, 'hp'), 1);
  eq('默认杀手伤害 = 1', cellAttr(cfg, CellType.KILLER, 'damage'), 1);

  cfg.cells.MOUTH.hp = 7;
  eq('Config.cells 覆盖注册表默认值', cellAttr(cfg, CellType.MOUTH, 'hp'), 7);

  const noCells = { brain: cfg.brain };
  eq('没有 cells 段时回退到注册表', cellAttr(noCells, CellType.MOUTH, 'hp'), 1);

  eq('cellAttrDefaults 给出出厂默认', cellAttrDefaults(CellType.KILLER).damage, 1);

  // 参数面板的元数据覆盖所有身体细胞 × 所有属性
  eq('细胞属性表覆盖所有身体细胞', Object.keys(cfg.cells).length, LIVING_CELL_TYPES.length);
  void world;
}

// ────────────────────────────────────────────────────────────
section('5. 感知：类型顺序、槽位、每只眼睛的视野');

{
  eq('SEEN_TYPES = 注册表里 visible 的类型', SEEN_TYPES.join(','), VISIBLE_CELL_TYPES.join(','));
  eq('视觉槽标签来自注册表',
    SEEN_TYPE_LABELS.join(','), SEEN_TYPES.map((t) => cellDef(t).label).join(','));
  eq('每方向槽数 = 可见类型数 + 1（距离）', VISUAL_SLOTS, SEEN_TYPES.length + 1);

  // 每只眼睛的视野：把眼睛的 senseRange 调小，远处的目标就看不见了
  const { world, cfg } = makeWorld();
  world.fillGrid(CellType.EMPTY);

  const watcher = new Organism(world, 10, 10);
  watcher.anatomy.addCell(CellType.EYE, 0, 0, 1);   // 朝右
  watcher.rotation = 0;
  world.addOrganism(watcher);

  // 在正右方 5 格放一块食物
  world.setCell(15, 10, CellType.FOOD, -1);

  const sensors = new Sensors(40);
  // 眼睛朝向 1（右）→ 落在第 1 个方向分组里
  const base = 1 * VISUAL_SLOTS;
  const distSlot = base + VISUAL_SLOTS - 1;

  cfg.cells.EYE.senseRange = 8;
  let v = Array.from(sensors.sense(watcher));
  ok('视野够远时看得见', v[distSlot] > 0, `closeness=${v[distSlot].toFixed(3)}`);

  cfg.cells.EYE.senseRange = 3;
  v = Array.from(sensors.sense(watcher));
  eq('视野不够远时看不见（距离分量 0）', v[distSlot], 0);

  cfg.cells.EYE.senseRange = 0;                     // 0 = 用全局值
  cfg.sense.lookRange = 30;
  v = Array.from(sensors.sense(watcher));
  ok('senseRange=0 时回退到全局视野', v[distSlot] > 0,
    `closeness=${v[distSlot].toFixed(3)}`);

  // one-hot 落在正确的槽位（食物是第 0 个可见类型）
  eq('食物落在该方向分组的视觉槽 0', v[base], 1);
}

// ────────────────────────────────────────────────────────────
section('6. 生命值：个体 = 所有细胞之和');

{
  const { world, cfg } = makeWorld();
  world.fillGrid(CellType.EMPTY);

  const org = new Organism(world, 20, 20);
  org.anatomy.addCell(CellType.MOUTH, 0, 0);
  org.anatomy.addCell(CellType.MOVER, 1, 0);
  org.anatomy.addCell(CellType.ARMOR, 0, 1);
  world.addOrganism(org);

  eq('默认 maxHealth = 细胞数', org.maxHealth(), 3);

  cfg.cells.ARMOR.hp = 6;
  eq('改某类细胞的 hp 后 maxHealth 跟着变', org.maxHealth(), 1 + 1 + 6);

  org.harm(3);
  ok('伤害未达上限时不死', org.alive === true, `damage=${org.damage}/${org.maxHealth()}`);
  org.harm(5);
  ok('伤害达到上限后死亡', org.alive === false);

  // 一击必杀
  const { world: w2, cfg: c2 } = makeWorld();
  w2.fillGrid(CellType.EMPTY);
  const o2 = new Organism(w2, 5, 5);
  o2.anatomy.addCell(CellType.MOUTH, 0, 0);
  w2.addOrganism(o2);
  c2.combat.instaKill = true;
  o2.harm(1);
  ok('一击必杀模式下任何伤害都致死', o2.alive === false);
}

// ────────────────────────────────────────────────────────────
section('7. 杀手伤害可调（走完整的行为分发路径）');

{
  const { world, cfg } = makeWorld();
  world.fillGrid(CellType.EMPTY);

  const killer = new Organism(world, 10, 10);
  killer.anatomy.addCell(CellType.KILLER, 0, 0);
  world.addOrganism(killer);

  const victim = new Organism(world, 11, 10);
  victim.anatomy.addCell(CellType.MOUTH, 0, 0);
  world.addOrganism(victim);
  cfg.cells.MOUTH.hp = 3;

  const kc = killer.anatomy.cells[0];
  performCellFunction(killer, kc);
  eq('默认伤害 1：一次攻击后受害者受伤 1', victim.damage, 1);
  ok('受害者还没死', victim.alive === true);
  performCellFunction(killer, kc);
  performCellFunction(killer, kc);
  ok('累计 3 点伤害后受害者死亡', victim.alive === false);

  // 把伤害调高
  const { world: w2, cfg: c2 } = makeWorld();
  w2.fillGrid(CellType.EMPTY);
  c2.cells.KILLER.damage = 5;
  c2.cells.MOUTH.hp = 3;
  const k2 = new Organism(w2, 10, 10);
  k2.anatomy.addCell(CellType.KILLER, 0, 0);
  w2.addOrganism(k2);
  const v2 = new Organism(w2, 11, 10);
  v2.anatomy.addCell(CellType.MOUTH, 0, 0);
  w2.addOrganism(v2);
  performCellFunction(k2, k2.anatomy.cells[0]);
  ok('伤害调到 5 后一次攻击即致死', v2.alive === false);

  // 装甲仍然免疫
  const { world: w3 } = makeWorld();
  w3.fillGrid(CellType.EMPTY);
  const k3 = new Organism(w3, 10, 10);
  k3.anatomy.addCell(CellType.KILLER, 0, 0);
  w3.addOrganism(k3);
  const v3 = new Organism(w3, 11, 10);
  v3.anatomy.addCell(CellType.ARMOR, 0, 0);
  w3.addOrganism(v3);
  performCellFunction(k3, k3.anatomy.cells[0]);
  eq('装甲格免疫杀手伤害', v3.damage, 0);
}

// ────────────────────────────────────────────────────────────
section('8. 死亡食物量由 foodValue 决定');

{
  const { world, cfg } = makeWorld();
  world.fillGrid(CellType.EMPTY);

  // 默认：一格一食物
  const a = new Organism(world, 30, 30);
  a.anatomy.addCell(CellType.MOUTH, 0, 0);
  a.anatomy.addCell(CellType.MOVER, 1, 0);
  world.addOrganism(a);
  const before = world.foodCells.length;
  a.die();
  eq('默认死亡后食物数 = 细胞数', world.foodCells.length - before, 2);

  // foodValue 调高 → 多出来的部分投放到周围空格
  const { world: w2, cfg: c2 } = makeWorld();
  w2.fillGrid(CellType.EMPTY);
  c2.cells.MOUTH.foodValue = 3;
  c2.cells.MOVER.foodValue = 3;
  const b = new Organism(w2, 30, 30);
  b.anatomy.addCell(CellType.MOUTH, 0, 0);
  b.anatomy.addCell(CellType.MOVER, 1, 0);
  w2.addOrganism(b);
  b.die();
  eq('foodValue=3 的两个细胞死亡后产生 6 份食物', w2.foodCells.length, 6);

  eq('foodValueTotal 就是各细胞 foodValue 之和', b.foodValueTotal(), 6);
}

// ────────────────────────────────────────────────────────────
section('9. 代谢消耗与饿死');

{
  // scale = 0（默认）→ 消耗关闭
  const { world, cfg } = makeWorld();
  world.fillGrid(CellType.EMPTY);
  const o = new Organism(world, 40, 40);
  o.anatomy.addCell(CellType.MOUTH, 0, 0);
  world.addOrganism(o);
  o.energy = 0.5;                    // 低于 foodNeeded，避免顺手繁殖掉能量
  eq('默认代谢倍率为 0', cfg.life.metabolismScale, 0);
  eq('默认 upkeep = 0', o.upkeepCost(), 0);
  o.act();
  eq('关闭消耗时能量不变', o.energy, 0.5);

  // 打开消耗
  const { world: w2, cfg: c2 } = makeWorld();
  w2.fillGrid(CellType.EMPTY);
  c2.life.metabolismScale = 1;
  c2.cells.MOUTH.cost = 0.1;
  const o2 = new Organism(w2, 40, 40);
  o2.anatomy.addCell(CellType.MOUTH, 0, 0);
  w2.addOrganism(o2);
  eq('upkeep = 细胞消耗之和', o2.upkeepCost(), 0.1);
  o2.energy = 5;
  o2.act();
  ok('开启消耗后能量下降', o2.energy < 5, `energy=${o2.energy.toFixed(3)}`);

  // 饿死：能量见底后撑 starvationTicks 个 tick
  const { world: w3, cfg: c3 } = makeWorld();
  w3.fillGrid(CellType.EMPTY);
  c3.life.metabolismScale = 1;
  c3.life.starvationTicks = 5;
  c3.cells.MOUTH.cost = 0.5;
  const o3 = new Organism(w3, 40, 40);
  o3.anatomy.addCell(CellType.MOUTH, 0, 0);
  w3.addOrganism(o3);
  o3.energy = 0.2;
  let ticks = 0;
  while (o3.alive && ticks < 50) { o3.act(); ticks++; }
  ok('能量见底后会在 starvationTicks 内饿死', o3.alive === false, `${ticks} tick`);
  ok('饿死不是立刻发生（有缓冲）', ticks >= 5, `${ticks} tick`);

  // 能吃上东西就不会饿死
  const { world: w4, cfg: c4 } = makeWorld();
  w4.fillGrid(CellType.EMPTY);
  c4.life.metabolismScale = 1;
  c4.cells.MOUTH.cost = 0.1;
  const o4 = new Organism(w4, 40, 40);
  o4.anatomy.addCell(CellType.MOUTH, 0, 0);
  w4.addOrganism(o4);
  for (let i = 0; i < 30; i++) {
    w4.setCell(41, 40, CellType.FOOD, -1);   // 每 tick 喂一口
    o4.act();
  }
  ok('持续有食物就不会饿死', o4.alive === true, `energy=${o4.energy.toFixed(2)}`);
}

// ────────────────────────────────────────────────────────────
section('9b. 食物量倍率（一份食物给多少能量）');

{
  // 默认：一份食物 = 1 能量（与旧行为逐位一致）
  const { world, cfg } = makeWorld();
  world.fillGrid(CellType.EMPTY);
  eq('默认食物量倍率为 1', cfg.life.foodEnergyScale, 1);

  const o = new Organism(world, 40, 40);
  o.anatomy.addCell(CellType.MOUTH, 0, 0);
  world.addOrganism(o);
  world.setCell(41, 40, CellType.FOOD, -1);
  o.energy = 0;
  performCellFunction(o, o.anatomy.cells[0]);
  eq('默认吃一份食物 +1 能量', o.energy, 1);
  eq('食物被吃掉', world.typeAt(41, 40), CellType.EMPTY);

  // 倍率调高：同一口食物给更多能量
  const { world: w2, cfg: c2 } = makeWorld();
  w2.fillGrid(CellType.EMPTY);
  c2.life.foodEnergyScale = 4;
  const o2 = new Organism(w2, 40, 40);
  o2.anatomy.addCell(CellType.MOUTH, 0, 0);
  w2.addOrganism(o2);
  w2.setCell(41, 40, CellType.FOOD, -1);
  o2.energy = 0;
  performCellFunction(o2, o2.anatomy.cells[0]);
  eq('倍率 4 时吃一份食物 +4 能量', o2.energy, 4);

  // 多口食物按份数累加
  const { world: w3, cfg: c3 } = makeWorld();
  w3.fillGrid(CellType.EMPTY);
  c3.life.foodEnergyScale = 2;
  const o3 = new Organism(w3, 40, 40);
  o3.anatomy.addCell(CellType.MOUTH, 0, 0);
  w3.addOrganism(o3);
  w3.setCell(41, 40, CellType.FOOD, -1);
  w3.setCell(40, 39, CellType.FOOD, -1);
  o3.energy = 0;
  performCellFunction(o3, o3.anatomy.cells[0]);
  eq('两口食物各 ×2 → +4 能量', o3.energy, 4);

  // 倍率只影响"吃进来多少"，不影响生产者造食与死亡掉落
  const { world: w4, cfg: c4 } = makeWorld();
  w4.fillGrid(CellType.EMPTY);
  c4.life.foodEnergyScale = 5;
  const o4 = new Organism(w4, 40, 40);
  o4.anatomy.addCell(CellType.MOUTH, 0, 0);
  o4.anatomy.addCell(CellType.MOVER, 1, 0);
  w4.addOrganism(o4);
  const beforeFood = w4.foodCells.length;
  o4.die();
  eq('食物量倍率不影响死亡掉落的食物份数', w4.foodCells.length - beforeFood, 2);

  // 非法倍率兜底成 1（旧存档没有这段 / 面板写进异常值）
  const { cfg: c5 } = makeWorld();
  c5.life.foodEnergyScale = 0;
  eq('倍率为 0 时兜底成 1', foodEnergyGain(c5), 1);
  c5.life.foodEnergyScale = undefined;
  eq('倍率缺失时兜底成 1', foodEnergyGain(c5), 1);
  c5.life.foodEnergyScale = 0.5;
  eq('合法的低倍率原样生效', foodEnergyGain(c5), 0.5);
}

// ────────────────────────────────────────────────────────────
section('10. 生物库');

{
  const lib = new CreatureLibrary();
  eq('node 里 localStorage 不可用 → 库退化为内存态', lib.persistent, false);
  eq('初始为空', lib.count, 0);

  const cells = [
    { type: CellType.MOUTH, dx: 0, dy: 0, dir: 0 },
    { type: CellType.EYE, dx: 1, dy: 0, dir: 2 },
  ];
  const brain = new Brain(Config, new RNG(7));
  const entry = lib.add({ cells, brain, label: '测试生物' }, '阿米巴');
  ok('加入成功并自动编号', entry && entry.id === 1, `id=${entry && entry.id}`);
  eq('名字被保存', entry.name, '阿米巴');
  eq('库里有 1 条', lib.count, 1);
  ok('带大脑的记录被标记', entry.brain !== null);

  // 往返：细胞与权重逐位一致
  const bp = lib.blueprintOf(entry);
  eq('往返后细胞数一致', bp.cells.length, 2);
  eq('往返后眼睛朝向保留', bp.cells[1].dir, 2);
  ok('往返后大脑权重逐位一致', sameFloat(bp.brain.w1, brain.w1)
    && sameFloat(bp.brain.b2, brain.b2));
  ok('往返得到的是新的 Brain 实例', bp.brain !== brain);

  // 无大脑的记录
  const e2 = lib.add({ cells: [{ type: CellType.MOUTH, dx: 0, dy: 0, dir: 0 }] }, '无脑');
  ok('手绘记录没有大脑', lib.blueprintOf(e2).brain === null);

  // 改名 / 删除
  lib.rename(entry.id, '变形虫');
  eq('改名生效', lib.byId(entry.id).name, '变形虫');
  eq('默认名兜底', lib.add({ cells }, '').name, '生物 3');

  const n0 = lib.count;
  lib.remove(entry.id);
  eq('删除减少一条', lib.count, n0 - 1);
  ok('删除不存在的 id 返回 false', lib.remove(999) === false);

  // toJSON / fromJSON
  const json = JSON.parse(JSON.stringify(lib.toJSON()));
  eq('导出格式正确', json.format, 'evosim-creatures');
  const lib2 = new CreatureLibrary();
  lib2.fromJSON(json);
  eq('导入后条数一致', lib2.count, lib.count);
  eq('导入后名字一致', lib2.entries[0].name, lib.entries[0].name);

  // 坏数据要能报错而不是静默
  let threw = false;
  try { lib2.fromJSON({ format: 'nope' }); } catch { threw = true; }
  ok('格式不符时抛错', threw === true);

  // merge
  const added = lib2.merge(lib.toJSON());
  eq('merge 返回新增条数', added, lib.count);
  eq('merge 后条数翻倍', lib2.count, lib.count * 2);

  // 投放
  const { world } = makeWorld();
  world.fillGrid(CellType.EMPTY);
  const placed = lib.spawnInto(world, 50, 50, lib2.entries[0]);
  ok('投放成功', !!placed, placed ? placed.anatomy.describe() : '失败');

  // 投放出的个体应当和记录里的蓝图同结构（指纹一致）
  const refAnatomy = new Anatomy();
  for (const c of lib2.entries[0].cells) refAnatomy.addCell(c[0], c[1], c[2], c[3] || 0);
  refAnatomy.recompute();
  eq('投放出的个体与记录同物种指纹', placed.speciesFingerprint, refAnatomy.fingerprint());
}

// ────────────────────────────────────────────────────────────
section('11. 回归：长程生态仍然正常');

{
  const { world } = makeWorld(1234);
  generateDemoWorld(world);
  seedOriginOfLife(world);
  spawnPopulation(world, 6);
  const t0 = Date.now();
  for (let i = 0; i < 3000; i++) world.step();
  const ms = Date.now() - t0;
  ok('3000 tick 后种群未灭绝', world.organisms.length > 0, `${world.organisms.length} 个`);
  ok('单 tick 耗时仍在预算内', ms / 3000 < 8, `${(ms / 3000).toFixed(3)} ms/tick`);
}

// ────────────────────────────────────────────────────────────
console.log(`\n${'='.repeat(52)}`);
if (fail === 0) console.log(`全部通过 ✓  (${pass} 项)`);
else console.log(`通过 ${pass} 项，失败 ${fail} 项 ✗`);
process.exit(fail === 0 ? 0 : 1);
