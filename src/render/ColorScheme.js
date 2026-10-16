import { CELL_DEFS } from '../life/CellType.js';

/**
 * 配色方案。
 *
 * 每套方案给出背景 + 眼睛瞳孔 + 各细胞类型的颜色。
 * **每种类型的颜色写在类型自己的定义对象里**（`colors` 字段，三套配色各一个），
 * 这里只是把它们按 id 摊平成一张查找表——所以加一种新细胞类型时，
 * 只要在注册表里给它三个颜色，三套配色就都有了，不必来这里补三处。
 */

/** 把注册表里的 per-scheme 颜色摊平成 id → 颜色 */
function colorsFor(schemeName) {
  const map = {};
  for (const d of CELL_DEFS) {
    map[d.id] = (d.colors && d.colors[schemeName]) || '#ff00ff';
  }
  return map;
}

const classic = {
  label: '经典',
  background: '#0b1020',
  gridLine: 'rgba(255,255,255,0.045)',
  accent: '#38bdf8',
  colors: colorsFor('classic'),
  pupil: '#1a1030',
};

const highContrast = {
  label: '高对比',
  background: '#000000',
  gridLine: 'rgba(255,255,255,0.07)',
  accent: '#00e5ff',
  colors: colorsFor('highContrast'),
  pupil: '#000000',
};

const colorblindSafe = {
  label: '色盲友好',
  background: '#0d1117',
  gridLine: 'rgba(255,255,255,0.045)',
  accent: '#56b4e9',
  colors: colorsFor('colorblindSafe'),
  pupil: '#1a1a1a',
};

export const COLOR_SCHEMES = { classic, highContrast, colorblindSafe };

// ────────────────────────────────────────────────────────────
// 物种颜色
// ────────────────────────────────────────────────────────────

/** FNV-1a 32 位哈希（与 Anatomy.fingerprint 同族，这里只用来散列） */
function fnv1a(str) {
  let h = 2166136261 >>> 0;
  for (let i = 0; i < str.length; i++) {
    h ^= str.charCodeAt(i);
    h = Math.imul(h, 16777619) >>> 0;
  }
  return h >>> 0;
}

/**
 * 指纹 → 色相（0–360）。
 *
 * 同一个物种在任何时候、任何视图里都是同一个颜色——这是"按物种着色"
 * 和物种树能互相对照的前提。哈希是确定性的，所以不需要存表。
 */
export function fingerprintHue(fingerprint) {
  if (!fingerprint) return 0;
  return fnv1a(fingerprint) % 360;
}

/** 指纹 → HSL 颜色字符串（画线/填充用） */
export function speciesColor(fingerprint, saturation = 62, lightness = 58) {
  return `hsl(${fingerprintHue(fingerprint)}, ${saturation}%, ${lightness}%)`;
}

/** 指纹 → [r, g, b]（写进 ImageData 的 Uint32 调色板用） */
export function speciesRGB(fingerprint, saturation = 62, lightness = 58) {
  const h = fingerprintHue(fingerprint) / 360;
  const s = saturation / 100;
  const l = lightness / 100;

  if (s === 0) {
    const v = Math.round(l * 255);
    return [v, v, v];
  }
  const q = l < 0.5 ? l * (1 + s) : l + s - l * s;
  const p = 2 * l - q;
  const channel = (t) => {
    let x = t;
    if (x < 0) x += 1;
    if (x > 1) x -= 1;
    if (x < 1 / 6) return p + (q - p) * 6 * x;
    if (x < 1 / 2) return q;
    if (x < 2 / 3) return p + (q - p) * (2 / 3 - x) * 6;
    return p;
  };
  return [
    Math.round(channel(h + 1 / 3) * 255),
    Math.round(channel(h) * 255),
    Math.round(channel(h - 1 / 3) * 255),
  ];
}

export class ColorScheme {
  constructor(name = 'classic') {
    this.setScheme(name);
  }

  setScheme(name) {
    this.name = COLOR_SCHEMES[name] ? name : 'classic';
    this.scheme = COLOR_SCHEMES[this.name];
    return this;
  }

  get background() {
    return this.scheme.background;
  }

  get gridLine() {
    return this.scheme.gridLine;
  }

  get accent() {
    return this.scheme.accent;
  }

  get pupil() {
    return this.scheme.pupil;
  }

  colorOf(typeId) {
    return this.scheme.colors[typeId] ?? '#ff00ff';
  }
}
