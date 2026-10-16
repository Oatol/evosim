/**
 * 全局参数表。
 *
 * 所有可调参数集中在这里，UI 面板直接绑定本对象。
 * 设计文档 §10 的默认值即此处。
 */

import { CELL_DEFS, cellAttrDefaults, CELL_ATTR_FIELDS } from '../life/CellType.js';

/** 每种身体细胞的可调属性表，初值来自注册表里的出厂默认 */
function cellAttrTable() {
  const table = {};
  for (const d of CELL_DEFS) {
    if (!d.isBodyCell) continue;
    table[d.key] = cellAttrDefaults(d.id);
  }
  return table;
}

export const Config = {
  world: {
    cols: 200,
    rows: 150,
    wrapEdges: true,
    foodDropProb: 0,          // 0 = 关闭全局食物掉落（靠生产者细胞循环）
    foodDecayRate: 2,         // 每 tick 腐败掉的食物存量比例（%）
    clearWallsOnReset: false,
    autoReset: false,
  },
  life: {
    lifespanMultiplier: 100,
    extraMoverFoodCost: 0,
    maxOrganisms: -1,          // -1 = 无上限
    foodProdProb: 5,           // 生产者细胞每 tick 生成食物的概率（%）
    moversCanProduce: false,   // 会动的生物能否使用生产者细胞
    turnCooldown: 4,           // 转身后的冷却 tick 数（防止原地打转）
    metabolismScale: 0,        // 全局消耗量倍率（0 = 关闭每 tick 消耗）
    foodEnergyScale: 1,        // 食物量倍率：一份食物能提供多少能量
    starvationTicks: 60,       // 能量见底后还能撑多少个 tick 才饿死
  },

  /**
   * 地形效果数值（阶段 8）。
   *
   * 效果本身写在 `life/Terrain.js`（哪个地形做什么），
   * 这里只放"做多少"——和细胞属性同一个思路。
   */
  terrain: {
    lavaEnterDamage: 2,        // 踏进岩浆那一下的冲击伤害
    lavaStandDamage: 1,        // 待在岩浆里每 tick 的持续伤害
    swampMoveCost: 0.5,        // 在沼泽里移动一次额外消耗的能量
    radDamage: 0.25,           // 待在辐射带里每 tick 扣的血
    radMutationBoost: 3,       // 辐射带对后代变异率的倍率（不是免疫式加成，是乘法）
    humusFoodProb: 0.5,        // 腐殖质每 tick 往相邻空格冒食物的概率（%）
  },

  /**
   * 细胞属性表：每种身体细胞一组。
   * 结构由 CellType 注册表决定，这里只放"可调的值"。
   * 参数面板的「细胞属性」分组就是从这张表生成的。
   */
  cells: cellAttrTable(),

  mutation: {
    addProb: 25,
    changeProb: 25,
    removeProb: 25,
    weightMutRate: 0.02,
    weightMutSigma: 0.20,
    weightResetRate: 0.01,
    useGlobalMutability: false,
    globalMutability: 5,
    initialMutability: 5,      // 新生生物的初始变异率（%）
    mutabilitySigma: 0.6,      // 元变异：变异率自身每次漂移的幅度
  },

  sense: {
    lookRange: 20,
    seeThroughSelf: false,
  },

  combat: {
    instaKill: false,
    foodBlocksReproduction: true,
  },

  stats: {
    interval: 100,
    recordSize: 2000,
    minDiscard: 10,
  },

  brain: {
    inSize: 40,
    hiddenSize: 12,
    outSize: 5,
  },

  sim: {
    baseTicksPerSecond: 60,
    maxStepsPerFrame: 400,
    seed: 20261015,
  },
};

/** 参数的分组标题，供 UI 面板渲染 */
export const CONFIG_SECTIONS = [
  { key: 'world',    label: '世界' },
  { key: 'life',     label: '生命' },
  { key: 'terrain',  label: '地形' },
  { key: 'cells',    label: '细胞属性' },
  { key: 'mutation', label: '变异' },
  { key: 'sense',    label: '感知' },
  { key: 'combat',   label: '战斗' },
  { key: 'stats',    label: '统计' },
  { key: 'brain',    label: '神经网络' },
  { key: 'sim',      label: '运行' },
];

/** 参数的中文标签与取值范围，供 UI 面板渲染 */
export const CONFIG_FIELDS = {
  world: {
    // 尺寸是"延迟生效"参数：改完还要点应用，因为重建世界会清空当前生态
    cols: { label: '网格列数', min: 40, max: 800, step: 10, deferred: 'resize' },
    rows: { label: '网格行数', min: 30, max: 600, step: 10, deferred: 'resize' },
    wrapEdges: { label: '边界环绕', type: 'bool', rebuildHint: '立即生效' },
    foodDropProb: { label: '食物掉落率', min: 0, max: 200, step: 1 },
    foodDecayRate: { label: '食物腐败率(%)', min: 0, max: 20, step: 0.1 },
    clearWallsOnReset: { label: '重置时保留地形', type: 'bool', note: '保留墙 / 岩浆 / 沼泽 / 辐射带 / 腐殖质' },
    autoReset: { label: '灭绝后自动重置', type: 'bool' },
  },
  life: {
    lifespanMultiplier: { label: '寿命倍率', min: 1, max: 1000, step: 1 },
    extraMoverFoodCost: { label: '移动者额外成本', min: 0, max: 20, step: 1 },
    maxOrganisms: { label: '种群上限', min: -1, max: 5000, step: 50, note: '-1 = 无上限' },
    foodProdProb: { label: '生产者造食率', min: 0, max: 100, step: 1 },
    moversCanProduce: { label: '移动者可造食', type: 'bool' },
    turnCooldown: { label: '转身冷却(tick)', min: 0, max: 40, step: 1 },
    metabolismScale: { label: '全局消耗量倍率', min: 0, max: 5, step: 0.05, note: '乘在所有细胞的每 tick 消耗上。0 = 关闭。调高会带来真实的选择压力，但当前食物经济撑不住太大值' },
    foodEnergyScale: { label: '食物量倍率', min: 0.1, max: 10, step: 0.1, note: '一份食物能提供多少能量（默认 1）。调高 → 吃一口就攒够繁殖，种群扩张更快' },
    starvationTicks: { label: '饿死延时(tick)', min: 0, max: 600, step: 5, note: '能量见底后还能撑多久' },
  },
  terrain: {
    lavaEnterDamage: { label: '岩浆·踏入伤害', min: 0, max: 20, step: 1, note: '踏进去那一下挨多少' },
    lavaStandDamage: { label: '岩浆·驻留伤害', min: 0, max: 20, step: 1, note: '待在岩浆里每 tick 挨多少' },
    swampMoveCost: { label: '沼泽·移动耗能', min: 0, max: 10, step: 0.05 },
    radDamage: { label: '辐射带·每tick伤害', min: 0, max: 10, step: 0.05 },
    radMutationBoost: { label: '辐射带·变异率倍率', min: 1, max: 20, step: 0.5, note: '站过辐射带的个体，后代变异率 × 这个数' },
    humusFoodProb: { label: '腐殖质·造食率(%)', min: 0, max: 20, step: 0.1 },
  },
  // 每种身体细胞的属性（生命值/消耗/伤害/感知/食物值）。
  // 键是 `类型KEY.属性名`，下面按注册表自动填。
  cells: {},
  mutation: {
    addProb: { label: '加细胞概率', min: 0, max: 100, step: 1 },
    changeProb: { label: '改细胞概率', min: 0, max: 100, step: 1 },
    removeProb: { label: '删细胞概率', min: 0, max: 100, step: 1 },
    weightMutRate: { label: '权重变异率', min: 0, max: 0.5, step: 0.005 },
    weightMutSigma: { label: '权重变异幅度', min: 0, max: 1, step: 0.01 },
    weightResetRate: { label: '权重重置率', min: 0, max: 0.2, step: 0.005 },
    useGlobalMutability: { label: '启用全局变异率', type: 'bool', note: '打开后忽略个体自带的变异率' },
    globalMutability: { label: '全局变异率值', min: 1, max: 100, step: 1 },
    initialMutability: { label: '初始变异率', min: 1, max: 100, step: 1, note: '只影响新生个体' },
    mutabilitySigma: { label: '变异率漂移幅度', min: 0, max: 5, step: 0.1 },
  },
  sense: {
    lookRange: { label: '视野范围', min: 1, max: 60, step: 1 },
    seeThroughSelf: { label: '可透视自身', type: 'bool' },
  },
  combat: {
    instaKill: { label: '一击必杀', type: 'bool' },
    foodBlocksReproduction: { label: '食物阻挡繁殖', type: 'bool' },
  },
  stats: {
    interval: { label: '采样间隔(tick)', min: 10, max: 1000, step: 10 },
    recordSize: { label: '记录长度', min: 100, max: 10000, step: 100, rebuildHint: '立即重建缓冲区' },
    minDiscard: { label: '灭绝过滤阈值', min: 0, max: 200, step: 1 },
  },
  brain: {
    inSize: { label: '输入维度', min: 8, max: 128, step: 1, readonly: true, note: '由感知编码决定' },
    hiddenSize: { label: '隐藏维度', min: 2, max: 64, step: 1, note: '只影响新生个体' },
    outSize: { label: '输出维度', min: 2, max: 16, step: 1, readonly: true, note: '由动作集决定' },
  },
  sim: {
    baseTicksPerSecond: { label: '基准 tick 速率', min: 1, max: 240, step: 1 },
    maxStepsPerFrame: { label: '单帧最大步数', min: 10, max: 5000, step: 10 },
    seed: { label: '随机种子', min: 1, max: 2147483647, step: 1, deferred: 'seed', note: '下次重置生效' },
  },
};

// 按注册表把「细胞属性」分组的字段填出来。
// 加一种新细胞类型，这里会自动多出一组控件——不需要手写任何一行。
for (const d of CELL_DEFS) {
  if (!d.isBodyCell) continue;
  for (const f of CELL_ATTR_FIELDS) {
    CONFIG_FIELDS.cells[`${d.key}.${f.name}`] = {
      label: `${d.label} · ${f.label}`,
      min: f.min,
      max: f.max,
      step: f.step,
      note: f.name === 'senseRange' ? '0 = 用全局视野范围' : undefined,
    };
  }
}

/**
 * 出厂默认值 —— 在模块加载时快照一次。
 *
 * 必须在任何代码修改 Config 之前捕获，所以它就放在 Config 定义之后、
 * 且不依赖任何运行时状态。参数面板的"恢复默认"用它。
 */
export const DEFAULT_CONFIG = JSON.parse(JSON.stringify(Config));

/** 深拷贝当前配置快照 */
export function snapshotConfig() {
  return JSON.parse(JSON.stringify(Config));
}

/** 用一份快照覆盖当前配置 */
export function restoreConfig(snap) {
  for (const section of Object.keys(snap)) {
    if (Config[section]) Object.assign(Config[section], snap[section]);
  }
}
