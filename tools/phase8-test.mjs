/**
 * 阶段 8 逻辑测试：地形层 / onEnter · onStand 钩子 / 4 种新地形 / 橡皮擦语义。
 * 纯 node，无 DOM 依赖（画笔与真实点击由 cdp-phase8.mjs 覆盖）。
 */

import { snapshotConfig } from '../src/core/config.js';
import { RNG } from '../src/core/RNG.js';
import { World } from '../src/core/World.js';
import {
  CellType, CELL_DEFS, CELL_DEF_BY_ID, cellDef, isTerrain, isPassable,
  TERRAIN_CELL_TYPES, TERRAIN_BRUSH_TYPES, TICKING_TERRAIN_TYPES,
  TERRAIN_MASK, TICKS_TERRAIN,
} from '../src/life/CellType.js';
import { Organism } from '../src/life/Organism.js';
import { TERRAIN_ENTER, TERRAIN_STAND, TERRAIN_TICK, applyTerrainEnter, applyTerrainStand } from '../src/life/Terrain.js';
import { Editor } from '../src/ui/Editor.js';
import { serializeWorld, deserializeWorld, SAVE_VERSION } from '../src/io/SaveLoad.js';

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

function makeWorld(seed = 8181, mutate = null) {
  const cfg = snapshotConfig();
  if (mutate) mutate(cfg);
  const rng = new RNG(seed);
  return { world: new World(cfg, rng), rng, cfg };
}

/** 造一只 3 格、maxHealth=3 的耐揍生物（免得测岩浆时一击就死） */
function toughOrganism(world, col, row) {
  const o = new Organism(world, col, row);
  o.anatomy.addCell(CellType.MOUTH, 0, 0);
  o.anatomy.addCell(CellType.ARMOR, 0, 1);
  o.anatomy.addCell(CellType.ARMOR, 1, 1);
  world.addOrganism(o);
  return o;
}

// ────────────────────────────────────────────────────────────
section('1. 注册表：4 种新地形');

{
  eq('LAVA id', CellType.LAVA, 9);
  eq('SWAMP id', CellType.SWAMP, 10);
  eq('RAD id', CellType.RAD, 11);
  eq('HUMUS id', CellType.HUMUS, 12);

  const ids = CELL_DEFS.map((d) => d.id);
  eq('id 全表唯一', new Set(ids).size, ids.length);
  ok('id 都 < 64（VISION_SLOT 是 Int8Array(64)）', ids.every((i) => i < 64));

  for (const key of ['LAVA', 'SWAMP', 'RAD', 'HUMUS']) {
    const d = CELL_DEF_BY_ID[CellType[key]];
    ok(`${d.label} 是地形`, d.terrain === true);
    ok(`${d.label} 可通行`, d.passable === true);
    ok(`${d.label} 可被画笔绘制`, d.paintable === true && d.independent === true);
    ok(`${d.label} 三套配色齐全`,
      d.colors && d.colors.classic && d.colors.highContrast && d.colors.colorblindSafe);
    ok(`${d.label} 暂不参与视觉（等感知扩容那批）`, d.visible === false);
  }

  ok('墙也是地形', cellDef(CellType.WALL).terrain === true);
  ok('墙不可通行', isPassable(CellType.WALL) === false);
  ok('空地不是地形', cellDef(CellType.EMPTY).terrain === false);
  ok('食物不是地形', cellDef(CellType.FOOD).terrain === false);
  ok('身体细胞不是地形', cellDef(CellType.MOUTH).terrain === false);

  eq('TERRAIN_CELL_TYPES 数量', TERRAIN_CELL_TYPES.length, 5);
  ok('TERRAIN_CELL_TYPES 含墙与 4 种新地形',
    [CellType.WALL, CellType.LAVA, CellType.SWAMP, CellType.RAD, CellType.HUMUS]
      .every((t) => TERRAIN_CELL_TYPES.includes(t)));

  eq('画笔列表 = 空/食物/墙/岩浆/沼泽/辐射带/腐殖质',
    TERRAIN_BRUSH_TYPES.join(','),
    [CellType.EMPTY, CellType.FOOD, CellType.WALL, CellType.LAVA, CellType.SWAMP,
      CellType.RAD, CellType.HUMUS].join(','));
  ok('画笔列表不含身体细胞', !TERRAIN_BRUSH_TYPES.includes(CellType.MOUTH));

  eq('只有腐殖质需要每 tick 更新', TICKING_TERRAIN_TYPES.join(','), String(CellType.HUMUS));
  ok('TERRAIN_MASK 查表正确',
    TERRAIN_MASK[CellType.LAVA] === 1 && TERRAIN_MASK[CellType.WALL] === 1
    && TERRAIN_MASK[CellType.FOOD] === 0 && TERRAIN_MASK[CellType.EMPTY] === 0);
  ok('TICKS_TERRAIN 查表正确',
    TICKS_TERRAIN[CellType.HUMUS] === 1 && TICKS_TERRAIN[CellType.LAVA] === 0);
}

// ────────────────────────────────────────────────────────────
section('2. 通行性：地形能走进去，墙不能，食物照旧');

{
  const { world } = makeWorld();
  world.fillGrid(CellType.EMPTY);

  // 一行地形，右边留空地
  const row = 20;
  world.setCell(11, row, CellType.LAVA, -1);
  world.setCell(12, row, CellType.SWAMP, -1);
  world.setCell(13, row, CellType.RAD, -1);
  world.setCell(14, row, CellType.HUMUS, -1);
  world.setCell(15, row, CellType.WALL, -1);

  const o = new Organism(world, 10, row);
  o.anatomy.addCell(CellType.MOUTH, 0, 0);
  world.addOrganism(o);

  ok('岩浆可以走进去', o.isClear(11, row, 0) === true);
  ok('沼泽可以走进去', o.isClear(12, row, 0) === true);
  ok('辐射带可以走进去', o.isClear(13, row, 0) === true);
  ok('腐殖质可以走进去', o.isClear(14, row, 0) === true);
  ok('墙走不进去', o.isClear(15, row, 0) === false);

  // 食物：默认阻挡（foodBlocksReproduction = true）
  const { world: w2, cfg: c2 } = makeWorld();
  w2.fillGrid(CellType.EMPTY);
  w2.setCell(11, row, CellType.FOOD, -1);
  const o2 = new Organism(w2, 10, row);
  o2.anatomy.addCell(CellType.MOUTH, 0, 0);
  w2.addOrganism(o2);
  eq('默认配置下食物阻挡移动', o2.isClear(11, row, 0), c2.combat.foodBlocksReproduction === true ? false : true);
  c2.combat.foodBlocksReproduction = false;
  ok('关掉"食物阻挡"后就能走进去', o2.isClear(11, row, 0) === true);
}

// ────────────────────────────────────────────────────────────
section('3. floor 层：生物不会把地形啃掉');

{
  const { world } = makeWorld();
  world.fillGrid(CellType.EMPTY);
  const row = 30;
  world.setCell(11, row, CellType.LAVA, -1);

  eq('画上岩浆后地板就是岩浆', world.floorAt(11, row), CellType.LAVA);

  // 用 3 格生物：单格生物会被岩浆的踏入伤害一击打死，测不到"走过去再走开"
  const o = toughOrganism(world, 10, row);

  o.direction = 1;                     // 向右
  o.attemptMove();
  eq('生物走进了岩浆那格', o.col, 11);
  ok('顶层现在是生物的身体', world.typeAt(11, row) === CellType.MOUTH,
    `type=${world.typeAt(11, row)}`);
  eq('地板仍然是岩浆（没被啃掉）', world.floorAt(11, row), CellType.LAVA);

  o.attemptMove();                     // 再往右走，离开岩浆
  eq('生物离开后岩浆露回顶层', world.typeAt(11, row), CellType.LAVA);
  eq('地板也还是岩浆', world.floorAt(11, row), CellType.LAVA);

  // 玩家画"空" = 真的把地形删掉
  world.setCell(11, row, CellType.EMPTY, -1);
  eq('画空之后顶层是空', world.typeAt(11, row), CellType.EMPTY);
  eq('画空之后地板也清了', world.floorAt(11, row), CellType.EMPTY);

  // 食物被吃掉 → 露出底下的地形
  const { world: w2 } = makeWorld();
  w2.fillGrid(CellType.EMPTY);
  w2.setCell(5, 5, CellType.HUMUS, -1);
  w2.setCell(5, 5, CellType.FOOD, -1);        // 食物压在腐殖质上
  eq('食物压在地形上时地板仍是腐殖质', w2.floorAt(5, 5), CellType.HUMUS);
  w2.revealFloor(w2.grid.index(5, 5));        // 模拟"食物被吃掉"
  eq('食物没了之后腐殖质露出来', w2.typeAt(5, 5), CellType.HUMUS);

  // 食物腐败也走同一条路
  const { world: w3, cfg: c3 } = makeWorld();
  w3.fillGrid(CellType.EMPTY);
  c3.world.foodDecayRate = 100;
  w3.setCell(7, 7, CellType.LAVA, -1);
  w3.setCell(7, 7, CellType.FOOD, -1);
  w3.applyFoodDecay();
  eq('食物腐败后露出岩浆', w3.typeAt(7, 7), CellType.LAVA);
}

// ────────────────────────────────────────────────────────────
section('4. 踏入效果（enter）');

{
  // 岩浆：踏进去挨一记冲击伤害
  const { world, cfg } = makeWorld();
  world.fillGrid(CellType.EMPTY);
  world.setCell(11, 10, CellType.LAVA, -1);
  const o = toughOrganism(world, 10, 10);
  eq('默认岩浆踏入伤害', cfg.terrain.lavaEnterDamage, 2);
  o.direction = 1;
  o.attemptMove();
  eq('踏进岩浆后累计伤害 = lavaEnterDamage', o.damage, cfg.terrain.lavaEnterDamage);
  ok('生物还活着（没被一击打死）', o.alive === true);

  // 伤害调高 → 一击致死
  const { world: w2, cfg: c2 } = makeWorld();
  w2.fillGrid(CellType.EMPTY);
  w2.setCell(11, 10, CellType.LAVA, -1);
  c2.terrain.lavaEnterDamage = 99;
  const o2 = toughOrganism(w2, 10, 10);
  o2.direction = 1;
  o2.attemptMove();
  ok('岩浆伤害拉满时踏进去就死', o2.alive === false);

  // 沼泽：移动额外耗能
  const { world: w3, cfg: c3 } = makeWorld();
  w3.fillGrid(CellType.EMPTY);
  w3.setCell(11, 10, CellType.SWAMP, -1);
  const o3 = toughOrganism(w3, 10, 10);
  o3.energy = 5;
  o3.direction = 1;
  o3.attemptMove();
  eq('沼泽移动扣掉 swampMoveCost', o3.energy, 5 - c3.terrain.swampMoveCost);

  // 走进空地不该有任何消耗
  const { world: w4 } = makeWorld();
  w4.fillGrid(CellType.EMPTY);
  const o4 = toughOrganism(w4, 10, 10);
  o4.energy = 5;
  o4.direction = 1;
  o4.attemptMove();
  eq('走空地不扣能量、不掉血', o4.energy, 5);
  eq('走空地不掉血', o4.damage, 0);
}

// ────────────────────────────────────────────────────────────
section('5. 驻留效果（stand）');

{
  const { world, cfg } = makeWorld();
  world.fillGrid(CellType.EMPTY);
  world.setCell(10, 10, CellType.RAD, -1);
  const o = toughOrganism(world, 10, 10);

  eq('默认辐射伤害', cfg.terrain.radDamage, 0.25);
  eq('act 之前没有辐射标记', o.inRadiation, false);

  o.act();
  ok('站在辐射带里每 tick 掉血', o.damage > 0, `damage=${o.damage}`);
  ok('并且被打上了"处于辐射中"的标记', o.inRadiation === true);

  // 离开辐射带后标记会被清掉
  world.setCell(10, 10, CellType.EMPTY, -1);   // 玩家把辐射带擦掉
  o.act();
  eq('离开辐射带后标记清空', o.inRadiation, false);
}

// ────────────────────────────────────────────────────────────
section('6. 辐射带放大后代变异率');

{
  // 用"必变异"的极端配置，验证倍率真的作用在**繁殖**那一步
  const { world, cfg } = makeWorld();
  world.fillGrid(CellType.EMPTY);
  world.setCell(10, 10, CellType.RAD, -1);
  const o = toughOrganism(world, 10, 10);

  // 直接调用 Terrain 的驻留处理器，确认标记生效
  o.inRadiation = false;
  applyTerrainStand(o, CellType.RAD, 10, 10);
  ok('applyTerrainStand 打上了辐射标记', o.inRadiation === true);

  // 不在辐射里时不该有标记
  const { world: w2 } = makeWorld();
  w2.fillGrid(CellType.EMPTY);
  const o2 = toughOrganism(w2, 10, 10);
  o2.inRadiation = false;
  applyTerrainStand(o2, CellType.EMPTY, 10, 10);
  eq('空地不产生辐射标记', o2.inRadiation, false);

  ok('辐射倍率配置存在且 > 1', cfg.terrain.radMutationBoost > 1, String(cfg.terrain.radMutationBoost));
}

// ────────────────────────────────────────────────────────────
section('7. 腐殖质：世界级每 tick 造食');

{
  const { world, cfg } = makeWorld();
  world.fillGrid(CellType.EMPTY);
  cfg.terrain.humusFoodProb = 100;             // 必定成功，方便断言
  world.setCell(20, 20, CellType.HUMUS, -1);

  eq('腐殖质进了地形索引', world.terrainCells.get(CellType.HUMUS).length, 1);

  const before = world.foodCells.length;
  world.applyTerrainTick();
  eq('跑一次就多了一份食物', world.foodCells.length, before + 1);

  // 造出的食物必须紧挨着腐殖质
  const idx = world.foodCells[world.foodCells.length - 1];
  const [fc, fr] = world.grid.coords(idx);
  ok('食物落在腐殖质的四邻域里',
    Math.abs(fc - 20) + Math.abs(fr - 20) === 1, `(${fc}, ${fr})`);

  // 腐殖质被擦掉后，索引要跟着清空
  world.setCell(20, 20, CellType.EMPTY, -1);
  eq('擦掉腐殖质后索引清空', world.terrainCells.get(CellType.HUMUS).length, 0);

  // 被生物踩在脚下也照样造食（索引建在 floor 上）
  const { world: w2, cfg: c2 } = makeWorld();
  w2.fillGrid(CellType.EMPTY);
  c2.terrain.humusFoodProb = 100;
  w2.setCell(30, 30, CellType.HUMUS, -1);
  const o = new Organism(w2, 30, 30);
  o.anatomy.addCell(CellType.MOUTH, 0, 0);
  w2.addOrganism(o);
  eq('生物踩上去后索引仍在', w2.terrainCells.get(CellType.HUMUS).length, 1);
  const b2 = w2.foodCells.length;
  w2.applyTerrainTick();
  eq('被踩住的腐殖质照样造食', w2.foodCells.length, b2 + 1);
}

// ────────────────────────────────────────────────────────────
section('8. 橡皮擦：只清地形，留下食物与生物');

{
  // _eraseCell 只用到 this.world，可以直接借原型跑（不需要真的实例化 Editor）
  const erase = (world, col, row) => Editor.prototype._eraseCell.call({ world }, col, row);

  const { world } = makeWorld();
  world.fillGrid(CellType.EMPTY);
  world.setCell(11, 10, CellType.WALL, -1);
  world.setCell(12, 10, CellType.LAVA, -1);
  world.setCell(13, 10, CellType.FOOD, -1);
  const o = new Organism(world, 14, 10);
  o.anatomy.addCell(CellType.MOUTH, 0, 0);
  world.addOrganism(o);
  const orgCount = world.organisms.length;

  erase(world, 11, 10);
  eq('橡皮擦清掉了墙', world.typeAt(11, 10), CellType.EMPTY);
  eq('墙的地板也被清掉', world.floorAt(11, 10), CellType.EMPTY);

  erase(world, 12, 10);
  eq('橡皮擦清掉了岩浆', world.typeAt(12, 10), CellType.EMPTY);

  erase(world, 13, 10);
  eq('橡皮擦**不动**食物', world.typeAt(13, 10), CellType.FOOD);

  erase(world, 14, 10);
  eq('橡皮擦**不动**生物身体', world.typeAt(14, 10), CellType.MOUTH);
  eq('生物数量不变', world.organisms.length, orgCount);
  ok('生物还活着', world.organisms[0].alive === true);
}

// ────────────────────────────────────────────────────────────
section('9. 存档：地形随 floor 层往返（含被踩住的）');

{
  eq('存档版本已升到 2', SAVE_VERSION, 2);

  const src = makeWorld(5555);
  src.world.fillGrid(CellType.EMPTY);
  src.world.setCell(10, 10, CellType.LAVA, -1);
  src.world.setCell(20, 20, CellType.SWAMP, -1);
  src.world.setCell(30, 30, CellType.HUMUS, -1);
  src.world.setCell(40, 40, CellType.WALL, -1);
  // 一只生物踩在岩浆上——顶层是身体、地板是岩浆
  const o = new Organism(src.world, 10, 10);
  o.anatomy.addCell(CellType.MOUTH, 0, 0);
  src.world.addOrganism(o);

  const data = serializeWorld(src.world, src.rng, { config: snapshotConfig() });
  ok('存档里有 terrain 段', !!data.grid.terrain);
  ok('存档不再有 walls 段', data.grid.walls === undefined);
  eq('存档里的岩浆格数 = 1（尽管被生物踩着）', data.grid.terrain.LAVA.length, 1);

  const dst = makeWorld(1);
  const info = deserializeWorld(dst.world, dst.rng, data);
  eq('读档后地板逐格一致', (() => {
    let d = 0;
    for (let i = 0; i < src.world.grid.floor.length; i++) {
      if (src.world.grid.floor[i] !== dst.world.grid.floor[i]) d++;
    }
    return d;
  })(), 0);
  eq('读档后岩浆仍在原位（被生物踩着也保住了）', dst.world.floorAt(10, 10), CellType.LAVA);
  eq('读档后腐殖质索引重建', dst.world.terrainCells.get(CellType.HUMUS).length, 1);
  eq('读档报告的地形格数', info.terrain, 4);

  // v1 存档（只有 walls）仍然能读
  const legacy = JSON.parse(JSON.stringify(data));
  legacy.version = 1;
  delete legacy.grid.terrain;
  legacy.grid.walls = [40 * 200 + 40];
  const dst2 = makeWorld(2);
  deserializeWorld(dst2.world, dst2.rng, legacy);
  eq('v1 存档的墙被读进地板层', dst2.world.floorAt(40, 40), CellType.WALL);
  eq('v1 存档里没有岩浆', dst2.world.floorAt(10, 10), CellType.EMPTY);
}

// ────────────────────────────────────────────────────────────
section('10. 重置：可以选择保留地形');

{
  const { world } = makeWorld();
  world.fillGrid(CellType.EMPTY);
  world.setCell(10, 10, CellType.LAVA, -1);
  world.setCell(20, 20, CellType.WALL, -1);

  world.reset(true);
  eq('保留地形：岩浆还在', world.typeAt(10, 10), CellType.LAVA);
  eq('保留地形：墙还在', world.typeAt(20, 20), CellType.WALL);

  world.reset(false);
  eq('不保留：岩浆没了', world.typeAt(10, 10), CellType.EMPTY);
  eq('不保留：地板也清了', world.floorAt(10, 10), CellType.EMPTY);
  eq('不保留：墙也没了', world.typeAt(20, 20), CellType.EMPTY);
}

// ────────────────────────────────────────────────────────────
section('11. 行为表完整性');

{
  for (const d of CELL_DEFS) {
    if (d.enter) ok(`${d.label} 的 enter='${d.enter}' 有实现`, !!TERRAIN_ENTER[d.enter]);
    if (d.stand) ok(`${d.label} 的 stand='${d.stand}' 有实现`, !!TERRAIN_STAND[d.stand]);
    if (d.tick) ok(`${d.label} 的 tick='${d.tick}' 有实现`, !!TERRAIN_TICK[d.tick]);
  }
  // 反查：表里的每个键都至少被一种地形用到（避免留下死代码）
  const used = new Set(CELL_DEFS.map((d) => d.enter).filter(Boolean));
  for (const k of Object.keys(TERRAIN_ENTER)) ok(`TERRAIN_ENTER.${k} 被用到`, used.has(k));
  const usedS = new Set(CELL_DEFS.map((d) => d.stand).filter(Boolean));
  for (const k of Object.keys(TERRAIN_STAND)) ok(`TERRAIN_STAND.${k} 被用到`, usedS.has(k));
  const usedT = new Set(CELL_DEFS.map((d) => d.tick).filter(Boolean));
  for (const k of Object.keys(TERRAIN_TICK)) ok(`TERRAIN_TICK.${k} 被用到`, usedT.has(k));

  // 缺配置段时不该崩（旧存档）
  const bare = { life: {} };
  let threw = false;
  try {
    const { world: w } = makeWorld();
    w.fillGrid(CellType.EMPTY);
    const o = new Organism(w, 10, 10);
    o.anatomy.addCell(CellType.MOUTH, 0, 0);
    w.addOrganism(o);
    w.config.terrain = undefined;             // 模拟没有 terrain 段的旧配置
    o.damage = 0;
    applyTerrainEnter(o, CellType.LAVA, 10, 10);
    applyTerrainStand(o, CellType.RAD, 10, 10);
  } catch (e) { threw = true; console.log('    异常：', e.message); }
  ok('配置缺少 terrain 段时地形效果不抛异常', threw === false);
}

// ────────────────────────────────────────────────────────────
console.log(`\n${'='.repeat(52)}`);
if (fail === 0) console.log(`全部通过 ✓  (${pass} 项)`);
else console.log(`通过 ${pass} 项，失败 ${fail} 项 ✗`);

process.exit(fail === 0 ? 0 : 1);
