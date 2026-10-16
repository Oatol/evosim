import { Brain } from '../life/Brain.js';
import { Config } from '../core/config.js';
import { spawnFromBlueprint } from '../life/OrganismFactory.js';
import { serializeBrain, restoreBrain, storageAvailable } from './SaveLoad.js';

/**
 * 生物库 —— 用户自己收集/设计的生物。
 *
 * 一条记录 = 一份蓝图（身体）+ 一份大脑权重 + 名字。可以：
 *   · 从检视器里"保存生物"（连它当时的大脑一起存下来）
 *   · 在物种工坊里手绘一个（没有大脑，投放时随机初始化）
 *   · 存进 localStorage，或导出成文件带走
 *
 * 为什么权重也用 base64 原始字节：和世界存档同一个理由——
 * JSON 数字数组会在浮点转十进制时丢精度，存回来的大脑就不是原来那个了。
 * 详见 SaveLoad.js 的注释。
 */

export const LIBRARY_FORMAT = 'evosim-creatures';
export const LIBRARY_VERSION = 1;
export const LIBRARY_SLOT = 'evosim.creatures';

/** 蓝图 cells（对象数组）→ 紧凑数组，省体积 */
function packCells(cells) {
  const out = new Array(cells.length);
  for (let i = 0; i < cells.length; i++) {
    const c = cells[i];
    out[i] = c.dir ? [c.type, c.dx, c.dy, c.dir] : [c.type, c.dx, c.dy];
  }
  return out;
}

function unpackCells(packed) {
  const out = new Array(packed.length);
  for (let i = 0; i < packed.length; i++) {
    const c = packed[i];
    out[i] = { type: c[0], dx: c[1], dy: c[2], dir: c[3] || 0 };
  }
  return out;
}

export class CreatureLibrary {
  constructor() {
    this.entries = [];
    this._nextId = 1;
    /** 存储不可用（file:// 等）时为 true，UI 用它决定要不要提示 */
    this.persistent = storageAvailable();
    this.load();
  }

  get count() {
    return this.entries.length;
  }

  byId(id) {
    return this.entries.find((e) => e.id === id) || null;
  }

  /**
   * 加一条记录。
   * @param {{cells:Array, brain?:Brain, label?:string}} blueprint
   * @param {string} name
   */
  add(blueprint, name) {
    if (!blueprint || !blueprint.cells || !blueprint.cells.length) return null;
    const entry = {
      id: this._nextId++,
      name: (name || '').trim() || `生物 ${this._nextId - 1}`,
      createdAt: new Date().toISOString(),
      label: blueprint.label || '',
      cells: packCells(blueprint.cells),
      brain: blueprint.brain ? serializeBrain(blueprint.brain) : null,
    };
    this.entries.push(entry);
    this.save();
    return entry;
  }

  remove(id) {
    const i = this.entries.findIndex((e) => e.id === id);
    if (i < 0) return false;
    this.entries.splice(i, 1);
    this.save();
    return true;
  }

  rename(id, name) {
    const e = this.byId(id);
    if (!e) return false;
    e.name = (name || '').trim() || e.name;
    this.save();
    return true;
  }

  clear() {
    this.entries.length = 0;
    this.save();
  }

  /** 记录 → 可直接投放的蓝图（大脑会实例化成一个新的 Brain） */
  blueprintOf(entry) {
    const blueprint = { cells: unpackCells(entry.cells), label: entry.label, brain: null };
    if (entry.brain) {
      // 拓扑不符（例如库来自改过隐藏层尺寸的会话）时保留随机初始化的新大脑
      const brain = new Brain(Config, null);
      if (restoreBrain(brain, entry.brain)) blueprint.brain = brain;
    }
    return blueprint;
  }

  /** 在世界的 (col,row) 投放一条记录，返回个体或 null */
  spawnInto(world, col, row, entry) {
    return spawnFromBlueprint(world, col, row, this.blueprintOf(entry));
  }

  // ────────────────────────────── 持久化 ──────────────────────────────

  load() {
    this.entries.length = 0;
    this._nextId = 1;
    if (!this.persistent) return;
    let raw = null;
    try {
      raw = localStorage.getItem(LIBRARY_SLOT);
    } catch {
      this.persistent = false;
      return;
    }
    if (!raw) return;
    try {
      this.fromJSON(JSON.parse(raw), false);
    } catch {
      // 坏数据不该让整个应用起不来：丢弃并继续
      this.entries.length = 0;
    }
  }

  save() {
    if (!this.persistent) return false;
    try {
      localStorage.setItem(LIBRARY_SLOT, JSON.stringify(this.toJSON()));
      return true;
    } catch {
      // 配额满了：库仍然可用，只是这次没写进去
      return false;
    }
  }

  toJSON() {
    return {
      format: LIBRARY_FORMAT,
      version: LIBRARY_VERSION,
      savedAt: new Date().toISOString(),
      entries: this.entries,
    };
  }

  /**
   * 用一份数据替换整个库。
   * @param {boolean} persist 是否立刻写回 localStorage
   */
  fromJSON(data, persist = true) {
    if (!data || typeof data !== 'object') throw new Error('不是合法的生物库数据');
    if (data.format !== LIBRARY_FORMAT) {
      throw new Error(`不是 evosim 生物库（format = ${JSON.stringify(data.format)}）`);
    }
    if (typeof data.version === 'number' && data.version > LIBRARY_VERSION) {
      throw new Error(`生物库版本 ${data.version} 高于当前支持的 ${LIBRARY_VERSION}`);
    }
    if (!Array.isArray(data.entries)) throw new Error('生物库缺少 entries');

    const next = [];
    let maxId = 0;
    for (const e of data.entries) {
      if (!e || !Array.isArray(e.cells) || !e.cells.length) continue;
      const id = Number.isFinite(e.id) ? e.id : next.length + 1;
      if (id > maxId) maxId = id;
      next.push({
        id,
        name: e.name || `生物 ${id}`,
        createdAt: e.createdAt || null,
        label: e.label || '',
        cells: e.cells,
        brain: e.brain || null,
      });
    }
    this.entries = next;
    this._nextId = maxId + 1;
    if (persist) this.save();
    return this;
  }

  /** 合并一份数据（导入文件时用），返回新增条数 */
  merge(data) {
    if (!data || data.format !== LIBRARY_FORMAT || !Array.isArray(data.entries)) {
      throw new Error('不是 evosim 生物库文件');
    }
    let added = 0;
    for (const e of data.entries) {
      if (!e || !Array.isArray(e.cells) || !e.cells.length) continue;
      this.entries.push({
        id: this._nextId++,
        name: e.name || '导入的生物',
        createdAt: e.createdAt || null,
        label: e.label || '',
        cells: e.cells,
        brain: e.brain || null,
      });
      added++;
    }
    this.save();
    return added;
  }
}
