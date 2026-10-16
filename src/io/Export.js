import { CELL_TYPES } from '../life/CellType.js';

/**
 * 数据导出 —— 设计文档 §9 的"完整实验室"收尾。
 *
 * 两条导出线：
 *   1. 演化时间序列（化石记录）→ 每采样点一行，可拿去画图/做统计
 *   2. 物种表 → 每个物种一行，含祖先关系，可以自己在外部重建系统发生树
 *
 * 一律输出 **CSV**：Excel / pandas / R 都能直接读，而且人也能看懂。
 * 用 UTF-8 BOM 开头，否则 Excel 打开中文表头会乱码。
 */

/** 只导出"身体细胞"类型在群落里的占比（食物/墙不参与这个统计） */
const BODY_TYPES = CELL_TYPES.filter((t) => t.isBodyCell).map((t) => t.id);

function csvCell(value) {
  if (value === null || value === undefined) return '';
  const s = String(value);
  // 含分隔符/引号/换行时按 RFC 4180 用双引号包裹，内部引号翻倍
  return /[",\n\r]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
}

function toCSV(header, rows) {
  const lines = [header.map(csvCell).join(',')];
  for (const row of rows) lines.push(row.map(csvCell).join(','));
  return lines.join('\r\n');
}

/**
 * 化石记录 → CSV。
 *
 * 按时间顺序（最旧 → 最新），每行一个采样点。
 */
export function fossilToCSV(record) {
  const header = ['tick', 'population', 'aliveSpecies', 'avgCells', 'avgMutability'];
  for (const t of BODY_TYPES) header.push(`share_${CELL_TYPES[t].name}`);

  const rows = [];
  for (let i = 0; i < record.count; i++) {
    const s = record.sampleAt(i);
    const row = [
      s.tick,
      s.population,
      s.species,
      s.avgCells.toFixed(4),
      s.avgMutability.toFixed(4),
    ];
    for (const t of BODY_TYPES) row.push(s.shares[t].toFixed(5));
    rows.push(row);
  }
  return toCSV(header, rows);
}

/**
 * 物种表 → CSV。
 *
 * 祖先用 `ancestorId` 表达，外部可以据此重建整棵树。
 * 顺带把 `lifespan` 算好——它是"存续时长"，手工算容易搞错 endTick = -1 的情况。
 */
export function speciesToCSV(registry) {
  const header = [
    'id', 'fingerprint', 'name', 'ancestorId', 'startTick', 'endTick', 'lifespan',
    'alive', 'population', 'peakPopulation', 'cumulativePop', 'cellCount', 'description',
  ];
  const rows = registry.list.map((sp) => [
    sp.id,
    sp.fingerprint,
    sp.name,
    sp.ancestorId === null ? '' : sp.ancestorId,
    sp.startTick,
    sp.endTick,
    sp.lifespan,
    sp.alive ? 1 : 0,
    sp.population,
    sp.peakPopulation,
    sp.cumulativePop,
    sp.cellCount,
    sp.description,
  ]);
  return toCSV(header, rows);
}

/** 触发浏览器下载一段文本 */
export function downloadText(text, filename, mime = 'text/csv;charset=utf-8', bom = true) {
  // BOM：Excel 不认没有 BOM 的 UTF-8，中文表头会变乱码
  const blob = new Blob([bom ? '\ufeff' + text : text], { type: mime });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 2000);
}

// ────────────────────────────── 保存到哪里 ──────────────────────────────

/**
 * 桌面版（evosim.exe）的本地服务会往页面里注入 `window.__EVOSIM_HOST__`，
 * 告诉我们"这个进程可以把文件直接写到程序所在目录"。
 *
 * 浏览器版没有这个标记，于是直接走下载——**一个探测请求都不发**。
 * 这一点很重要：失败的网络请求会在控制台留下 "Failed to load resource"，
 * 而"无控制台错误"是我们所有端到端测试的硬指标。
 */
export function hostSaveDir() {
  if (typeof window === 'undefined') return null;
  const host = window.__EVOSIM_HOST__;
  return host && host.app === 'evosim' && host.dir ? host.dir : null;
}

/**
 * 保存一段文本：优先写到**程序所在目录**，没有该能力时退回浏览器下载。
 *
 * @returns {Promise<{where:'host'|'download', path:string}>} 供 UI 提示用
 */
export async function saveText(text, filename, opts = {}) {
  const mime = opts.mime || 'text/csv;charset=utf-8';
  const bom = opts.bom !== false;              // 默认带 BOM（CSV 场景）
  const payload = bom ? '\ufeff' + text : text;

  const dir = hostSaveDir();
  if (dir) {
    try {
      const res = await fetch('/__evosim/save', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ filename, content: payload }),
      });
      if (res.ok) {
        const data = await res.json();
        return { where: 'host', path: data.path || filename };
      }
    } catch { /* 写盘失败就退回下载，别把导出整个搞坏 */ }
  }

  downloadText(text, filename, mime, bom);
  return { where: 'download', path: filename };
}

/** 给提示条用的一句话：说清文件到底去哪儿了 */
export function describeSaveTarget(result) {
  if (result.where === 'host') return `已保存到程序目录：${result.path}`;
  return `已下载：${result.path}（浏览器下载目录）`;
}

/** 导出文件名：带 tick 与时间戳，避免多次导出互相覆盖 */
export function exportFilename(kind, tick, date = new Date()) {
  const pad = (n, w = 2) => String(n).padStart(w, '0');
  const stamp = `${date.getFullYear()}${pad(date.getMonth() + 1)}${pad(date.getDate())}`
    + `-${pad(date.getHours())}${pad(date.getMinutes())}${pad(date.getSeconds())}`;
  return `evosim-${kind}-tick${tick}-${stamp}.csv`;
}
