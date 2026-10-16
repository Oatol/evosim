import { CellType, DIR_VECTORS } from '../life/CellType.js';
import { rotateCol, rotateRow } from '../life/BodyCell.js';
import { INPUT_LABELS, DIR_LABELS, SEEN_TYPE_LABELS, VISION_DIRS, VISUAL_SLOTS, STATE_BASE } from '../life/Sensors.js';
import { enableDrag } from './floating.js';

/**
 * 个体检视器 —— 设计文档 §9.3。
 *
 * 这是整个项目里最能"看懂进化在做什么"的模块：
 *   左栏  身体蓝图（放大，带朝向）+ 个体属性
 *   右栏  神经网络连线图（40 → 12 → 5）+ 实时激活值
 *
 * 连线是静态的（生物的权重不会变），所以拆成"静态层 + 动态层"：
 * 静态层只在切换检视对象时重建一次，每帧只重画节点填充。
 * 540 条连线每帧重画会很浪费。
 */

/** 连线按权重绝对值分档——每档一个 globalAlpha，避免逐条设置状态 */
const LINK_TIERS = [
  { max: 0.10, alpha: 0.07, width: 0.8 },
  { max: 0.25, alpha: 0.16, width: 0.9 },
  { max: 0.50, alpha: 0.30, width: 1.3 },
  { max: Infinity, alpha: 0.55, width: 1.9 },
];

const POS_COLOR = '#2563eb';
const NEG_COLOR = '#dc2626';
const NODE_IDLE = '#dfe3e9';

export class Inspector {
  constructor(colorScheme, world) {
    this.colorScheme = colorScheme;
    this.world = world;
    this.organism = null;

    /** 相机是否跟随当前个体（由 main.js 在每帧消费） */
    this.following = false;
    this.onFollowChanged = null;

    /** 点"保存生物"时的回调：(organism) => void */
    this.onSaveCreature = null;

    /** 图钉：锁定时点画布不会把它关掉 */
    this.pinned = false;

    this._bodyW = 0;
    this._bodyH = 0;
    this._netW = 0;
    this._netH = 0;
    this._staticDirty = true;
    this._layout = null;

    this._build();
  }

  get isOpen() {
    return this.organism !== null;
  }

  // ────────────────────────────── DOM ──────────────────────────────

  _build() {
    const root = document.createElement('div');
    root.className = 'inspector';
    root.hidden = true;

    const panel = document.createElement('div');
    panel.className = 'inspector-panel';

    // ── 头部 ──
    const head = document.createElement('header');
    head.className = 'inspector-head';

    const titles = document.createElement('div');
    titles.className = 'inspector-titles';
    this.titleEl = document.createElement('div');
    this.titleEl.className = 'inspector-title';
    this.titleEl.textContent = '个体检视器';
    this.subtitleEl = document.createElement('div');
    this.subtitleEl.className = 'inspector-subtitle';
    titles.appendChild(this.titleEl);
    titles.appendChild(this.subtitleEl);

    const close = document.createElement('button');
    close.className = 'inspector-close';
    close.textContent = '×';
    close.title = '关闭（Esc）';
    close.addEventListener('click', () => this.close());

    // 图钉：锁定后点画布也不会关掉这个检视器，方便一边盯着一只生物一边操作世界
    this.pinBtn = document.createElement('button');
    this.pinBtn.className = 'inspector-pin';
    this.pinBtn.type = 'button';
    this.pinBtn.title = '锁定：锁定后点击画布不会关闭';
    this.pinBtn.innerHTML =
      '<svg viewBox="0 0 24 24" width="13" height="13" aria-hidden="true">' +
      '<path fill="currentColor" d="M16 9V4h1c.55 0 1-.45 1-1s-.45-1-1-1H7c-.55 0-1 .45-1 ' +
      '1s.45 1 1 1h1v5c0 1.66-1.34 3-3 3v2h5.97v7l1 1 1-1v-7H19v-2c-1.66 0-3-1.34-3-3z"/></svg>';
    this.pinBtn.addEventListener('click', () => this.setPinned(!this.pinned));

    // 保存生物：把当前个体（含大脑权重）收进生物库
    this.saveBtn = document.createElement('button');
    this.saveBtn.className = 'inspector-action';
    this.saveBtn.type = 'button';
    this.saveBtn.textContent = '保存生物';
    this.saveBtn.title = '把这个个体连同它的大脑存进生物库';
    this.saveBtn.addEventListener('click', () => {
      if (this.onSaveCreature && this.organism) this.onSaveCreature(this.organism);
    });

    // 跟随：让相机盯住这个个体。观察一个捕食者的行动轨迹时很有用。
    this.followBtn = document.createElement('button');
    this.followBtn.className = 'inspector-follow';
    this.followBtn.type = 'button';
    this.followBtn.textContent = '跟随';
    this.followBtn.title = '相机跟着这个个体移动';
    this.followBtn.addEventListener('click', () => this.setFollow(!this.following));

    head.appendChild(titles);
    head.appendChild(this.saveBtn);
    head.appendChild(this.followBtn);
    head.appendChild(this.pinBtn);
    head.appendChild(close);

    // ── 主体 ──
    const body = document.createElement('div');
    body.className = 'inspector-body';

    const left = document.createElement('div');
    left.className = 'inspector-col inspector-col-left';

    left.appendChild(sectionTitle('身体蓝图'));
    this.bodyCanvas = document.createElement('canvas');
    this.bodyCanvas.className = 'inspector-body-canvas';
    left.appendChild(this.bodyCanvas);

    left.appendChild(sectionTitle('属性'));
    this.attrList = document.createElement('dl');
    this.attrList.className = 'stat-list inspector-attrs';
    left.appendChild(this.attrList);

    const right = document.createElement('div');
    right.className = 'inspector-col inspector-col-right';

    const netTitle = sectionTitle('神经网络');
    const dim = document.createElement('span');
    dim.className = 'inspector-dim';
    dim.textContent = '40 → 12 → 5 · 557 权重';
    netTitle.appendChild(dim);
    right.appendChild(netTitle);

    this.netCanvas = document.createElement('canvas');
    this.netCanvas.className = 'inspector-net-canvas';
    right.appendChild(this.netCanvas);

    this.legendEl = document.createElement('div');
    this.legendEl.className = 'inspector-legend';
    this.legendEl.textContent = '节点颜色 = 激活值（蓝正 / 红负）· 连线粗细 = |权重| · 蓝框 = 网络当前选择';
    right.appendChild(this.legendEl);

    body.appendChild(left);
    body.appendChild(right);

    panel.appendChild(head);
    panel.appendChild(body);
    root.appendChild(panel);

    document.body.appendChild(root);

    this.root = root;
    this.panel = panel;
    this.bodyCtx = this.bodyCanvas.getContext('2d');
    this.netCtx = this.netCanvas.getContext('2d');

    this._attrEls = new Map();
    this._buildAttrs();

    this._enableDrag(head, panel);
    this._bindOutsideClose();
  }

  /** 拖头部移动浮窗（逻辑见 floating.js） */
  _enableDrag(handle, panel) {
    enableDrag(handle, panel);
  }

  /**
   * 未锁定时，点**世界画布**才关闭；点工具栏 / 顶栏 / 时间轴一律不关。
   *
   * 这一条是关键：早先"点任何地方都关"，导致点工具栏的"克隆选中"按钮时
   * 先把检视器关掉、选中被清空，于是永远提示"先用检视工具点一个个体"。
   */
  _bindOutsideClose() {
    document.addEventListener('pointerdown', (event) => {
      if (!this.isOpen || this.pinned) return;
      const t = event.target;
      if (!t || !t.closest) return;
      if (t.closest('.inspector-panel')) return;
      if (t.closest('.canvas-wrap')) this.close();
    }, true);
  }

  /** 图钉开关 */
  setPinned(on) {
    this.pinned = !!on;
    if (this.pinBtn) this.pinBtn.classList.toggle('is-active', this.pinned);
  }

  _buildAttrs() {
    const rows = [
      ['age', '年龄'],
      ['lifespan', '寿命'],
      ['energy', '能量'],
      ['foodNeeded', '繁殖阈值'],
      ['mutability', '变异率'],
      ['species', '物种'],
      ['ancestry', '祖先'],
      ['position', '坐标'],
      ['facing', '朝向'],
      ['state', '状态'],
    ];
    for (const [key, label] of rows) {
      const dt = document.createElement('dt');
      dt.textContent = label;
      const dd = document.createElement('dd');
      dd.textContent = '—';
      this.attrList.appendChild(dt);
      this.attrList.appendChild(dd);
      this._attrEls.set(key, dd);
    }
  }

  // ────────────────────────────── 开 / 关 ──────────────────────────────

  open(organism) {
    if (!organism) return;
    this.organism = organism;
    this.root.hidden = false;
    this._staticDirty = true;
    this._sizeCanvases();
    this.refresh();
  }

  /**
   * 关闭检视器。所有关闭路径（× 按钮 / 点背景 / Esc / 世界重建）都走这里，
   * 所以 onClose 放在这儿触发——否则会漏掉某条路径上的清理。
   */
  close() {
    if (this.organism === null) return;
    this.organism = null;
    this.root.hidden = true;
    this.setFollow(false);
    if (this.onClose) this.onClose();
  }

  /**
   * 跟随开关。
   *
   * 关掉时把 `following` 与按钮状态一起复位——个体死亡、检视器关闭、
   * 世界重建都要能自动解除跟随，否则相机会一直卡在一个不存在的坐标上。
   */
  setFollow(on) {
    this.following = !!on && !!this.organism && this.organism.alive;
    if (this.followBtn) this.followBtn.classList.toggle('is-active', this.following);
    if (this.onFollowChanged) this.onFollowChanged(this.following);
  }

  toggle(organism) {
    if (this.organism === organism) this.close();
    else this.open(organism);
  }

  // ────────────────────────────── 尺寸 ──────────────────────────────

  _sizeCanvases() {
    const dpr = window.devicePixelRatio || 1;

    const bodyRect = this.bodyCanvas.getBoundingClientRect();
    const bw = Math.max(1, Math.round(bodyRect.width));
    const bh = Math.max(1, Math.round(bodyRect.height));
    if (bw !== this._bodyW || bh !== this._bodyH || this._bodyDpr !== dpr) {
      this._bodyW = bw;
      this._bodyH = bh;
      this._bodyDpr = dpr;
      this.bodyCanvas.width = Math.round(bw * dpr);
      this.bodyCanvas.height = Math.round(bh * dpr);
      this.bodyCtx.setTransform(dpr, 0, 0, dpr, 0, 0);
      this._staticDirty = true;
    }

    const netRect = this.netCanvas.getBoundingClientRect();
    const nw = Math.max(1, Math.round(netRect.width));
    const nh = Math.max(1, Math.round(netRect.height));
    if (nw !== this._netW || nh !== this._netH || this._netDpr !== dpr) {
      this._netW = nw;
      this._netH = nh;
      this._netDpr = dpr;
      this.netCanvas.width = Math.round(nw * dpr);
      this.netCanvas.height = Math.round(nh * dpr);
      this.netCtx.setTransform(dpr, 0, 0, dpr, 0, 0);
      this._staticDirty = true;
    }
  }

  // ────────────────────────────── 刷新 ──────────────────────────────

  refresh() {
    const org = this.organism;
    if (!org) return;

    if (!org.alive) {
      // 个体死了就停在这一帧，让用户还能看清它生前的状态
      this.titleEl.textContent = '个体检视器 · 已死亡';
      if (this.following) this.setFollow(false);   // 跟着一个死掉的个体没有意义
    } else {
      this.titleEl.textContent = '个体检视器';
    }

    this._drawBody(org);
    this._drawNetwork(org);
    this._refreshAttrs(org);
  }

  _refreshAttrs(org) {
    const world = this.world;
    const set = (key, value) => {
      const el = this._attrEls.get(key);
      if (el) el.textContent = value;
    };

    set('age', `${org.age} / ${org.lifespan()}`);
    set('lifespan', String(org.lifespan()));
    set('energy', `${round1(org.energy)} / ${round1(org.reproduceAt())}`);
    set('foodNeeded', String(round1(org.reproduceAt())));
    set('mutability', org.mutability.toFixed(2) + '%');

    const sp = world.species.byFingerprint(org.speciesFingerprint);
    set('species', sp ? `${sp.name} · ${sp.description}` : '未知');

    // 祖先链：往上追三代就够看出谱系了，再长显示不下也没意义
    const chain = [];
    let cur = sp;
    for (let i = 0; i < 3 && cur; i++) {
      const parentId = cur.ancestorId;
      if (parentId === null) { chain.push('起源'); break; }
      const parent = world.species.list.find((s) => s.id === parentId);
      if (!parent) break;
      chain.push(parent.name);
      cur = parent;
    }
    set('ancestry', chain.length ? chain.join(' ← ') : '—');

    set('position', `${org.col}, ${org.row}`);
    set('facing', `${DIR_LABELS[org.rotation]}（身体）/ ${DIR_LABELS[org.direction]}（移动）`);
    let state = org.alive ? (org.anatomy.hasMover ? '存活 · 可移动' : '存活 · 静止') : '已死亡';
    if (org.anatomy.hasStorage) state += ` · 储备 ${round1(org.reproduceAt() - org.foodNeeded())}`;
    if (org.anatomy.hasSpike) state += ` · 反伤 ${round1(org.spikeDamage())}`;
    set('state', state);
  }

  // ────────────────────────────── 身体蓝图 ──────────────────────────────

  _drawBody(org) {
    const ctx = this.bodyCtx;
    const W = this._bodyW;
    const H = this._bodyH;
    if (W <= 0 || H <= 0) return;

    ctx.clearRect(0, 0, W, H);
    ctx.fillStyle = '#0b1020';
    ctx.fillRect(0, 0, W, H);

    const radius = Math.max(org.anatomy.radius, 1);
    const span = radius * 2 + 3;                       // 留一圈余量
    const cell = Math.max(4, Math.floor(Math.min(W, H) / span));
    const cx = W / 2;
    const cy = H / 2;

    const cells = org.anatomy.cells;
    for (let i = 0; i < cells.length; i++) {
      const c = cells[i];
      // 旋转后画——检视器显示的是生物**当前朝向**下的样子
      const rx = rotateCol(c.dx, c.dy, org.rotation);
      const ry = rotateRow(c.dx, c.dy, org.rotation);
      const x = cx + rx * cell - cell / 2;
      const y = cy + ry * cell - cell / 2;

      ctx.fillStyle = this.colorScheme.colorOf(c.type);
      ctx.fillRect(x, y, cell - 1, cell - 1);

      if (c.type === CellType.EYE) {
        // 眼睛朝向细缝
        const d = DIR_VECTORS[(org.rotation + c.dir) & 3];
        const mx = x + cell / 2;
        const my = y + cell / 2;
        ctx.strokeStyle = 'rgba(255,255,255,0.92)';
        ctx.lineWidth = Math.max(1, cell * 0.14);
        ctx.beginPath();
        ctx.moveTo(mx, my);
        ctx.lineTo(mx + d[0] * cell * 0.36, my + d[1] * cell * 0.36);
        ctx.stroke();
      }
    }

    // 中心枢轴标记
    ctx.strokeStyle = 'rgba(255,255,255,0.28)';
    ctx.lineWidth = 1;
    ctx.strokeRect(cx - cell / 2, cy - cell / 2, cell, cell);

    // 移动方向箭头（画在身体外面，表示"它想往哪走"）
    if (org.anatomy.hasMover) {
      const d = DIR_VECTORS[org.direction];
      const r = (radius + 1.4) * cell;
      ctx.strokeStyle = 'rgba(96,165,250,0.85)';
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.moveTo(cx + d[0] * r * 0.62, cy + d[1] * r * 0.62);
      ctx.lineTo(cx + d[0] * r, cy + d[1] * r);
      ctx.stroke();
    }
  }

  // ────────────────────────────── 神经网络 ──────────────────────────────

  _computeLayout() {
    const W = this._netW;
    const H = this._netH;
    const padLeft = 52;
    const padRight = 46;
    const padY = 14;

    const inX = padLeft;
    const outX = W - padRight;
    const hidX = padLeft + (outX - padLeft) * 0.52;

    const spanY = H - padY * 2;
    const yOf = (i, n) => padY + (n <= 1 ? spanY / 2 : (i / (n - 1)) * spanY);

    const IN = this.organism ? this.organism.brain.inSize : 40;
    const HID = this.organism ? this.organism.brain.hiddenSize : 12;
    const OUT = this.organism ? this.organism.brain.outSize : 5;

    const inY = new Float32Array(IN);
    for (let i = 0; i < IN; i++) inY[i] = yOf(i, IN);
    const hidY = new Float32Array(HID);
    for (let i = 0; i < HID; i++) hidY[i] = yOf(i, HID);
    const outY = new Float32Array(OUT);
    for (let i = 0; i < OUT; i++) outY[i] = yOf(i, OUT);

    return { W, H, inX, hidX, outX, inY, hidY, outY, IN, HID, OUT };
  }

  _drawNetwork(org) {
    const ctx = this.netCtx;
    if (this._netW <= 0 || this._netH <= 0) return;

    const L = this._layout || (this._layout = this._computeLayout());
    // 拓扑变了（比如运行中改过隐藏层尺寸）就重新布局
    if (L.IN !== org.brain.inSize || L.HID !== org.brain.hiddenSize || L.OUT !== org.brain.outSize) {
      this._layout = this._computeLayout();
      this._staticDirty = true;
    }
    const layout = this._layout;

    if (this._staticDirty) {
      this._drawStatic(org, layout);
      this._staticDirty = false;
    }

    ctx.clearRect(0, 0, layout.W, layout.H);
    ctx.drawImage(this._staticCanvas, 0, 0, layout.W, layout.H);

    const hidden = org.brain.hiddenActivations;
    const output = org.brain.outputActivations;
    const inputs = org.brain.lastInput;

    // 输出层归一化：用 max(|v|, 1) 而不是纯 max，这样"整体很弱"不会被放大成满色
    let outMax = 1;
    for (let k = 0; k < layout.OUT; k++) {
      const a = output[k] < 0 ? -output[k] : output[k];
      if (a > outMax) outMax = a;
    }

    // argmax 输出高亮
    let best = 0;
    for (let k = 1; k < layout.OUT; k++) if (output[k] > output[best]) best = k;

    // ── 输入节点 ──
    for (let i = 0; i < layout.IN; i++) {
      const v = inputs ? inputs[i] : 0;
      this._node(ctx, layout.inX, layout.inY[i], 3.4, v, 1, 0.55);
    }

    // ── 隐藏节点（tanh，范围 -1..1）──
    for (let j = 0; j < layout.HID; j++) {
      this._node(ctx, layout.hidX, layout.hidY[j], 5.5, hidden[j], 1, 0.85);
    }

    // ── 输出节点 ──
    for (let k = 0; k < layout.OUT; k++) {
      const r = k === best ? 9 : 7;
      this._node(ctx, layout.outX, layout.outY[k], r, output[k], outMax, 0.95);
      if (k === best) {
        ctx.strokeStyle = '#2563eb';
        ctx.lineWidth = 2;
        ctx.beginPath();
        ctx.arc(layout.outX, layout.outY[k], r + 3.5, 0, Math.PI * 2);
        ctx.stroke();
      }
    }

    // ── 输出标签 ──
    ctx.font = '10px ui-monospace, SFMono-Regular, Menlo, monospace';
    ctx.textAlign = 'right';
    ctx.textBaseline = 'middle';
    const outLabels = ['前', '右', '后', '左', '转身'];
    for (let k = 0; k < layout.OUT; k++) {
      ctx.fillStyle = k === best ? '#2563eb' : '#59616d';
      ctx.fillText(outLabels[k] || `o${k}`, layout.outX + 40, layout.outY[k]);
    }

    // ── 输出数值 ──
    ctx.textAlign = 'left';
    ctx.fillStyle = '#8b939f';
    ctx.font = '9px ui-monospace, SFMono-Regular, Menlo, monospace';
    for (let k = 0; k < layout.OUT; k++) {
      ctx.fillText(output[k].toFixed(2), layout.outX + 12, layout.outY[k]);
    }
  }

  /** 静态层：连线 + 分组标签。只在切换对象或尺寸变化时重建。 */
  _drawStatic(org, layout) {
    const dpr = window.devicePixelRatio || 1;
    if (!this._staticCanvas) this._staticCanvas = document.createElement('canvas');

    const c = this._staticCanvas;
    c.width = Math.round(layout.W * dpr);
    c.height = Math.round(layout.H * dpr);
    const ctx = c.getContext('2d');
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, layout.W, layout.H);

    const brain = org.brain;

    // 按 (符号, 档位) 分组批量描边——540 条线逐条设置状态太慢
    for (const sign of [1, -1]) {
      for (let tier = 0; tier < LINK_TIERS.length; tier++) {
        const t = LINK_TIERS[tier];
        ctx.beginPath();
        let any = false;

        for (let j = 0; j < layout.HID; j++) {
          for (let i = 0; i < layout.IN; i++) {
            const w = brain.w1[j * layout.IN + i];
            if (w === 0 || (w > 0 ? 1 : -1) !== sign) continue;
            const a = w < 0 ? -w : w;
            const tierOf = a < 0.10 ? 0 : a < 0.25 ? 1 : a < 0.50 ? 2 : 3;
            if (tierOf !== tier) continue;
            ctx.moveTo(layout.inX, layout.inY[i]);
            ctx.lineTo(layout.hidX, layout.hidY[j]);
            any = true;
          }
        }
        for (let k = 0; k < layout.OUT; k++) {
          for (let j = 0; j < layout.HID; j++) {
            const w = brain.w2[j * layout.OUT + k];
            if (w === 0 || (w > 0 ? 1 : -1) !== sign) continue;
            const a = w < 0 ? -w : w;
            const tierOf = a < 0.10 ? 0 : a < 0.25 ? 1 : a < 0.50 ? 2 : 3;
            if (tierOf !== tier) continue;
            ctx.moveTo(layout.hidX, layout.hidY[j]);
            ctx.lineTo(layout.outX, layout.outY[k]);
            any = true;
          }
        }

        if (!any) continue;
        ctx.globalAlpha = t.alpha;
        ctx.strokeStyle = sign > 0 ? POS_COLOR : NEG_COLOR;
        ctx.lineWidth = t.width;
        ctx.stroke();
      }
    }
    ctx.globalAlpha = 1;

    // 视觉输入的四个方向分组标签
    ctx.font = '9px ui-monospace, SFMono-Regular, Menlo, monospace';
    ctx.textAlign = 'right';
    ctx.textBaseline = 'middle';
    ctx.fillStyle = '#8b939f';
    for (let d = 0; d < VISION_DIRS; d++) {
      const first = d * VISUAL_SLOTS;
      const last = first + VISUAL_SLOTS - 1;
      const mid = (layout.inY[first] + layout.inY[last]) / 2;
      ctx.fillText(DIR_LABELS[d], layout.inX - 8, mid);
    }
    // 状态输入分组标签
    const stateMid = (layout.inY[STATE_BASE] + layout.inY[layout.IN - 1]) / 2;
    ctx.fillText('状态', layout.inX - 8, stateMid);

    // 分组分隔虚线
    ctx.strokeStyle = 'rgba(16,20,28,0.10)';
    ctx.lineWidth = 1;
    ctx.setLineDash([2, 3]);
    for (let d = 1; d <= VISION_DIRS; d++) {
      const y = (layout.inY[d * VISUAL_SLOTS - 1] + layout.inY[d * VISUAL_SLOTS]) / 2;
      if (layout.inY[d * VISUAL_SLOTS] === undefined) continue;
      ctx.beginPath();
      ctx.moveTo(layout.inX - 4, y);
      ctx.lineTo(layout.hidX - 8, y);
      ctx.stroke();
    }
    ctx.setLineDash([]);
  }

  /** 画一个激活值节点：正蓝负红，透明度表示强度 */
  _node(ctx, x, y, r, value, norm, baseAlpha) {
    const t = Math.max(-1, Math.min(1, value / (norm || 1)));
    const mag = t < 0 ? -t : t;

    ctx.beginPath();
    ctx.arc(x, y, r, 0, Math.PI * 2);
    ctx.fillStyle = NODE_IDLE;
    ctx.fill();

    if (mag > 0.02) {
      ctx.globalAlpha = baseAlpha * (0.25 + mag * 0.75);
      ctx.fillStyle = t > 0 ? POS_COLOR : NEG_COLOR;
      ctx.fill();
      ctx.globalAlpha = 1;
    }
  }

  /** 切换检视对象后需要重建静态层 */
  markDirty() {
    this._staticDirty = true;
    this._layout = null;
  }
}

function sectionTitle(text) {
  const h = document.createElement('h3');
  h.className = 'inspector-section-title';
  h.textContent = text;
  return h;
}

/** 能量这类量现在可能是小数（代谢/储备都会产生），显示成 1 位小数就够了 */
function round1(v) {
  return Math.round(v * 10) / 10;
}
