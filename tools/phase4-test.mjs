/**
 * 阶段 4 逻辑测试：神经网络 / 感知编码 / 大脑遗传 / 行为驱动 / 长程生态。
 * 纯 node，无 DOM 依赖（Inspector 的渲染部分由 CDP 脚本覆盖）。
 */

import { Config, snapshotConfig } from '../src/core/config.js';
import { RNG } from '../src/core/RNG.js';
import { World } from '../src/core/World.js';
import { CellType } from '../src/life/CellType.js';
import { Anatomy } from '../src/life/Anatomy.js';
import { Organism } from '../src/life/Organism.js';
import { Brain } from '../src/life/Brain.js';
import { Sensors, SENSE_DIM, VISUAL_SLOTS, STATE_BASE, INPUT_LABELS } from '../src/life/Sensors.js';
import { mutateBrain, mutateOrganism } from '../src/life/Mutation.js';
import { generateDemoWorld } from '../src/core/WorldGen.js';
import { seedOriginOfLife, spawnPopulation, createOriginOrganism } from '../src/life/OrganismFactory.js';

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

/** 造一个只有嘴和眼睛的观察者，位置固定在 (50,50) */
function makeWatcher(world, { eyeDir = 1, extra = [] } = {}) {
  const org = new Organism(world, 50, 50);
  org.anatomy.addCell(CellType.MOUTH, 0, 0);
  org.anatomy.addCell(CellType.EYE, 1, 0, eyeDir);
  for (const [type, dx, dy] of extra) org.anatomy.addCell(type, dx, dy);
  org.rotation = 0;
  world.addOrganism(org);
  return org;
}

// ────────────────────────────────────────────────────────────
section('1. Brain 结构与初始化');

{
  const { cfg } = makeWorld();
  const brain = new Brain(cfg, null);

  eq('输入维度 = 40', brain.inSize, 40);
  eq('隐藏维度 = 12', brain.hiddenSize, 12);
  eq('输出维度 = 5', brain.outSize, 5);
  eq('w1 长度 = 40×12', brain.w1.length, 480);
  eq('b1 长度 = 12', brain.b1.length, 12);
  eq('w2 长度 = 12×5', brain.w2.length, 60);
  eq('b2 长度 = 5', brain.b2.length, 5);
  eq('权重总数 = 557', brain.weightCount, 557);
}

{
  // 同种子 → 同初始权重；不同种子 → 不同
  const { cfg } = makeWorld();
  const a = new Brain(cfg, new RNG(7));
  const b = new Brain(cfg, new RNG(7));
  const c = new Brain(cfg, new RNG(8));

  let same = true;
  for (let i = 0; i < a.w1.length; i++) if (a.w1[i] !== b.w1[i]) same = false;
  ok('同种子初始化的权重完全一致', same);

  let diff = false;
  for (let i = 0; i < a.w1.length; i++) if (a.w1[i] !== c.w1[i]) diff = true;
  ok('不同种子初始化的权重不同', diff);

  // 权重不该全是 0——否则所有个体行为一致
  let nonzero = 0;
  for (let i = 0; i < a.w1.length; i++) if (a.w1[i] !== 0) nonzero++;
  ok('w1 初始化不是全 0', nonzero > 400, `${nonzero} / 480 非零`);

  let b2nonzero = 0;
  for (let i = 0; i < a.b2.length; i++) if (a.b2[i] !== 0) b2nonzero++;
  ok('b2 有随机初值（无眼睛生物也有个体差异）', b2nonzero === 5, `${b2nonzero} / 5`);
}

// ────────────────────────────────────────────────────────────
section('2. 前向传播数学');

{
  const { cfg } = makeWorld();
  const brain = new Brain(cfg, null);
  brain.w1.fill(0); brain.b1.fill(0); brain.w2.fill(0); brain.b2.fill(0);

  // h[0] = tanh(in[0])，o[0] = 2·h[0]
  brain.w1[0 * 40 + 0] = 1;
  brain.w2[0 * 5 + 0] = 2;

  const inputs = new Float32Array(SENSE_DIM);
  inputs[0] = 0.5;
  const out = brain.forward(inputs);
  eq('隐藏层走 tanh', brain.hiddenActivations[0], Math.tanh(0.5));
  eq('输出 = 2·tanh(0.5)', out[0], 2 * Math.tanh(0.5));

  // 输出层是线性的：把 w2 放大 10 倍，输出也放大 10 倍（隐藏层已被 tanh 压住不变）
  inputs[0] = 100;
  brain.forward(inputs);
  const h = brain.hiddenActivations[0];
  eq('饱和输入下隐藏层 ≈ 1', h, 1);

  brain.w2[0 * 5 + 0] = 20;
  const out2 = brain.forward(inputs);
  eq('输出层线性（不饱和）', out2[0], 20 * Math.tanh(100));
  ok('输出层确实没有激活函数', out2[0] > 19, String(out2[0]));
}

{
  // 偏置生效
  const { cfg } = makeWorld();
  const brain = new Brain(cfg, null);
  brain.w1.fill(0); brain.b1.fill(0); brain.w2.fill(0); brain.b2.fill(0);
  brain.b1[3] = 1;            // h[3] = tanh(1)
  brain.w2[3 * 5 + 2] = 1;    // o[2] = h[3]
  brain.b2[4] = 0.5;

  const inputs = new Float32Array(SENSE_DIM);
  const out = brain.forward(inputs);
  eq('隐藏层偏置生效', brain.hiddenActivations[3], Math.tanh(1));
  eq('输出 = tanh(1)', out[2], Math.tanh(1));
  eq('输出层偏置生效', out[4], 0.5);
}

{
  // 零输入 + 零权重 → 全零输出
  const { cfg } = makeWorld();
  const brain = new Brain(cfg, null);
  brain.w1.fill(0); brain.b1.fill(0); brain.w2.fill(0); brain.b2.fill(0);
  const out = brain.forward(new Float32Array(SENSE_DIM));
  ok('零权重零输入 → 全零输出', out.every((v) => v === 0));
}

{
  // 复用缓冲：连续调用返回同一个数组，不分配
  const { cfg } = makeWorld();
  const brain = new Brain(cfg, new RNG(3));
  const inputs = new Float32Array(SENSE_DIM);
  const o1 = brain.forward(inputs);
  const o2 = brain.forward(inputs);
  ok('forward 复用输出缓冲', o1 === o2);
  ok('forward 记录最后一次输入（供检视器）', brain.lastInput === inputs);
}

// ────────────────────────────────────────────────────────────
section('3. 权重复制与变异');

{
  const { cfg } = makeWorld();
  const a = new Brain(cfg, new RNG(11));
  const b = new Brain(cfg, new RNG(22));
  const before = Array.from(b.w1);

  ok('copyFrom 返回 true', a.copyFrom(b) === true);
  let same = true;
  for (let i = 0; i < a.w1.length; i++) if (a.w1[i] !== b.w1[i]) same = false;
  ok('copyFrom 后权重一致', same);

  let changed = false;
  for (let i = 0; i < b.w1.length; i++) if (b.w1[i] !== before[i]) changed = true;
  ok('copyFrom 不修改源大脑', !changed);
}

{
  // 拓扑不一致时拒绝复制，而不是按错误步长复制出乱码
  const cfg = snapshotConfig();
  const a = new Brain(cfg, new RNG(1));
  const cfg2 = snapshotConfig();
  cfg2.brain.hiddenSize = 8;
  const b = new Brain(cfg2, new RNG(1));
  ok('拓扑不一致时 copyFrom 返回 false', a.copyFrom(b) === false);
}

{
  const { cfg } = makeWorld();
  const brain = new Brain(cfg, new RNG(5));
  const before = Array.from(brain.w1);

  const mut = { ...cfg.mutation, weightMutRate: 0 };
  mutateBrain(brain, new RNG(9), mut);
  let same = true;
  for (let i = 0; i < brain.w1.length; i++) if (brain.w1[i] !== before[i]) same = false;
  ok('weightMutRate=0 时权重不变', same);

  const mut2 = { ...cfg.mutation, weightMutRate: 1, weightMutSigma: 0.5, weightResetRate: 0 };
  const brain2 = new Brain(cfg, new RNG(5));
  const before2 = Array.from(brain2.w1);
  mutateBrain(brain2, new RNG(9), mut2);
  let diffCount = 0;
  for (let i = 0; i < brain2.w1.length; i++) if (brain2.w1[i] !== before2[i]) diffCount++;
  ok('weightMutRate=1 时权重几乎全部改变', diffCount > 470, `${diffCount} / 480`);

  // 扰动幅度应大致符合 sigma
  let sumSq = 0;
  for (let i = 0; i < brain2.w1.length; i++) {
    const d = brain2.w1[i] - before2[i];
    sumSq += d * d;
  }
  const rms = Math.sqrt(sumSq / brain2.w1.length);
  ok('扰动幅度与 sigma 同量级', rms > 0.25 && rms < 0.75, `rms=${rms.toFixed(3)}`);
}

// ────────────────────────────────────────────────────────────
section('4. 感知编码');

{
  // 没有眼睛 → 视觉槽全 0，状态槽照常
  const { world } = makeWorld();
  world.fillGrid(CellType.EMPTY);

  const org = createOriginOrganism(world, 50, 50);
  org.energy = 3;
  org.age = 50;
  world.addOrganism(org);

  const inputs = org.sensors.sense(org);
  let visionSum = 0;
  for (let i = 0; i < STATE_BASE; i++) visionSum += inputs[i];
  eq('无眼睛时视觉槽全 0', visionSum, 0);

  eq('能量比例槽', inputs[STATE_BASE], org.energyRatio);
  eq('年龄比例槽', inputs[STATE_BASE + 1], org.ageRatio);
  eq('移动能力槽 = 0（无 mover）', inputs[STATE_BASE + 2], 0);
  eq('偏置槽 = 1', inputs[STATE_BASE + 3], 1);
  eq('输入维度正确', inputs.length, SENSE_DIM);
  eq('标签表长度 = 40', INPUT_LABELS.length, 40);
}

{
  // 眼睛看到食物 → 对应方向槽 one-hot + 距离
  const { world } = makeWorld();
  world.fillGrid(CellType.EMPTY);

  const org = makeWatcher(world, { eyeDir: 1 });      // 局部方向 1（右）
  world.setCell(53, 50, CellType.FOOD);               // 眼睛在 (51,50)，往右第 2 格

  const inputs = org.sensors.sense(org);
  const base = 1 * VISUAL_SLOTS;

  eq('右侧槽：食物 one-hot', inputs[base + 0], 1);
  eq('右侧槽：距离 = 1 - 2/20', inputs[base + 8], 0.9);
  eq('其他方向（前）无输入', inputs[0 * VISUAL_SLOTS + 0], 0);
  eq('其他方向（后）无输入', inputs[2 * VISUAL_SLOTS + 0], 0);
}

{
  // 看到墙 → 墙的槽亮，食物槽不亮
  const { world } = makeWorld();
  world.fillGrid(CellType.EMPTY);

  const org = makeWatcher(world, { eyeDir: 1 });
  world.setCell(52, 50, CellType.WALL);

  const inputs = org.sensors.sense(org);
  const base = 1 * VISUAL_SLOTS;
  eq('墙的 one-hot 槽 = 1', inputs[base + 1], 1);
  eq('食物槽保持 0', inputs[base + 0], 0);
  eq('距离 = 1 - 1/20', inputs[base + 8], 0.95);
}

{
  // 眼睛朝向随身体旋转
  const { world } = makeWorld();
  world.fillGrid(CellType.EMPTY);

  const org = makeWatcher(world, { eyeDir: 1 });      // 局部右
  org.clearGrid();
  org.rotation = 1;                                   // 身体转 90° → 局部右变成绝对下
  org.writeGrid();

  world.setCell(50, 53, CellType.FOOD);               // 眼睛仍在 (51,50)，往下看

  const inputs = org.sensors.sense(org);
  eq('身体旋转后视线跟着转（局部槽不变）', inputs[1 * VISUAL_SLOTS + 0], 1);
  eq('距离正确', inputs[1 * VISUAL_SLOTS + 8], 0.9);
}

{
  // 视野范围内没东西 → 全 0
  const { world } = makeWorld();
  world.fillGrid(CellType.EMPTY);
  const org = makeWatcher(world, { eyeDir: 1 });
  const inputs = org.sensors.sense(org);
  let sum = 0;
  for (let i = 0; i < STATE_BASE; i++) sum += inputs[i];
  eq('视野内无物 → 视觉全 0', sum, 0);
}

{
  // 超过视野范围 → 看不到
  const { world } = makeWorld();
  world.fillGrid(CellType.EMPTY);
  const org = makeWatcher(world, { eyeDir: 1 });
  const range = world.config.sense.lookRange;
  world.setCell(51 + range + 3, 50, CellType.FOOD);   // 放到视野外

  const inputs = org.sensors.sense(org);
  let sum = 0;
  for (let i = 0; i < STATE_BASE; i++) sum += inputs[i];
  eq('超出视野范围看不到', sum, 0);
}

{
  // seeThroughSelf：自己的身体挡住视线
  const { world } = makeWorld();
  world.fillGrid(CellType.EMPTY);

  const org = makeWatcher(world, { eyeDir: 1, extra: [[CellType.PRODUCER, 2, 0]] });
  // 眼睛在 (51,50)，往右：step1 → (52,50) 是自己的生产者格
  world.setCell(53, 50, CellType.FOOD);               // 食物在更远处

  world.config.sense.seeThroughSelf = false;
  const blocked = org.sensors.sense(org);
  let sumBlocked = 0;
  for (let i = 0; i < STATE_BASE; i++) sumBlocked += blocked[i];
  eq('seeThroughSelf=false：被自己挡住 → 看不到', sumBlocked, 0);

  world.config.sense.seeThroughSelf = true;
  const through = org.sensors.sense(org);
  eq('seeThroughSelf=true：穿过自身看到食物', through[1 * VISUAL_SLOTS + 0], 1);
}

{
  // 视野被边界截断时不崩
  const { world } = makeWorld();
  world.fillGrid(CellType.EMPTY);
  world.config.world.wrapEdges = false;
  world.grid.wrap = false;

  const org = new Organism(world, 1, 1);
  org.anatomy.addCell(CellType.MOUTH, 0, 0);
  org.anatomy.addCell(CellType.EYE, -1, 0, 3);        // 朝左，正对边界
  world.addOrganism(org);

  let threw = false;
  try { org.sensors.sense(org); } catch { threw = true; }
  ok('视线撞到边界不抛异常', !threw);
}

{
  // 多个眼睛指向同一方向：取更近的观测
  const { world } = makeWorld();
  world.fillGrid(CellType.EMPTY);

  const org = new Organism(world, 50, 50);
  org.anatomy.addCell(CellType.MOUTH, 0, 0);
  org.anatomy.addCell(CellType.EYE, 3, 0, 1);         // 远的眼睛，朝右
  org.anatomy.addCell(CellType.EYE, 1, 0, 1);         // 近的眼睛，朝右
  org.rotation = 0;
  world.addOrganism(org);

  world.setCell(53, 50, CellType.WALL);               // 近眼看到（距离 2）
  world.setCell(60, 50, CellType.FOOD);               // 远眼看到（距离 7）

  const inputs = org.sensors.sense(org);
  const base = 1 * VISUAL_SLOTS;
  eq('同方向多眼 → 取更近的观测（墙）', inputs[base + 1], 1);
  eq('距离来自近眼', inputs[base + 8], 1 - 2 / world.config.sense.lookRange);
}

// ────────────────────────────────────────────────────────────
section('5. 大脑遗传');

{
  // 变异率 0 → 子代大脑与母体完全一致
  const cfg = snapshotConfig();
  cfg.mutation.useGlobalMutability = true;
  cfg.mutation.globalMutability = 0;
  cfg.mutation.weightMutRate = 0;

  const rng = new RNG(4321);
  const world = new World(cfg, rng);
  world.fillGrid(CellType.EMPTY);

  const parent = createOriginOrganism(world, 50, 50);
  parent.energy = 1000;
  world.addOrganism(parent);

  parent.reproduce();
  const child = world.organisms[1];

  ok('繁殖产生了子代', !!child);
  if (child) {
    let same = true;
    for (let i = 0; i < parent.brain.w1.length; i++) {
      if (child.brain.w1[i] !== parent.brain.w1[i]) same = false;
    }
    ok('变异率为 0 时子代大脑与母体完全一致', same);
    ok('子代大脑是独立对象', child.brain !== parent.brain);
  }
}

{
  // 权重变异率 100% → 子代大脑与母体不同
  const cfg = snapshotConfig();
  cfg.mutation.useGlobalMutability = true;
  cfg.mutation.globalMutability = 0;      // 关掉结构变异，只测权重
  cfg.mutation.weightMutRate = 1;
  cfg.mutation.weightMutSigma = 0.3;
  cfg.mutation.weightResetRate = 0;

  const rng = new RNG(555);
  const world = new World(cfg, rng);
  world.fillGrid(CellType.EMPTY);

  const parent = createOriginOrganism(world, 50, 50);
  parent.energy = 1000;
  world.addOrganism(parent);
  parent.reproduce();
  const child = world.organisms[1];

  if (child) {
    let diff = 0;
    for (let i = 0; i < parent.brain.w1.length; i++) {
      if (child.brain.w1[i] !== parent.brain.w1[i]) diff++;
    }
    ok('权重变异率 100% → 子代大脑明显不同', diff > 400, `${diff} / 480`);
    eq('权重变异不产生新物种', child.speciesFingerprint, parent.speciesFingerprint);
  }
}

// ────────────────────────────────────────────────────────────
section('6. 决策与行为');

{
  // 没有移动器 → 不产生移动意图
  const { world } = makeWorld();
  world.fillGrid(CellType.EMPTY);
  const org = createOriginOrganism(world, 50, 50);
  world.addOrganism(org);
  org.think();
  ok('无移动器时 planTurn = false', org.planTurn === false);
}

{
  // 有移动器 → 意图方向合法
  const { world } = makeWorld();
  world.fillGrid(CellType.EMPTY);
  const org = createOriginOrganism(world, 50, 50);
  org.anatomy.addCell(CellType.MOVER, 0, 1);
  world.addOrganism(org);

  let allValid = true;
  for (let i = 0; i < 200; i++) {
    org.think();
    if (org.planDirection < 0 || org.planDirection > 3) allValid = false;
  }
  ok('移动意图方向始终在 0-3', allValid);

  // 决策确实来自网络：手工把某个输出拉高，方向应当跟着变
  const brain = org.brain;
  brain.w1.fill(0); brain.b1.fill(0); brain.w2.fill(0); brain.b2.fill(0);
  brain.b2[2] = 5;                     // 让相对方向 2（后）成为最强输出
  org.rotation = 0;
  org.turnCooldown = 0;
  org.think();
  eq('输出 argmax → 相对方向 2', org.planDirection, 2);

  org.rotation = 1;                    // 身体转 90°：相对方向 2 变成绝对方向 3
  org.think();
  eq('相对方向随身体旋转（2 + 1 = 3）', org.planDirection, 3);

  // o[4] 最大 → 转身
  brain.b2.fill(0);
  brain.b2[4] = 5;
  org.turnCooldown = 0;
  org.think();
  ok('o[4] 最强时要求转身', org.planTurn === true);

  // 冷却期内不再转身
  org.turnCooldown = 5;
  org.think();
  ok('转身冷却期内不转身', org.planTurn === false);
}

{
  // 不同大脑 → 相同情境下可能做出不同决策
  const { world } = makeWorld();
  world.fillGrid(CellType.EMPTY);

  const decisions = new Set();
  for (let i = 0; i < 12; i++) {
    const org = new Organism(world, 20 + i * 3, 20);
    org.anatomy.addCell(CellType.MOUTH, 0, 0);
    org.anatomy.addCell(CellType.MOVER, 0, 1);
    world.addOrganism(org);
    org.think();
    decisions.add(org.planDirection);
  }
  ok('不同个体在相同情境下会做出不同决策', decisions.size > 1, `出现了 ${decisions.size} 种方向`);
}

{
  // 网络输出会随输入变化（不是常量）
  const { world } = makeWorld();
  world.fillGrid(CellType.EMPTY);

  const org = makeWatcher(world, { eyeDir: 1, extra: [[CellType.MOVER, 0, 1]] });
  const before = Array.from(org.brain.forward(org.sensors.sense(org)));

  world.setCell(52, 50, CellType.FOOD);       // 眼睛前方出现食物
  const after = Array.from(org.brain.forward(org.sensors.sense(org)));

  let changed = false;
  for (let i = 0; i < before.length; i++) if (before[i] !== after[i]) changed = true;
  ok('看到食物后网络输出发生变化', changed);
}

{
  // 转身冷却生效：转身后 moveRange 内不再转身
  const cfg = snapshotConfig();
  cfg.mutation.useGlobalMutability = true;
  cfg.mutation.globalMutability = 0;

  const rng = new RNG(8888);
  const world = new World(cfg, rng);
  world.fillGrid(CellType.EMPTY);

  const org = createOriginOrganism(world, 50, 50);
  org.anatomy.addCell(CellType.MOVER, 0, 1);
  world.addOrganism(org);

  // 强制网络一直要求转身
  org.brain.w1.fill(0); org.brain.b1.fill(0); org.brain.w2.fill(0); org.brain.b2.fill(0);
  org.brain.b2[4] = 5;

  let turns = 0;
  for (let t = 0; t < 60; t++) {
    org.think();
    if (org.planTurn) turns++;
    org.act();
  }
  const cd = cfg.life.turnCooldown;
  ok('转身冷却限制了转身频率', turns <= Math.ceil(60 / (cd + 1)) + 1, `60 tick 内转身 ${turns} 次（冷却 ${cd}）`);
}

// ────────────────────────────────────────────────────────────
section('7. 可复现性');

{
  function run(seed) {
    const cfg = snapshotConfig();
    const rng = new RNG(seed);
    const world = new World(cfg, rng);
    generateDemoWorld(world);
    seedOriginOfLife(world);
    spawnPopulation(world, 6);
    for (let i = 0; i < 600; i++) world.step();
    return world;
  }

  const a = run(20261015);
  const b = run(20261015);
  const c = run(20261016);

  eq('同种子 600 tick 后生物数一致', a.organisms.length, b.organisms.length);

  let gridSame = true;
  for (let i = 0; i < a.grid.types.length; i++) {
    if (a.grid.types[i] !== b.grid.types[i]) { gridSame = false; break; }
  }
  ok('同种子 600 tick 后网格完全一致', gridSame);

  let brainSame = true;
  for (let i = 0; i < Math.min(a.organisms.length, b.organisms.length); i++) {
    for (let j = 0; j < a.organisms[i].brain.w1.length; j++) {
      if (a.organisms[i].brain.w1[j] !== b.organisms[i].brain.w1[j]) { brainSame = false; break; }
    }
  }
  ok('同种子 600 tick 后大脑权重一致', brainSame);

  let different = a.organisms.length !== c.organisms.length;
  if (!different) {
    for (let i = 0; i < a.grid.types.length; i++) {
      if (a.grid.types[i] !== c.grid.types[i]) { different = true; break; }
    }
  }
  ok('不同种子结果不同', different);
}

// ────────────────────────────────────────────────────────────
section('8. 长程生态');

{
  const cfg = snapshotConfig();
  const rng = new RNG(20261015);
  const world = new World(cfg, rng);
  generateDemoWorld(world);
  seedOriginOfLife(world);
  spawnPopulation(world, 6);

  const TICKS = 12000;
  const t0 = Date.now();
  let peakMovers = 0;
  let peakEyes = 0;

  for (let t = 0; t < TICKS; t++) {
    world.step();
    if (world.tick % 1000 === 0) {
      const bio = world.organismStats();
      if (bio.movers > peakMovers) peakMovers = bio.movers;
      if (bio.eyes > peakEyes) peakEyes = bio.eyes;
    }
  }
  const elapsed = Date.now() - t0;

  const bio = world.organismStats();
  ok('12000 tick 后种群未灭绝', world.organisms.length > 30, `${world.organisms.length} 个生物`);
  ok('单 tick 耗时仍在预算内（含神经网络前向传播）', elapsed / TICKS < 8, `${(elapsed / TICKS).toFixed(3)} ms/tick`);
  ok('演化出过会移动的个体', peakMovers > 0, `峰值 ${peakMovers} 个`);
  ok('演化出过有眼睛的个体', peakEyes > 0, `峰值 ${peakEyes} 个`);
  ok('平均体型合理', bio.avgCells >= 1 && bio.avgCells < 60, `${bio.avgCells.toFixed(2)} 格`);

  console.log(`      ${TICKS} tick / ${elapsed} ms · 最终 ${world.organisms.length} 生物 · ${world.species.aliveCount} 存活物种`);
  console.log(`      平均体型 ${bio.avgCells.toFixed(2)} 格 · 会移动的 ${bio.movers} · 有眼睛的 ${bio.eyes} · 有杀手的 ${bio.killers}`);
}

// ────────────────────────────────────────────────────────────
console.log(`\n${'='.repeat(52)}`);
if (fail === 0) console.log(`全部通过 ✓  (${pass} 项)`);
else console.log(`通过 ${pass} 项，失败 ${fail} 项 ✗`);
process.exit(fail === 0 ? 0 : 1);
