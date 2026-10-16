import { speciesColor } from '../render/ColorScheme.js';
import { enableDrag } from './floating.js';

/**
 * 系统发生树 —— 设计文档 §9.4。
 *
 * 纵向是时间轴（上旧下新），横向是谱系分支：每个物种是一根竖条，
 * 从它诞生的 tick 一直画到灭绝（或"现在"），横线把它接到祖先那一列。
 *
 * 三个刻意的取舍：
 *   1. **只展开存活分支**：显示"存活物种 + 它们的祖先链"。把几千个已灭绝
 *      的旁支全画出来会糊成一团，而用户真正关心的是"现在活着的这些是从哪来的"。
 *   2. **颜色 = 物种指纹色**（与"按物种着色"共用同一个映射），所以树上的
 *      一根条和世界画布上那片生物是同一个颜色，可以互相对照。
 *   3. **整段历史压缩显示**，不做滚动：树会随时间"向下生长"，
 *      一屏就能看完全部谱系，代价是很久以前的早期分支会变得很细。
 *
 * 做成**非模态浮窗**（和检视器同一套路）：不铺满视口、只有面板吃事件，
 * 其余点击穿透到画布。早先它是全屏模态，遮罩把画布整个盖住——于是在树上
 * 选中一个物种之后，根本看不到世界画布上对应的高亮（用户实际反馈过）。
 *
 * 图钉**默认锁上**：树是"参照物"，你选完物种往往还要去画布上缩放、点个体，
 * 那些点击不该把树关掉。想让它随手点画布就收起，按一下图钉解锁即可。
 */

/** 超过这个数量的节点就按"累计个体数"裁剪（存活物种一律保留） */
const MAX_NODES = 240;

const PAD_LEFT = 56;
const PAD_RIGHT = 18;
const PAD_TOP = 16;
const PAD_BOTTOM = 30;

/**
 * 谱系推导 —— 纯函数，不碰 DOM，所以逻辑测试可以直接调它。
 *
 * 输入物种表（`world.species.list`）与当前 tick，输出：
 *   order        前序遍历后的节点顺序（横向从左到右）
 *   columnOf     id → 列号
 *   parentColumn id → 祖先的列号（画连线用；祖先被裁掉时接到最近的仍在展示中的祖先）
 *   aliveIds     存活物种 id 集合
 *   t0 / t1      时间范围（纵向映射用）
 *
 * 列的顺序用**前序遍历**：父节点紧挨着它的后代，兄弟按诞生时间排序。
 * 这样一棵子树在横向上是连续的，肉眼能看出"这一片是同一个祖先的后代"。
 */
export function buildLineage(list, now, maxNodes = MAX_NODES) {
  const byId = new Map();
  for (const sp of list) byId.set(sp.id, sp);

  // ── 1. 存活物种 ──
  const aliveIds = new Set();
  for (const sp of list) if (sp.population > 0) aliveIds.add(sp.id);

  // ── 2. 存活物种 + 它们的祖先链（"只展开存活分支"）──
  const shown = new Map();
  for (const sp of list) {
    if (!aliveIds.has(sp.id)) continue;
    let cur = sp;
    let guard = 0;
    while (cur && !shown.has(cur.id) && guard++ < 10000) {
      shown.set(cur.id, cur);
      cur = cur.ancestorId === null ? null : byId.get(cur.ancestorId);
    }
  }

  // ── 3. 超上限时裁剪：存活的一律保留，其余按累计个体数留最重要的 ──
  let nodes = [...shown.values()];
  if (nodes.length > maxNodes) {
    const keepAlive = nodes.filter((s) => aliveIds.has(s.id));
    const rest = nodes
      .filter((s) => !aliveIds.has(s.id))
      .sort((a, b) => b.cumulativePop - a.cumulativePop);
    const room = Math.max(0, maxNodes - keepAlive.length);
    nodes = keepAlive.concat(rest.slice(0, room));
  }

  const nodeSet = new Set(nodes.map((s) => s.id));

  /** 往上找到最近的、仍在展示中的祖先 */
  const nearestShownAncestor = (sp) => {
    let anc = sp.ancestorId === null ? null : byId.get(sp.ancestorId);
    let guard = 0;
    while (anc && !nodeSet.has(anc.id) && guard++ < 10000) {
      anc = anc.ancestorId === null ? null : byId.get(anc.ancestorId);
    }
    return anc || null;
  };

  // ── 4. 父子关系 ──
  const childrenOf = new Map();
  const roots = [];
  for (const sp of nodes) {
    const anc = nearestShownAncestor(sp);
    if (anc) {
      if (!childrenOf.has(anc.id)) childrenOf.set(anc.id, []);
      childrenOf.get(anc.id).push(sp);
    } else {
      roots.push(sp);
    }
  }

  // ── 5. 前序遍历定列 ──
  const order = [];
  const columnOf = new Map();
  const visit = (sp) => {
    columnOf.set(sp.id, order.length);
    order.push(sp);
    const kids = childrenOf.get(sp.id);
    if (kids) {
      kids.sort((a, b) => a.startTick - b.startTick);
      for (const k of kids) visit(k);
    }
  };
  roots.sort((a, b) => a.startTick - b.startTick);
  for (const r of roots) visit(r);

  // ── 6. 时间范围 ──
  let t0 = Infinity;
  let t1 = 0;
  for (const sp of order) {
    if (sp.startTick < t0) t0 = sp.startTick;
    const end = sp.endTick >= 0 ? sp.endTick : now;
    if (end > t1) t1 = end;
  }
  if (!Number.isFinite(t0)) t0 = 0;
  t1 = Math.max(t1, now);
  if (t1 <= t0) t1 = t0 + 1;

  // ── 7. 每个节点的父列 ──
  const parentColumn = new Map();
  for (const sp of nodes) {
    const anc = nearestShownAncestor(sp);
    if (anc) parentColumn.set(sp.id, columnOf.get(anc.id));
  }

  return { order, columnOf, parentColumn, aliveIds, t0, t1, now };
}

export class SpeciesTree {
  constructor(world, colorScheme) {
    this.world = world;
    this.colorScheme = colorScheme;

    this.selected = null;
    this.onSelect = null;         // (species|null) => void
    this.onOpenChanged = null;    // (open: boolean) => void

    /** 图钉：默认锁上（树是参照物，不该被画布上的点击关掉） */
    this.pinned = true;

    this._layout = null;
    this._hit = [];
    this._sizeDirty = true;

    this._build();
  }

  get isOpen() {
    return !this.root.hidden;
  }

  // ────────────────────────────── DOM ──────────────────────────────

  _build() {
    const root = document.createElement('div');
    root.className = 'species-tree';
    root.hidden = true;

    const panel = document.createElement('div');
    panel.className = 'species-tree-panel';

    // ── 头部 ──
    const head = document.createElement('header');
    head.className = 'species-tree-head';

    const titles = document.createElement('div');
    titles.className = 'species-tree-titles';
    const title = document.createElement('div');
    title.className = 'species-tree-title';
    title.textContent = '系统发生树';
    this.subtitleEl = document.createElement('div');
    this.subtitleEl.className = 'species-tree-subtitle';
    titles.appendChild(title);
    titles.appendChild(this.subtitleEl);

    // 图钉：锁定后点画布不会把树关掉
    this.pinBtn = document.createElement('button');
    this.pinBtn.className = 'species-tree-pin';
    this.pinBtn.type = 'button';
    this.pinBtn.title = '锁定：锁定后点击画布不会关闭物种树';
    this.pinBtn.innerHTML =
      '<svg viewBox="0 0 24 24" width="13" height="13" aria-hidden="true">' +
      '<path fill="currentColor" d="M16 9V4h1c.55 0 1-.45 1-1s-.45-1-1-1H7c-.55 0-1 .45-1 ' +
      '1s.45 1 1 1h1v5c0 1.66-1.34 3-3 3v2h5.97v7l1 1 1-1v-7H19v-2c-1.66 0-3-1.34-3-3z"/></svg>';
    this.pinBtn.addEventListener('click', () => this.setPinned(!this.pinned));

    const close = document.createElement('button');
    close.className = 'species-tree-close';
    close.textContent = '×';
    close.title = '关闭（Esc）';
    close.addEventListener('click', () => this.close());

    head.appendChild(titles);
    head.appendChild(this.pinBtn);
    head.appendChild(close);

    // ── 画布 ──
    const body = document.createElement('div');
    body.className = 'species-tree-body';
    this.canvas = document.createElement('canvas');
    this.canvas.className = 'species-tree-canvas';
    body.appendChild(this.canvas);

    // ── 底部详情 ──
    this.detailEl = document.createElement('div');
    this.detailEl.className = 'species-tree-detail';
    this.detailEl.textContent = '点击任意一根竖条查看该物种；点击后世界画布上会高亮它的存活个体。';

    panel.appendChild(head);
    panel.appendChild(body);
    panel.appendChild(this.detailEl);
    root.appendChild(panel);
    document.body.appendChild(root);

    this.root = root;
    this.panel = panel;
    this.ctx = this.canvas.getContext('2d');
    this.bodyEl = body;

    this.canvas.addEventListener('click', (e) => this._onClick(e));

    enableDrag(head, panel);
    this.setPinned(this.pinned);
    this._bindOutsideClose();
  }

  /** 图钉开关 */
  setPinned(on) {
    this.pinned = !!on;
    if (this.pinBtn) this.pinBtn.classList.toggle('is-active', this.pinned);
  }

  /**
   * 未锁定时，点**世界画布**才关闭；点工具栏 / 顶栏 / 时间轴一律不关。
   *
   * 与检视器同一条规则——判据是"点没点在画布上"，而不是"点没点在面板外"。
   */
  _bindOutsideClose() {
    document.addEventListener('pointerdown', (event) => {
      if (!this.isOpen || this.pinned) return;
      const t = event.target;
      if (!t || !t.closest) return;
      if (t.closest('.species-tree-panel')) return;
      if (t.closest('.canvas-wrap')) this.close();
    }, true);
  }

  // ────────────────────────────── 开合 ──────────────────────────────

  open() {
    this.root.hidden = false;
    this._sizeDirty = true;
    this.refresh();
    if (this.onOpenChanged) this.onOpenChanged(true);
  }

  close() {
    if (this.root.hidden) return;
    this.root.hidden = true;
    if (this.selected) {
      this.selected = null;
      this._renderDetail();
      if (this.onSelect) this.onSelect(null);
    }
    if (this.onOpenChanged) this.onOpenChanged(false);
  }

  toggle() {
    if (this.isOpen) this.close();
    else this.open();
  }

  markDirty() {
    this._sizeDirty = true;
  }

  onResize() {
    this._sizeDirty = true;
  }

  // ────────────────────────────── 尺寸 ──────────────────────────────

  _sizeCanvas() {
    const rect = this.bodyEl.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    const w = Math.max(1, Math.floor(rect.width));
    const h = Math.max(1, Math.floor(rect.height));
    this._cssW = w;
    this._cssH = h;
    this.canvas.width = Math.round(w * dpr);
    this.canvas.height = Math.round(h * dpr);
    this.canvas.style.width = w + 'px';
    this.canvas.style.height = h + 'px';
    this.ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    this._sizeDirty = false;
  }

  // ────────────────────────────── 布局 ──────────────────────────────

  /**
   * 推导谱系并排好列。真正的算法在模块级纯函数 buildLineage() 里，
   * 这里只负责把 world 的当前状态喂给它。
   */
  _rebuildLayout() {
    this._layout = buildLineage(this.world.species.list, this.world.tick, MAX_NODES);
  }

  // ────────────────────────────── 绘制 ──────────────────────────────

  refresh() {
    if (!this.isOpen) return;
    if (this._sizeDirty) this._sizeCanvas();
    this._rebuildLayout();
    this._draw();
    this._renderDetail();
    this.refreshSubtitle();
  }

  _yOf(tick) {
    const { t0, t1 } = this._layout;
    const usable = this._cssH - PAD_TOP - PAD_BOTTOM;
    return PAD_TOP + ((tick - t0) / (t1 - t0)) * usable;
  }

  _xOf(col) {
    const n = Math.max(1, this._layout.order.length);
    const usable = this._cssW - PAD_LEFT - PAD_RIGHT;
    return PAD_LEFT + ((col + 0.5) / n) * usable;
  }

  _draw() {
    const ctx = this.ctx;
    const L = this._layout;
    const W = this._cssW;
    const H = this._cssH;

    ctx.clearRect(0, 0, W, H);
    ctx.fillStyle = '#0d1117';
    ctx.fillRect(0, 0, W, H);

    // ── 时间网格 ──
    ctx.font = '10px ui-monospace, SFMono-Regular, Menlo, monospace';
    ctx.textAlign = 'right';
    ctx.textBaseline = 'middle';
    const steps = 6;
    for (let i = 0; i <= steps; i++) {
      const tick = L.t0 + ((L.t1 - L.t0) * i) / steps;
      const y = Math.round(this._yOf(tick)) + 0.5;
      ctx.strokeStyle = 'rgba(255,255,255,0.07)';
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(PAD_LEFT, y);
      ctx.lineTo(W - PAD_RIGHT, y);
      ctx.stroke();

      ctx.fillStyle = '#6b7280';
      ctx.fillText(String(Math.round(tick)), PAD_LEFT - 8, y);
    }

    const n = L.order.length;
    if (n === 0) {
      ctx.fillStyle = '#6b7280';
      ctx.textAlign = 'center';
      ctx.fillText('还没有物种记录', W / 2, H / 2);
      this._hit = [];
      return;
    }

    const colW = (W - PAD_LEFT - PAD_RIGHT) / n;
    const barW = Math.max(1.5, Math.min(5, colW * 0.62));

    // ── 连线（先画，压在竖条下面）──
    ctx.strokeStyle = 'rgba(148,163,184,0.34)';
    ctx.lineWidth = 1;
    ctx.beginPath();
    for (const sp of L.order) {
      const pc = L.parentColumn.get(sp.id);
      if (pc === undefined) continue;
      const y = this._yOf(sp.startTick);
      ctx.moveTo(this._xOf(pc), y);
      ctx.lineTo(this._xOf(L.columnOf.get(sp.id)), y);
    }
    ctx.stroke();

    // ── 竖条 ──
    this._hit = [];
    for (const sp of L.order) {
      const col = L.columnOf.get(sp.id);
      const x = this._xOf(col);
      const y0 = this._yOf(sp.startTick);
      const y1 = this._yOf(sp.endTick >= 0 ? sp.endTick : L.now);
      const alive = L.aliveIds.has(sp.id);
      const isSel = this.selected === sp;

      const color = alive ? speciesColor(sp.fingerprint) : '#4b5563';
      ctx.fillStyle = color;
      ctx.globalAlpha = alive ? 0.92 : 0.5;
      ctx.fillRect(x - barW / 2, y0, barW, Math.max(1.5, y1 - y0));
      ctx.globalAlpha = 1;

      // 诞生点
      ctx.beginPath();
      ctx.arc(x, y0, Math.max(1.4, barW * 0.45), 0, Math.PI * 2);
      ctx.fill();

      // 存活物种在"现在"画一个亮点，表示仍在延续
      if (alive) {
        ctx.fillStyle = '#e5e7eb';
        ctx.beginPath();
        ctx.arc(x, y1, Math.max(1.2, barW * 0.4), 0, Math.PI * 2);
        ctx.fill();
      }

      if (isSel) {
        ctx.strokeStyle = '#f8fafc';
        ctx.lineWidth = 1.5;
        ctx.strokeRect(x - barW / 2 - 2, y0 - 2, barW + 4, Math.max(1.5, y1 - y0) + 4);
      }

      this._hit.push({ sp, x, y0, y1 });
    }

    // ── "现在"标注 ──
    const yNow = this._yOf(L.now);
    ctx.strokeStyle = 'rgba(56,189,248,0.55)';
    ctx.setLineDash([4, 4]);
    ctx.beginPath();
    ctx.moveTo(PAD_LEFT, yNow);
    ctx.lineTo(W - PAD_RIGHT, yNow);
    ctx.stroke();
    ctx.setLineDash([]);
    ctx.fillStyle = '#38bdf8';
    ctx.textAlign = 'left';
    ctx.fillText(`现在 tick ${L.now}`, PAD_LEFT + 2, Math.min(H - 8, yNow + 10));
  }

  // ────────────────────────────── 交互 ──────────────────────────────

  _onClick(event) {
    if (!this._layout) return;
    const rect = this.canvas.getBoundingClientRect();
    const mx = event.clientX - rect.left;
    const my = event.clientY - rect.top;

    let best = null;
    let bestDx = Infinity;
    for (const h of this._hit) {
      const dx = Math.abs(mx - h.x);
      if (dx > 6 || dx >= bestDx) continue;
      if (my < h.y0 - 4 || my > h.y1 + 4) continue;
      best = h;
      bestDx = dx;
    }

    this.selected = best ? best.sp : null;
    this._draw();
    this._renderDetail();
    if (this.onSelect) this.onSelect(this.selected);
  }

  _renderDetail() {
    const sp = this.selected;
    if (!sp) {
      this.detailEl.textContent = '点击任意一根竖条查看该物种；点击后世界画布上会高亮它的存活个体。';
      this.detailEl.classList.remove('is-active');
      return;
    }
    this.detailEl.classList.add('is-active');
    const end = sp.endTick >= 0 ? sp.endTick : this.world.tick;
    const parts = [
      `<b>${sp.name}</b>`,
      `指纹 ${sp.fingerprint}`,
      `形态 ${sp.description || '—'}`,
      `存续 ${sp.startTick} → ${sp.endTick >= 0 ? sp.endTick : '现在'}（${end - sp.startTick} tick）`,
      `当前 ${sp.population} 只`,
      `峰值 ${sp.peakPopulation}`,
      `累计 ${sp.cumulativePop}`,
      sp.alive ? '存活中' : '已灭绝',
    ];
    this.detailEl.innerHTML = parts.join('　·　');
  }

  /** 选中物种的存活个体（供主循环高亮用） */
  selectedFingerprint() {
    return this.selected ? this.selected.fingerprint : null;
  }

  refreshSubtitle() {
    const reg = this.world.species;
    this.subtitleEl.textContent =
      `存活 ${reg.aliveCount} · 累计 ${reg.totalCreated} · 峰值存活 ${reg.peakAliveCount}`
      + ` · 已灭绝 ${reg.totalExtinct}`;
  }
}
