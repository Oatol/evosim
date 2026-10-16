/**
 * 演化曲线 —— 手写 Canvas mini 折线图，零依赖。
 *
 * 每个图一条曲线：种群数 / 物种数 / 平均体型 / 平均变异率。
 * 数据来自 FossilRecord 的环形缓冲，重绘频率跟读数面板一致（约 7 次/秒），
 * 不跟渲染帧跑——曲线没必要 60fps。
 */

/** 每个图的规格。zeroBased = 是否强制从 0 起（计数类要，均值类不要） */
const CHART_SPECS = [
  { key: 'populations',   label: '种群数',     color: '#2563eb', digits: 0, zeroBased: true },
  { key: 'species',       label: '物种数',     color: '#7c3aed', digits: 0, zeroBased: true },
  { key: 'avgCells',      label: '平均体型',   color: '#059669', digits: 2, zeroBased: false },
  { key: 'avgMutability', label: '平均变异率', color: '#d97706', digits: 1, zeroBased: false },
];

export class Charts {
  constructor(container, record) {
    this.container = container;
    this.record = record;
    this.charts = [];
    this._needsResize = true;

    const styles = getComputedStyle(document.documentElement);
    this.gridColor = styles.getPropertyValue('--border').trim() || 'rgba(16,20,28,0.10)';
    this.emptyColor = styles.getPropertyValue('--text-3').trim() || '#8b939f';
    this.bgColor = styles.getPropertyValue('--surface-2').trim() || '#f0f2f5';

    this._build();
  }

  _build() {
    const el = this.container;
    el.innerHTML = '';

    const title = document.createElement('h2');
    title.className = 'panel-title';
    title.textContent = '演化曲线';
    el.appendChild(title);

    for (const spec of CHART_SPECS) this._addChart(el, spec);

    // 和下方的读数面板之间画条分隔线
    const divider = document.createElement('div');
    divider.className = 'panel-divider';
    el.appendChild(divider);
  }

  _addChart(parent, spec) {
    const wrap = document.createElement('div');
    wrap.className = 'chart';

    const head = document.createElement('div');
    head.className = 'chart-head';

    const label = document.createElement('span');
    label.className = 'chart-label';
    label.textContent = spec.label;

    const value = document.createElement('span');
    value.className = 'chart-value';
    value.textContent = '—';

    head.appendChild(label);
    head.appendChild(value);

    const canvas = document.createElement('canvas');
    canvas.className = 'chart-canvas';

    wrap.appendChild(head);
    wrap.appendChild(canvas);
    parent.appendChild(wrap);

    const entry = {
      spec,
      canvas,
      ctx: canvas.getContext('2d'),
      valueEl: value,
      w: 0,
      h: 0,
      dpr: 0,
      tint: hexToRgba(spec.color, 0.13),
    };
    this.charts.push(entry);
    return entry;
  }

  /** 按 CSS 尺寸 + DPR 同步 canvas 位图尺寸。返回是否发生了变化。 */
  _syncSize(entry) {
    const rect = entry.canvas.getBoundingClientRect();
    const w = Math.round(rect.width);
    const h = Math.round(rect.height);
    if (w <= 0 || h <= 0) return false;      // 面板被隐藏（窄屏降级）

    const dpr = window.devicePixelRatio || 1;
    if (entry.w === w && entry.h === h && entry.dpr === dpr) return false;

    entry.w = w;
    entry.h = h;
    entry.dpr = dpr;
    entry.canvas.width = Math.round(w * dpr);
    entry.canvas.height = Math.round(h * dpr);
    entry.ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    return true;
  }

  refresh() {
    for (let i = 0; i < this.charts.length; i++) {
      const entry = this.charts[i];
      // 只在没量过 / 明确要求重量时才碰 getBoundingClientRect——
      // 它可能触发 layout，而曲线每秒要刷好几次
      const resized = (entry.w === 0 || this._needsResize) ? this._syncSize(entry) : false;
      if (!resized && !entry.dirty && entry.drawnCount === this.record.length) continue;
      this._draw(entry);
      entry.dirty = false;
      entry.drawnCount = this.record.length;
    }
    this._needsResize = false;
  }

  /** 标记全部曲线需要重绘（世界重置、配色变化时用） */
  markDirty() {
    for (const entry of this.charts) entry.dirty = true;
  }

  /** 容器宽度可能变了（窗口缩放 / 侧栏窄屏降级），下次刷新时重新量 */
  onResize() {
    this._needsResize = true;
  }

  _draw(entry) {
    const { ctx, w, h, spec } = entry;
    ctx.clearRect(0, 0, w, h);

    // 背景
    ctx.fillStyle = this.bgColor;
    ctx.fillRect(0, 0, w, h);

    const data = this.record.extract(spec.key, Math.max(64, w * 2));
    const n = data.n;

    if (n === 0) {
      entry.valueEl.textContent = '—';
      return;
    }

    entry.valueEl.textContent = formatNumber(data.values[n - 1], spec.digits);

    if (n < 2) return;

    // ── 值域 ──
    let max = -Infinity;
    let min = Infinity;
    for (let i = 0; i < n; i++) {
      const v = data.values[i];
      if (v > max) max = v;
      if (v < min) min = v;
    }

    let lo;
    let hi;
    if (spec.zeroBased) {
      lo = 0;
      hi = max > 0 ? max * 1.08 : 1;
    } else {
      const span = max - min;
      if (span < 1e-9) {
        // 完全平直：给一个对称的小窗口，让线落在中间
        lo = min - Math.max(Math.abs(min) * 0.1, 0.5);
        hi = max + Math.max(Math.abs(max) * 0.1, 0.5);
      } else {
        lo = min - span * 0.12;
        hi = max + span * 0.12;
      }
    }
    const scale = (h - 4) / (hi - lo || 1);

    const px = (i) => 1 + (i / (n - 1)) * (w - 2);
    const py = (v) => h - 2 - (v - lo) * scale;

    // ── 中线 ──
    ctx.strokeStyle = this.gridColor;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(0, Math.round(h / 2) + 0.5);
    ctx.lineTo(w, Math.round(h / 2) + 0.5);
    ctx.stroke();

    // ── 面积填充 ──
    const baseY = spec.zeroBased ? py(0) : h;
    ctx.beginPath();
    ctx.moveTo(px(0), baseY);
    for (let i = 0; i < n; i++) ctx.lineTo(px(i), py(data.values[i]));
    ctx.lineTo(px(n - 1), baseY);
    ctx.closePath();
    ctx.fillStyle = entry.tint;
    ctx.fill();

    // ── 折线 ──
    ctx.beginPath();
    for (let i = 0; i < n; i++) {
      const x = px(i);
      const y = py(data.values[i]);
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.strokeStyle = spec.color;
    ctx.lineWidth = 1.5;
    ctx.lineJoin = 'round';
    ctx.lineCap = 'round';
    ctx.stroke();

    // ── 末端圆点 ──
    ctx.beginPath();
    ctx.arc(px(n - 1), py(data.values[n - 1]), 1.8, 0, Math.PI * 2);
    ctx.fillStyle = spec.color;
    ctx.fill();
  }
}

function formatNumber(value, digits) {
  if (!Number.isFinite(value)) return '—';
  return digits > 0 ? value.toFixed(digits) : Math.round(value).toLocaleString('en-US');
}

/** #rrggbb → rgba(r,g,b,a) */
function hexToRgba(hex, alpha) {
  const h = hex.replace('#', '');
  const r = parseInt(h.slice(0, 2), 16);
  const g = parseInt(h.slice(2, 4), 16);
  const b = parseInt(h.slice(4, 6), 16);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}
