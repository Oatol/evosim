import { LIVING_CELL_TYPES, isDirectional } from './CellType.js';
import { NEIGHBORS_8 } from './BodyCell.js';

/**
 * 变异算子 —— 设计文档 §6.5。
 *
 * 三件事，顺序固定：
 *   1. 元变异    变异率本身遗传 + 漂移（让"高变异谱系"和"低变异谱系"能分化）
 *   2. 结构变异  加 / 改 / 删细胞 —— 会改变物种指纹
 *   3. 权重变异  神经网络权重扰动 —— 不改变物种指纹（阶段 4 接入）
 *
 * 变异率是**两级**的：
 *   先掷一次 mutability 决定"这一胎要不要变异"，
 *   通过后再对三个结构算子各自独立掷 addProb / changeProb / removeProb。
 * 这样 mutability 是"总开关"（元变异的作用对象），
 * 而三个算子概率是"变异长什么样"的调节旋钮。
 */

/** 加细胞时最多试几次找一个空格（身体可能已经很挤） */
const MAX_ADD_ATTEMPTS = 6;

/**
 * 对子代应用全部变异。就地修改 child。
 *
 * @param {Organism} child  已复制母体蓝图的子代
 * @param {Organism} parent 母体
 * @param {RNG} rng
 * @param {object} config
 * @returns {boolean} 身体结构是否真的变了（变了才有新物种指纹）
 */
export function mutateOrganism(child, parent, rng, config) {
  const mut = config.mutation;

  // ── 1. 元变异：变异率从母体继承 ──
  child.mutability = parent.mutability;

  // ── 2. 结构变异 ──
  const rate = mut.useGlobalMutability ? mut.globalMutability : child.mutability;
  let changed = false;

  if (rate > 0 && rng.chance(rate)) {
    changed = mutateAnatomy(child.anatomy, rng, mut);

    // 变异真的发生了，变异率本身也漂移一点。
    // 只在"变异发生时"漂移（而不是每胎都漂移），是为了让低变异谱系保持稳定——
    // 否则变异率会做无约束的随机游走，演化信号被噪声淹没。
    if (changed) {
      child.mutability = clamp(parent.mutability + rng.gaussian(0, mut.mutabilitySigma), 1, 100);
    }
  }

  // ── 3. 权重变异（阶段 4 才有大脑） ──
  if (child.brain) mutateBrain(child.brain, rng, mut);

  // ── 指纹 ──
  // 结构没变时直接沿用母体指纹，省掉一次排序 + 哈希。
  // 变异率默认只有几个百分点，所以绝大多数繁殖走的是这条零成本路径。
  child.parentFingerprint = parent.speciesFingerprint;
  child.speciesFingerprint = changed
    ? child.anatomy.fingerprint()
    : parent.speciesFingerprint;

  return changed;
}

/**
 * 结构变异：三个算子各自独立判定。
 * @returns {boolean} 是否至少有一个算子真的改动了身体
 */
export function mutateAnatomy(anatomy, rng, mut) {
  let changed = false;

  if (rng.chance(mut.addProb) && addRandomCell(anatomy, rng)) changed = true;
  if (rng.chance(mut.changeProb) && changeRandomCell(anatomy, rng)) changed = true;
  if (rng.chance(mut.removeProb) && anatomy.removeRandomCell(rng)) changed = true;

  return changed;
}

/** 在某个已有细胞的随机八邻域长出一个随机类型的新细胞 */
function addRandomCell(anatomy, rng) {
  for (let attempt = 0; attempt < MAX_ADD_ATTEMPTS; attempt++) {
    const base = anatomy.randomCell(rng);
    if (!base) return false;

    const n = NEIGHBORS_8[rng.int(8)];
    const dx = base.dx + n[0];
    const dy = base.dy + n[1];
    if (anatomy.cellAt(dx, dy)) continue;      // 这个方向已经被占了，换个方向再试

    const type = rng.pick(LIVING_CELL_TYPES);
    return anatomy.addCell(type, dx, dy, isDirectional(type) ? rng.int(4) : 0) !== null;
  }
  return false;
}

/** 把一个随机细胞的类型换掉（眼睛顺带换朝向） */
function changeRandomCell(anatomy, rng) {
  const cell = anatomy.randomCell(rng);
  if (!cell) return false;

  const type = rng.pick(LIVING_CELL_TYPES);
  // 随机到同类型且不带朝向 → 什么也没发生，如实报告
  if (type === cell.type && !isDirectional(type)) return false;

  cell.type = type;
  cell.dir = isDirectional(type) ? rng.int(4) : 0;
  anatomy.recompute();
  return true;
}

/**
 * 权重变异 —— 阶段 4 接入。
 *
 * 权重变异**不改变物种指纹**：身体形态是"物种"的判据，大脑是个体差异。
 * 这样物种树不会被权重漂移搅乱。
 */
export function mutateBrain(brain, rng, mut) {
  const arrays = [brain.w1, brain.b1, brain.w2, brain.b2];
  for (let a = 0; a < arrays.length; a++) {
    const w = arrays[a];
    if (!w) continue;
    for (let i = 0; i < w.length; i++) {
      if (rng.chance(mut.weightMutRate * 100)) {
        if (rng.chance(mut.weightResetRate * 100)) {
          w[i] = rng.range(-1, 1);                        // 整体重置
        } else {
          w[i] += rng.gaussian(0, mut.weightMutSigma);    // 高斯扰动
          if (w[i] > 4) w[i] = 4;
          else if (w[i] < -4) w[i] = -4;
        }
      }
    }
  }
  return brain;
}

function clamp(v, lo, hi) {
  return v < lo ? lo : v > hi ? hi : v;
}
