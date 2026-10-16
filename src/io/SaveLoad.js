import { CellType } from '../life/CellType.js';
import { Organism } from '../life/Organism.js';
import { Species } from '../stats/Species.js';
import { restoreConfig } from '../core/config.js';

/**
 * 世界存档与读档 —— 设计文档 §11。
 *
 * 存档要"完整可继续演化"，所以四样东西都得存：
 *   1. 配置快照    —— 否则读档后物理规则变了，生态立刻跑偏
 *   2. 网格地形    —— 食物与墙（生物的身体会重新写进去）
 *   3. 全部个体    —— 位置、朝向、能量、年龄、变异率，以及**完整的大脑权重**
 *   4. RNG 状态    —— 否则读档后随机序列从头开始，"继续跑"和"没存过"结果不同
 *
 * 还额外存了物种表：不存的话读档后物种 id 会重新分配，
 * 祖先关系（物种树的骨架）就断了。
 */

export const SAVE_FORMAT = 'evosim-world';
export const SAVE_VERSION = 1;

/**
 * 权重用 base64 编码原始字节，而不是 JSON 数字数组。
 *
 * 两个理由，缺一不可：
 *   1. **精确**。JSON 里的浮点数如果做四舍五入，读档后演化轨迹就不再逐位一致——
 *      而"存档能原样接着跑"是这个模拟的硬要求（RNG 那边专门为此做了 getState）。
 *      编码原始 Float32 字节是零损失的。
 *   2. **小**。557 个权重写成十进制数字约 3.4KB，编码成 base64 只有 2.6KB；
 *      1500 个个体就是 1.2MB 的差距。
 *
 * 代价是存档里这几行不可读——所以身体蓝图仍然保持明文数组，
 * 需要人眼检查的部分（形态）是可读的，只有权重是机器码。
 *
 * 注：依赖小端字节序（所有主流平台都是）。
 */
function encodeFloats(arr) {
  const bytes = new Uint8Array(arr.buffer, arr.byteOffset, arr.byteLength);
  let bin = '';
  const CHUNK = 0x8000;          // 分块避免超长参数列表爆栈
  for (let i = 0; i < bytes.length; i += CHUNK) {
    bin += String.fromCharCode.apply(null, bytes.subarray(i, i + CHUNK));
  }
  return btoa(bin);
}

function decodeFloats(str, expectedLength) {
  const bin = atob(str);
  if (bin.length !== expectedLength * 4) {
    throw new Error(`权重数据长度不符（${bin.length} 字节，期望 ${expectedLength * 4}）`);
  }
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  const out = new Float32Array(expectedLength);
  new Uint8Array(out.buffer).set(bytes);
  return out;
}

// ────────────────────────────────────────────────────────────
// 序列化
// ────────────────────────────────────────────────────────────

export function serializeBrain(brain) {
  return {
    inSize: brain.inSize,
    hiddenSize: brain.hiddenSize,
    outSize: brain.outSize,
    w1: encodeFloats(brain.w1),
    b1: encodeFloats(brain.b1),
    w2: encodeFloats(brain.w2),
    b2: encodeFloats(brain.b2),
  };
}

function serializeOrganism(org) {
  const cells = org.anatomy.cells;
  const out = new Array(cells.length);
  for (let i = 0; i < cells.length; i++) {
    const c = cells[i];
    // 只有眼睛带朝向，省掉一个数字 × 几千个细胞
    out[i] = c.dir ? [c.type, c.dx, c.dy, c.dir] : [c.type, c.dx, c.dy];
  }

  return {
    col: org.col,
    row: org.row,
    rot: org.rotation,
    dir: org.direction,
    energy: org.energy,
    age: org.age,
    damage: org.damage,
    mut: org.mutability,
    fp: org.speciesFingerprint,
    pfp: org.parentFingerprint ?? null,
    cells: out,
    brain: serializeBrain(org.brain),
  };
}

function serializeSpecies(reg) {
  return {
    nextId: reg.nextId,
    aliveCount: reg.aliveCount,
    peakAliveCount: reg.peakAliveCount,
    totalCreated: reg.totalCreated,
    totalExtinct: reg.totalExtinct,
    list: reg.list.map((sp) => ({
      id: sp.id,
      fp: sp.fingerprint,
      name: sp.name,
      desc: sp.description,
      cellCount: sp.cellCount,
      ancestorId: sp.ancestorId,
      startTick: sp.startTick,
      endTick: sp.endTick,
      population: sp.population,
      peakPopulation: sp.peakPopulation,
      cumulativePop: sp.cumulativePop,
      alive: sp.alive,
      lastSeenTick: sp.lastSeenTick,
    })),
  };
}

/**
 * 把世界打包成一个纯 JSON 对象。
 *
 * 地形只存非空格，而且存**扁平索引**而不是 [col, row]：
 * 索引在环绕世界里是无歧义的（[col,row] 还要重新归一化），
 * 而且体积只有一半。
 *
 * 食物列表**按 world.foodCells 的原样顺序**存，而不是按索引升序重排——
 * 这一点至关重要：食物腐败是 `foodCells[rng.int(len)]` 按**数组位置**抽样的，
 * 顺序一变，同样的随机数就挑到不同的食物，读档后演化立刻分叉。
 * foodCells 的顺序是 swap-remove 留下的历史产物，重排无法复原它，只能原样搬运。
 */
export function serializeWorld(world, rng, extra = {}) {
  const grid = world.grid;
  const types = grid.types;
  const walls = [];

  // 食物：直接搬 world.foodCells（顺序有意义）
  const food = Array.from(world.foodCells);

  for (let i = 0; i < types.length; i++) {
    if (types[i] === CellType.WALL) walls.push(i);
  }

  const organisms = new Array(world.organisms.length);
  for (let i = 0; i < world.organisms.length; i++) {
    organisms[i] = serializeOrganism(world.organisms[i]);
  }

  return {
    format: SAVE_FORMAT,
    version: SAVE_VERSION,
    savedAt: new Date().toISOString(),
    tick: world.tick,
    grid: {
      cols: grid.cols,
      rows: grid.rows,
      wrap: grid.wrap,
      food,
      walls,
    },
    config: extra.config ?? null,
    rng: rng ? rng.getState() : null,
    species: serializeSpecies(world.species),
    organisms,
  };
}

// ────────────────────────────────────────────────────────────
// 反序列化
// ────────────────────────────────────────────────────────────

/** 存档格式校验。失败时抛出可读的错误信息，而不是让后面某处莫名崩掉。 */
export function validateSave(data) {
  if (!data || typeof data !== 'object') throw new Error('存档不是合法的 JSON 对象');
  if (data.format !== SAVE_FORMAT) {
    throw new Error(`不是 evosim 存档（format = ${JSON.stringify(data.format)}）`);
  }
  if (typeof data.version !== 'number' || data.version > SAVE_VERSION) {
    throw new Error(`存档版本 ${data.version} 高于当前支持的 ${SAVE_VERSION}`);
  }
  if (!data.grid || typeof data.grid.cols !== 'number' || typeof data.grid.rows !== 'number') {
    throw new Error('存档缺少网格信息');
  }
  if (!Array.isArray(data.organisms)) throw new Error('存档缺少个体列表');
  if (!data.species || !Array.isArray(data.species.list)) throw new Error('存档缺少物种表');
  return data;
}

export function restoreBrain(brain, raw) {
  if (!raw) return false;
  if (brain.inSize !== raw.inSize
    || brain.hiddenSize !== raw.hiddenSize
    || brain.outSize !== raw.outSize) {
    // 拓扑对不上（例如存档来自改过隐藏层尺寸的会话）：
    // 保留构造函数里刚随机出来的权重，宁可让这个个体是"新大脑"，
    // 也不要按错误的步长硬塞出乱码。
    return false;
  }
  // 权重在存档里是 base64 编码的原始 Float32 字节，要解码回 TypedArray。
  // decodeFloats 会校验长度，所以"存的和现在的大脑对不上"会当场抛出，
  // 而不是静默写进一半。
  brain.w1.set(decodeFloats(raw.w1, brain.w1.length));
  brain.b1.set(decodeFloats(raw.b1, brain.b1.length));
  brain.w2.set(decodeFloats(raw.w2, brain.w2.length));
  brain.b2.set(decodeFloats(raw.b2, brain.b2.length));
  return true;
}

function restoreSpeciesRegistry(world, raw) {
  const reg = world.species;
  reg.map.clear();
  reg.list.length = 0;

  for (let i = 0; i < raw.list.length; i++) {
    const s = raw.list[i];
    // anatomy 传 null：物种只存描述快照，本来就不该持有蓝图（§7.2）
    const sp = new Species(s.id, s.fp, null, s.ancestorId, s.startTick);
    sp.name = s.name;
    sp.description = s.desc;
    sp.cellCount = s.cellCount;
    sp.endTick = s.endTick;
    sp.population = s.population;
    sp.peakPopulation = s.peakPopulation;
    sp.cumulativePop = s.cumulativePop;
    sp.alive = s.alive;
    sp.lastSeenTick = s.lastSeenTick;
    reg.map.set(sp.fingerprint, sp);
    reg.list.push(sp);
  }

  reg.nextId = raw.nextId ?? (raw.list.length + 1);
  reg.aliveCount = raw.aliveCount ?? 0;
  reg.peakAliveCount = raw.peakAliveCount ?? 0;
  reg.totalCreated = raw.totalCreated ?? raw.list.length;
  reg.totalExtinct = raw.totalExtinct ?? 0;
  return reg;
}

/**
 * 把存档灌回世界。
 *
 * 顺序有讲究：**先恢复配置和网格尺寸，再建生物**——Organism 的构造函数
 * 会按 config.brain.inSize 建感知器缓冲，配置没恢复就构造个体会得到错的维度。
 *
 * @returns {{ organisms: number, food: number, walls: number, species: number, brainSkipped: number }}
 */
export function deserializeWorld(world, rng, rawData) {
  const data = validateSave(rawData);

  // ── 1. 配置 ──
  if (data.config) restoreConfig(data.config);

  // ── 2. 网格（resize 会清空生物、重建物种表、tick 归零）──
  const cols = data.grid.cols;
  const rows = data.grid.rows;
  world.resize(cols, rows);

  const wrap = data.grid.wrap ?? world.config.world.wrapEdges;
  world.grid.wrap = wrap;
  world.config.world.cols = cols;
  world.config.world.rows = rows;
  world.config.world.wrapEdges = wrap;

  // ── 3. 地形：直接写 TypedArray，最后统一重建食物索引 ──
  // （逐个走 setCellAt 会触发几万次 cellChanged 事件，纯属浪费）
  const grid = world.grid;
  grid.types.fill(CellType.EMPTY);
  grid.owners.fill(-1);

  const wallList = data.grid.walls || [];
  const foodList = data.grid.food || [];
  for (let i = 0; i < wallList.length; i++) grid.types[wallList[i]] = CellType.WALL;
  for (let i = 0; i < foodList.length; i++) grid.types[foodList[i]] = CellType.FOOD;

  world.walls = wallList.slice();

  // 食物索引按存档里的**原始顺序**重建，而不是走 _initFoodIndex() 的升序扫描。
  // 腐败抽样依赖数组位置，顺序错了读档后就会分叉（见 serializeWorld 注释）。
  world.foodCells = foodList.slice();
  world.foodPos = new Int32Array(grid.length).fill(-1);
  for (let p = 0; p < world.foodCells.length; p++) world.foodPos[world.foodCells[p]] = p;

  // ── 4. 物种表（必须在建生物之前，这样物种 id 与祖先链原样保留）──
  restoreSpeciesRegistry(world, data.species);

  // ── 5. 个体 ──
  world.organisms.length = 0;
  world._organismsById.clear();
  world.nextOrganismId = 1;

  let brainSkipped = 0;
  for (let i = 0; i < data.organisms.length; i++) {
    const raw = data.organisms[i];
    const org = new Organism(world, raw.col, raw.row);

    org.anatomy.clear();
    for (let j = 0; j < raw.cells.length; j++) {
      const c = raw.cells[j];
      org.anatomy.addCell(c[0], c[1], c[2], c[3] || 0);
    }
    org.anatomy.recompute();

    org.rotation = raw.rot & 3;
    org.direction = raw.dir & 3;
    org.energy = raw.energy;
    org.age = raw.age;
    org.damage = raw.damage || 0;
    org.mutability = raw.mut;
    org.speciesFingerprint = raw.fp || org.anatomy.fingerprint();
    org.parentFingerprint = raw.pfp ?? null;

    if (!restoreBrain(org.brain, raw.brain)) brainSkipped++;

    world.adoptOrganism(org);
  }

  // ── 6. tick 与随机状态 ──
  world.tick = data.tick || 0;
  if (data.rng && rng) rng.setState(data.rng);

  world.species.update(world, world.tick);
  world.emit('fullRedraw');
  world.emit('reset');

  return {
    organisms: world.organisms.length,
    food: foodList.length,
    walls: wallList.length,
    species: world.species.list.length,
    brainSkipped,
  };
}

// ────────────────────────────────────────────────────────────
// 传输：文件下载 / 上传、localStorage 快存
// ────────────────────────────────────────────────────────────

export function saveFilename(world, date = new Date()) {
  const pad = (n, w = 2) => String(n).padStart(w, '0');
  const stamp = `${date.getFullYear()}${pad(date.getMonth() + 1)}${pad(date.getDate())}`
    + `-${pad(date.getHours())}${pad(date.getMinutes())}${pad(date.getSeconds())}`;
  return `evosim-tick${world.tick}-${stamp}.json`;
}

/** 触发浏览器下载 */
export function downloadSave(data, filename) {
  const blob = new Blob([JSON.stringify(data)], { type: 'application/json' });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 2000);
}

/** 读取用户选择的文件，解析成存档对象 */
export function readSaveFile(file) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      try {
        resolve(validateSave(JSON.parse(reader.result)));
      } catch (err) {
        reject(err);
      }
    };
    reader.onerror = () => reject(new Error('文件读取失败'));
    reader.readAsText(file);
  });
}

export const QUICK_SLOT = 'evosim.quicksave';

/**
 * localStorage 是否可用。
 *
 * 单文件版（dist/evosim.html）是走 file:// 打开的，而 Chrome 在 file:// 下
 * 访问 localStorage 会直接抛 SecurityError。这里统一探测一次，
 * 让"快存不可用"表现为一条友好提示，而不是一个未捕获异常。
 */
let _storageOk = null;
export function storageAvailable() {
  if (_storageOk !== null) return _storageOk;
  try {
    const probe = '__evosim_probe__';
    localStorage.setItem(probe, '1');
    localStorage.removeItem(probe);
    _storageOk = true;
  } catch {
    _storageOk = false;
  }
  return _storageOk;
}

/** 快存到 localStorage。配额通常是 5MB，大世界会失败——所以要能优雅报错。 */
export function saveToStorage(data, slot = QUICK_SLOT) {
  if (!storageAvailable()) {
    throw new Error('当前环境不支持本地快存（file:// 下浏览器会禁用 localStorage）。请用"保存到文件"。');
  }
  const json = JSON.stringify(data);
  try {
    localStorage.setItem(slot, json);
  } catch (err) {
    const mb = (json.length / 1048576).toFixed(1);
    throw new Error(`快存失败（需要约 ${mb} MB，超出浏览器存储配额）。请改用"保存到文件"。`);
  }
  return json.length;
}

export function loadFromStorage(slot = QUICK_SLOT) {
  if (!storageAvailable()) return null;
  const raw = localStorage.getItem(slot);
  if (!raw) return null;
  return validateSave(JSON.parse(raw));
}

export function storageInfo(slot = QUICK_SLOT) {
  if (!storageAvailable()) return null;
  const raw = localStorage.getItem(slot);
  if (!raw) return null;
  let savedAt = null;
  let tick = null;
  try {
    const parsed = JSON.parse(raw);
    savedAt = parsed.savedAt || null;
    tick = parsed.tick ?? null;
  } catch { /* 坏存档，只报告大小 */ }
  return { bytes: raw.length, savedAt, tick };
}

export function clearStorage(slot = QUICK_SLOT) {
  if (!storageAvailable()) return;
  localStorage.removeItem(slot);
}
