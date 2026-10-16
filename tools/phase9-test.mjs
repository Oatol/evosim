/**
 * 阶段 9 逻辑测试：储食 / 再生 / 刺 三种新身体细胞。
 *
 * 纯 node，无 DOM 依赖（工坊调色板与参数面板由 cdp-phase9.mjs 覆盖）。
 */

import { snapshotConfig } from '../src/core/config.js';
import { RNG } from '../src/core/RNG.js';
import { World } from '../src/core/World.js';
import {
  CellType, CELL_DEFS, CELL_DEF_BY_ID, cellDef, cellAttr,
  LIVING_CELL_TYPES, VISIBLE_CELL_TYPES, CELL_FLAG_NAMES,
} from '../src/life/CellType.js';
import { Organism } from '../src/life/Organism.js';
import { CELL_BEHAVIORS, performCellFunction } from '../src/life/BodyCell.js';
import { Sensors, SENSE_DIM } from '../src/life/Sensors.js';

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

function makeWorld(seed = 9191, mutate = null) {
  const cfg = snapshotConfig();
  if (mutate) mutate(cfg);
  const rng = new RNG(seed);
  return { world: new World(cfg, rng), rng, cfg };
}

/** 造一只 N 格生物，居中放在空地上 */
function makeOrg(world, types, col = 60, row = 60) {
  const o = new Organism(world, col, row);
  types.forEach((t, i) => {
    const dx = i === 0 ? 0 : i;
    const dy = 0;
    o.anatomy.addCell(t, dx, dy);
  });
  world.addOrganism(o);
  return o;
}

// ────────────────────────────────────────────────────────────
section('1. 注册表：三种新身体细胞');

{
  eq('STORAGE id', CellType.STORAGE, 13);
  eq('REGEN id', CellType.REGEN, 14);
  eq('SPIKE id', CellType.SPIKE, 15);

  const ids = CELL_DEFS.map((d) => d.id);
  eq('id 全表唯一', new Set(ids).size, ids.length);
  ok('id 都 < 64（VISION_SLOT 是 Int8Array(64)）', ids.every((i) => i < 64));

  for (const key of ['STORAGE', 'REGEN', 'SPIKE']) {
    const d = CELL_DEF_BY_ID[CellType[key]];
    ok(`${d.label} 是身体细胞`, d.isBodyCell === true);
    ok(`${d.label} 可在工坊里画`, d.paintable === true);
    ok(`${d.label} 不是地形`, d.terrain === false);
  }

  // 三种新细胞都不参与视觉 —— 这是"不动 VISUAL_SLOTS、不作废旧存档"的前提
  for (const key of ['STORAGE', 'REGEN', 'SPIKE']) {
    ok(`${CELL_DEF_BY_ID[CellType[key]].label} 不参与视觉（存档不因它断代）`,
      CELL_DEF_BY_ID[CellType[key]].visible === false);
  }
  eq('视觉类型仍是 8 种', VISIBLE_CELL_TYPES.length, 8);
  eq('感知维度仍是 40', SENSE_DIM, 40);

  eq('身体细胞从 6 种变成 9 种', LIVING_CELL_TYPES.length, 9);

  // 能力标志位
  eq('储食的标志位', CELL_DEF_BY_ID[CellType.STORAGE].flag, 'hasStorage');
  eq('再生的标志位', CELL_DEF_BY_ID[CellType.REGEN].flag, 'hasRegen');
  eq('刺的标志位', CELL_DEF_BY_ID[CellType.SPIKE].flag, 'hasSpike');
  ok('三个新标志位都进了 CELL_FLAG_NAMES',
    ['hasStorage', 'hasRegen', 'hasSpike'].every((f) => CELL_FLAG_NAMES.includes(f)));

  // 只有再生挂行为函数（储食/刺是纯结构性的）
  eq('再生挂了行为函数', CELL_DEF_BY_ID[CellType.REGEN].behavior, 'regen');
  eq('行为表里有 regen', typeof CELL_BEHAVIORS.regen, 'function');
  eq('储食没有行为函数', CELL_DEF_BY_ID[CellType.STORAGE].behavior, null);
  eq('刺没有行为函数', CELL_DEF_BY_ID[CellType.SPIKE].behavior, null);

  // 属性默认值
  eq('储食的死亡食物值是 3（脂肪更肥）', CELL_DEF_BY_ID[CellType.STORAGE].foodValue, 3);
  eq('刺的伤害默认 1', CELL_DEF_BY_ID[CellType.SPIKE].damage, 1);

  // 属性表自动覆盖
  const { cfg } = makeWorld();
  eq('细胞属性表覆盖 9 种身体细胞', Object.keys(cfg.cells).length, 9);
  ok('属性表里有 STORAGE 行', !!cfg.cells.STORAGE);
  ok('属性表里有 REGEN 行', !!cfg.cells.REGEN);
  ok('属性表里有 SPIKE 行', !!cfg.cells.SPIKE);
}

// ────────────────────────────────────────────────────────────
section('2. 储食：抬高繁殖阈值，多出来的变成储备');

{
  const { world, cfg } = makeWorld();
  world.fillGrid(CellType.EMPTY);

  // 无储食：阈值 == 基础成本（逐位相同）
  const plain = makeOrg(world, [CellType.MOUTH, CellType.MOVER, CellType.KILLER]);
  eq('无储食时 foodNeeded = 3', plain.foodNeeded(), 3);
  eq('无储食时阈值 == 基础成本', plain.reproduceAt(), 3);
  eq('无储食时储备为 0', plain.reproduceAt() - plain.foodNeeded(), 0);
  eq('无储食时 storageCount = 0', plain.storageCount(), 0);

  // 有 1 个储食（默认 bonus = 1）→ 阈值翻倍
  const fat = makeOrg(world, [CellType.MOUTH, CellType.MOVER, CellType.STORAGE]);
  eq('储食个数', fat.storageCount(), 1);
  eq('有 1 储食时基础成本仍是 3', fat.foodNeeded(), 3);
  eq('阈值被抬到 3 × (1 + 1×1) = 6', fat.reproduceAt(), 6);

  // bonus = 0 时退回基线
  cfg.life.storageFoodBonus = 0;
  eq('储备倍率为 0 时阈值回到基础成本', fat.reproduceAt(), 3);

  // bonus = 0.5、2 个储食 → 3 × (1 + 2×0.5) = 6
  cfg.life.storageFoodBonus = 0.5;
  const fat2 = makeOrg(world, [CellType.MOUTH, CellType.STORAGE, CellType.STORAGE]);
  eq('2 储食 × 0.5 倍 → 阈值 6', fat2.reproduceAt(), 6);
  eq('它的基础成本是 3', fat2.foodNeeded(), 3);
}

// ────────────────────────────────────────────────────────────
section('3. 储食：繁殖时只扣基础成本，差额留作储备');

{
  const { world } = makeWorld();
  world.fillGrid(CellType.EMPTY);

  const fat = makeOrg(world, [CellType.MOUTH, CellType.MOVER, CellType.STORAGE]);
  fat.energy = 6;                    // 刚好到阈值
  eq('到阈值了', fat.energy >= fat.reproduceAt(), true);

  const before = world.organisms.length;
  fat.act();
  // act() 里会先走一遍细胞功能（嘴吃空气），再繁殖
  ok('触发了繁殖（世界多了一个个体）', world.organisms.length > before,
    `${before} → ${world.organisms.length}`);
  eq('繁殖后能量 = 6 - 基础成本 3 = 3（差额是储备）', fat.energy, 3);

  // 对照：无储食者到阈值 3 就繁殖，扣完是 0
  const plain = makeOrg(world, [CellType.MOUTH, CellType.MOVER, CellType.KILLER], 100, 60);
  plain.energy = 3;
  plain.act();
  eq('无储食者繁殖后能量归 0', plain.energy, 0);

  // 储备的实际意义：繁殖之后储食个体还留着能量，无储食个体从 0 开始挨饿
  const { world: w2, cfg: c2 } = makeWorld();
  w2.fillGrid(CellType.EMPTY);
  c2.life.starvationTicks = 10;
  c2.cells.MOUTH.cost = 0.5;
  c2.cells.MOVER.cost = 0.5;

  // 先关掉消耗，让两只都在"刚好到阈值"时繁殖（否则代谢会先把能量扣到阈值以下）
  c2.life.metabolismScale = 0;
  const a = makeOrg(w2, [CellType.MOUTH, CellType.MOVER, CellType.STORAGE], 60, 60);
  a.energy = a.reproduceAt();        // 6 → 繁殖后剩 3
  a.act();
  const b = makeOrg(w2, [CellType.MOUTH, CellType.MOVER, CellType.KILLER], 120, 60);
  b.energy = b.reproduceAt();        // 3 → 繁殖后归 0
  b.act();

  eq('繁殖后储食个体还留着 3 点储备', a.energy, 3);
  eq('繁殖后无储食个体从 0 开始挨饿', b.energy, 0);

  // 现在开消耗，看谁撑得久
  c2.life.metabolismScale = 1;
  let aTicks = 0; while (a.alive && aTicks < 300) { a.act(); aTicks++; }
  let bTicks = 0; while (b.alive && bTicks < 300) { b.act(); bTicks++; }
  ok('同样开消耗，储食个体多活几 tick', aTicks > bTicks,
    `储食 ${aTicks} tick vs 无储食 ${bTicks} tick`);
}

// ────────────────────────────────────────────────────────────
section('4. 再生：每 tick 抹掉累积伤害');

{
  const { world, cfg } = makeWorld();
  world.fillGrid(CellType.EMPTY);
  cfg.life.regenRate = 0.5;

  const o = makeOrg(world, [CellType.MOUTH, CellType.REGEN, CellType.REGEN]);
  ok('有再生标志位', o.anatomy.hasRegen === true);
  eq('maxHealth = 3', o.maxHealth(), 3);

  o.damage = 2;
  o.act();
  // 2 个再生细胞 × 0.5 = 1 → 2 - 1 = 1
  eq('2 个再生细胞回 1 点（2 → 1）', o.damage, 1);

  o.act();
  eq('再走一 tick 伤害归 0', o.damage, 0);

  // 已经没伤了就不再往下扣（不会变成负伤害 = 加血）
  o.act();
  eq('无伤时不会变成负伤害', o.damage, 0);

  // 对照：没有再生细胞的生物不会自己回血
  const plain = makeOrg(world, [CellType.MOUTH, CellType.MOVER, CellType.KILLER], 100, 60);
  plain.damage = 2;
  plain.act();
  eq('无再生细胞时不回血', plain.damage, 2);

  // 再生救得回被杀手磨的血，但救不回岩浆
  cfg.life.regenRate = 0.1;
  const tough = makeOrg(world, [CellType.MOUTH, CellType.REGEN, CellType.REGEN, CellType.REGEN], 140, 60);
  tough.damage = 1;
  tough.act();
  // 3 个再生细胞 × 0.1 = 0.3 → 1 - 0.3 = 0.7
  eq('3 个再生细胞每 tick 只回 0.3（回不过累积伤害）', tough.damage, 0.7);
}

// ────────────────────────────────────────────────────────────
section('5. 刺：被杀手攻击时反伤');

{
  const { world, cfg } = makeWorld();
  world.fillGrid(CellType.EMPTY);
  cfg.combat.instaKill = false;

  // 杀手在 60,60，刺生物紧挨着在 61,60
  const k = new Organism(world, 60, 60);
  k.anatomy.addCell(CellType.KILLER, 0, 0);
  world.addOrganism(k);
  const s = new Organism(world, 61, 60);
  s.anatomy.addCell(CellType.SPIKE, 0, 0);
  world.addOrganism(s);

  ok('刺生物有反伤能力', s.anatomy.hasSpike === true);
  eq('1 个刺细胞的反伤值是 1', s.spikeDamage(), 1);
  eq('杀手没有反伤能力', k.spikeDamage(), 0);

  k.act();
  eq('杀手对刺生物造成了 1 点伤害', s.damage, 1);
  eq('杀手被反伤了 1 点', k.damage, 1);

  // 两只都带刺 → 不会无限互相反伤
  world.clearOrganisms();
  world.fillGrid(CellType.EMPTY);
  const x = new Organism(world, 60, 60);
  x.anatomy.addCell(CellType.SPIKE, 0, 0);
  x.anatomy.addCell(CellType.KILLER, 1, 0);   // 同一只身上既有刺又有杀手
  world.addOrganism(x);
  const y = new Organism(world, 62, 60);
  y.anatomy.addCell(CellType.SPIKE, 0, 0);
  y.anatomy.addCell(CellType.MOUTH, 1, 0);
  world.addOrganism(y);

  // x 的杀手在 (61,60)，y 的刺在 (62,60) —— 相邻
  x.act();
  eq('互相有刺时各自只挨一下（无连锁）', x.damage, 1);
  eq('被攻击方也挨了一下', y.damage, 1);

  // 地形伤害不触发反伤
  const { world: w2, cfg: c2 } = makeWorld();
  w2.fillGrid(CellType.EMPTY);
  c2.terrain.lavaEnterDamage = 5;
  const lone = new Organism(w2, 60, 60);
  lone.anatomy.addCell(CellType.SPIKE, 0, 0);
  lone.anatomy.addCell(CellType.MOUTH, 1, 0);
  lone.anatomy.addCell(CellType.MOUTH, 0, 1);
  w2.addOrganism(lone);
  // 把右边一格地板设成岩浆
  w2.setCellAt(w2.grid.index(61, 60), CellType.LAVA, -1);
  lone.direction = 1;
  const before2 = lone.damage;
  lone.attemptMove();
  ok('走进岩浆会挨烫', lone.damage > before2, `damage=${lone.damage}`);
  eq('但岩浆不会被"反伤"（没有攻击者）', lone.damage, before2 + 5);

  // 装甲挡不住刺（刺穿甲）
  world.clearOrganisms();
  world.fillGrid(CellType.EMPTY);
  const kArm = new Organism(world, 60, 60);
  kArm.anatomy.addCell(CellType.KILLER, 0, 0);
  kArm.anatomy.addCell(CellType.ARMOR, 0, 1);
  world.addOrganism(kArm);
  const sArm = new Organism(world, 61, 60);
  sArm.anatomy.addCell(CellType.SPIKE, 0, 0);
  world.addOrganism(sArm);
  kArm.act();
  eq('带装甲的攻击者照样被刺到', kArm.damage, 1);
}

// ────────────────────────────────────────────────────────────
section('6. 反伤不会连锁（递归截断）');

{
  const { world } = makeWorld();
  world.fillGrid(CellType.EMPTY);

  // A 有杀手，B 有刺，C 也有刺 —— 确认只发生一层
  const a = new Organism(world, 60, 60);
  a.anatomy.addCell(CellType.KILLER, 0, 0);
  a.anatomy.addCell(CellType.SPIKE, 0, 1);   // A 自己也带刺
  world.addOrganism(a);
  const b = new Organism(world, 61, 60);
  b.anatomy.addCell(CellType.SPIKE, 0, 0);
  b.anatomy.addCell(CellType.SPIKE, 1, 0);
  world.addOrganism(b);

  a.act();
  // A 打 B：B 挨 1（1 个杀手细胞），A 挨 B 的反伤 2（2 个刺细胞）
  eq('B 挨了 1 点', b.damage, 1);
  eq('A 挨了 B 的反伤 2 点（不再连锁回 B）', a.damage, 2);
}

// ────────────────────────────────────────────────────────────
section('7. 基线不受影响：无新细胞时逐位一致');

{
  // 无储食 → reproduceAt 与 foodNeeded 完全相同
  const { world } = makeWorld();
  world.fillGrid(CellType.EMPTY);
  const o = makeOrg(world, [CellType.MOUTH, CellType.PRODUCER, CellType.MOVER, CellType.KILLER]);
  eq('阈值 == 成本', o.reproduceAt(), o.foodNeeded());
  eq('spikeDamage 为 0', o.spikeDamage(), 0);
  eq('hasRegen 为 false', o.anatomy.hasRegen, false);
  eq('hasStorage 为 false', o.anatomy.hasStorage, false);

  // 感知向量长度没变
  const sensors = new Sensors(SENSE_DIM);
  const v = sensors.sense(o);
  eq('输入向量长度仍是 40', v.length, 40);
}

// ────────────────────────────────────────────────────────────
section('8. 死亡掉落：储食细胞让尸体更肥');

{
  const { world } = makeWorld();
  world.fillGrid(CellType.EMPTY);
  const o = makeOrg(world, [CellType.MOUTH, CellType.STORAGE, CellType.STORAGE], 60, 60);
  eq('foodValueTotal = 1 + 3 + 3 = 7', o.foodValueTotal(), 7);
  o.die();
  eq('3 格尸体 + 4 份额外食物 = 7', world.foodCells.length, 7);
}

console.log(`\n────────────────────────────\n通过 ${pass} 项，失败 ${fail} 项`);
process.exit(fail === 0 ? 0 : 1);
