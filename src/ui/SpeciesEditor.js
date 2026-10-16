import { CellType, LIVING_CELL_TYPES, cellDef, isDirectional, DIR_VECTORS, DIR } from '../life/CellType.js';
import { Anatomy } from '../life/Anatomy.js';
import { saveText, describeSaveTarget, hostSaveDir } from '../io/Export.js';
import { enableDrag } from './floating.js';

/**
 * 物种工坊 —— 手绘蓝图 + 生物库。
 *
 * 左边是一块 15×15 的小网格：点格子放细胞，右键/橡皮擦抹掉，
 * 放眼睛时可以选朝向。画好之后可以直接投放到世界，或者存进生物库。
 *
 * 右边是生物库：从检视器"保存生物"存下来的个体会出现在这里，
 * 也可以把手绘的存进去；支持导出/导入文件，方便分享或备份。
 *
 * 蓝图的坐标就是 Anatomy 的局部坐标：中心 (0,0) 是旋转枢轴，必须保留。
 */

const RADIUS = 7;                 // 15 × 15
const CELL = 26;                  // 每格像素

/** 画布上"朝向细缝"用的方向名 */
const DIR_NAMES = ['上', '右', '下', '左'];

export class SpeciesEditor {
  constructor(world, library, colorScheme) {
    this.world = world;
    this.library = library;
    this.colorScheme = colorScheme;

    /** "dx,dy" → { type, dir } */
    this.cells = new Map();
    this.brush = CellType.MOUTH;
    this.brushDir = 0;
    this.eraseMode = false;
    this.name = '';

    /** 投放请求：main.js 负责真正把它放进世界（它知道视口中心在哪） */
    this.onSpawn = null;
    /** 手动放置请求：(blueprint, name) => void —— main.js 把它装进放置笔刷 */
    this.onPlace = null;
    /** 提示条 */
    this.onToast = null;
    /** 库变化（增删改）后的回调，UI 用来刷新别处 */
    this.onLibraryChanged = null;
    /** 开合变化（顶栏按钮同步高亮用） */
    this.onOpenChanged = null;

    this._paintType = null;

    this._build();
    this._seedDefault();
  }

  get isOpen() {
    return !this.root.hidden;
  }

  // ────────────────────────────── DOM ──────────────────────────────

  _build() {
    const root = document.createElement('div');
    root.className = 'species-editor';
    root.hidden = true;

    const panel = document.createElement('div');
    panel.className = 'species-editor-panel';

    // ── 头部 ──
    const head = document.createElement('header');
    head.className = 'species-editor-head';
    const titles = document.createElement('div');
    titles.className = 'species-editor-titles';
    const title = document.createElement('div');
    title.className = 'species-editor-title';
    title.textContent = '物种工坊';
    const sub = document.createElement('div');
    sub.className = 'species-editor-subtitle';
    sub.textContent = '点格子放细胞 · 右键擦除 · 「手动放置」后点画布放到任意位置';
    titles.appendChild(title);
    titles.appendChild(sub);

    const close = document.createElement('button');
    close.className = 'species-editor-close';
    close.textContent = '×';
    close.title = '关闭（Esc）';
    close.addEventListener('click', () => this.close());

    head.appendChild(titles);
    head.appendChild(close);

    // ── 主体：左编辑 / 右库 ──
    const body = document.createElement('div');
    body.className = 'species-editor-body';

    body.appendChild(this._buildEditor());
    body.appendChild(this._buildLibrary());

    panel.appendChild(head);
    panel.appendChild(body);
    root.appendChild(panel);
    document.body.appendChild(root);

    this.root = root;
    this.panel = panel;
    this.ctx = this.canvas.getContext('2d');

    enableDrag(head, panel);
  }

  _buildEditor() {
    const col = document.createElement('div');
    col.className = 'species-editor-col';

    col.appendChild(sectionTitle('细胞类型'));
    const palette = document.createElement('div');
    palette.className = 'se-palette';
    this._paletteBtns = [];

    const types = [CellType.EMPTY, ...LIVING_CELL_TYPES];
    for (const type of types) {
      const d = cellDef(type);
      const btn = document.createElement('button');
      btn.className = 'se-swatch';
      btn.type = 'button';
      btn.title = type === CellType.EMPTY ? '橡皮擦：抹掉这一格' : d.label;

      const dot = document.createElement('span');
      dot.className = 'se-swatch-dot';
      dot.style.background = this.colorScheme.colorOf(type);
      const text = document.createElement('span');
      text.className = 'se-swatch-label';
      text.textContent = type === CellType.EMPTY ? '擦除' : d.label;

      btn.appendChild(dot);
      btn.appendChild(text);
      btn.addEventListener('click', () => {
        if (type === CellType.EMPTY) this.eraseMode = true;
        else { this.eraseMode = false; this.brush = type; }
        this._syncPalette();
      });
      this._paletteBtns.push({ type, btn });
      palette.appendChild(btn);
    }
    col.appendChild(palette);

    // 眼睛朝向
    this.dirRow = document.createElement('div');
    this.dirRow.className = 'se-dir-row';
    const dirLabel = document.createElement('span');
    dirLabel.className = 'se-dir-label';
    dirLabel.textContent = '眼睛朝向';
    this.dirRow.appendChild(dirLabel);
    this._dirBtns = [];
    for (let d = 0; d < 4; d++) {
      const b = document.createElement('button');
      b.className = 'se-dir-btn';
      b.type = 'button';
      b.textContent = DIR_NAMES[d];
      b.addEventListener('click', () => { this.brushDir = d; this._syncDir(); });
      this._dirBtns.push(b);
      this.dirRow.appendChild(b);
    }
    col.appendChild(this.dirRow);

    col.appendChild(sectionTitle('蓝图'));
    this.canvas = document.createElement('canvas');
    this.canvas.className = 'se-canvas';
    this.canvas.width = (RADIUS * 2 + 1) * CELL;
    this.canvas.height = (RADIUS * 2 + 1) * CELL;
    this.canvas.addEventListener('pointerdown', (e) => this._onCanvasDown(e));
    this.canvas.addEventListener('pointermove', (e) => this._onCanvasMove(e));
    this.canvas.addEventListener('pointerup', () => { this._paintType = null; });
    this.canvas.addEventListener('pointerleave', () => { this._paintType = null; this._hover = null; this._draw(); });
    this.canvas.addEventListener('contextmenu', (e) => e.preventDefault());
    col.appendChild(this.canvas);

    this.infoEl = document.createElement('div');
    this.infoEl.className = 'se-info';
    col.appendChild(this.infoEl);

    const tools = document.createElement('div');
    tools.className = 'se-tools';
    tools.appendChild(button('清空', '只留中心格', () => { this._clear(); }));
    tools.appendChild(button('随机生成', '随机长出一个能活的小生物', () => { this._randomize(); }));
    tools.appendChild(button('镜像', '把左半边镜像到右半边，做出对称身体', () => { this._mirror(); }));
    col.appendChild(tools);

    return col;
  }

  _buildLibrary() {
    const col = document.createElement('div');
    col.className = 'species-editor-col se-col-right';

    col.appendChild(sectionTitle('投放 / 保存'));
    const nameRow = document.createElement('div');
    nameRow.className = 'se-name-row';
    this.nameInput = document.createElement('input');
    this.nameInput.className = 'se-name-input';
    this.nameInput.type = 'text';
    this.nameInput.placeholder = '给这个生物起个名字';
    this.nameInput.addEventListener('input', () => { this.name = this.nameInput.value; });
    // 输入框里按 Esc 也关掉面板（否则主快捷键因为"正在输入"被拦下）
    this.nameInput.addEventListener('keydown', (e) => {
      if (e.key === 'Escape') { this.nameInput.blur(); this.close(); }
    });
    nameRow.appendChild(this.nameInput);
    col.appendChild(nameRow);

    const actions = document.createElement('div');
    actions.className = 'se-tools';
    actions.appendChild(button('手动放置', '装进放置笔刷，然后在画布上点哪里放哪里（Esc 取消）',
      () => this._placeManual()));
    actions.appendChild(button('放到视野中心', '不用点画布，直接放在当前视野中心', () => this._spawnCentered()));
    actions.appendChild(button('存入生物库', '连同当前蓝图一起保存', () => this._saveToLibrary()));
    col.appendChild(actions);

    col.appendChild(sectionTitle('生物库'));
    this.libraryEl = document.createElement('div');
    this.libraryEl.className = 'se-library';
    col.appendChild(this.libraryEl);

    const libTools = document.createElement('div');
    libTools.className = 'se-tools';
    libTools.appendChild(button('导出文件', '把整个生物库存成 JSON 文件', () => this._exportLibrary()));
    libTools.appendChild(button('导入文件', '从 JSON 文件合并进生物库', () => this.fileInput.click()));
    libTools.appendChild(button('清空库', '删掉生物库里所有生物', () => this._clearLibrary()));
    col.appendChild(libTools);

    this.fileInput = document.createElement('input');
    this.fileInput.type = 'file';
    this.fileInput.accept = 'application/json,.json';
    this.fileInput.hidden = true;
    this.fileInput.addEventListener('change', () => this._importLibrary());
    col.appendChild(this.fileInput);

    this.statusEl = document.createElement('div');
    this.statusEl.className = 'se-status';
    col.appendChild(this.statusEl);

    return col;
  }

  // ────────────────────────────── 开 / 关 ──────────────────────────────

  open() {
    this.root.hidden = false;
    this._renderLibrary();
    this._draw();
    this._syncPalette();
    this._syncDir();
    this._refreshInfo();
    this._refreshStatus();
    if (this.onOpenChanged) this.onOpenChanged(true);
  }

  close() {
    if (this.root.hidden) return;
    this.root.hidden = true;
    if (this.onOpenChanged) this.onOpenChanged(false);
  }

  toggle() {
    if (this.isOpen) this.close();
    else this.open();
  }

  // ────────────────────────────── 蓝图编辑 ──────────────────────────────

  _seedDefault() {
    this._clear(false);
    this.cells.set('0,0', { type: CellType.MOUTH, dir: 0 });
    this.cells.set('1,1', { type: CellType.PRODUCER, dir: 0 });
    this.cells.set('-1,-1', { type: CellType.PRODUCER, dir: 0 });
    this._refreshInfo();
  }

  _clear(redraw = true) {
    this.cells.clear();
    this.cells.set('0,0', { type: CellType.MOUTH, dir: 0 });
    if (redraw) { this._draw(); this._refreshInfo(); }
  }

  _setCell(dx, dy, type, dir) {
    if (dx === 0 && dy === 0) {
      // 中心格是枢轴，永远保留（可以换类型，但不能没有）
      this.cells.set('0,0', { type, dir });
      return;
    }
    this.cells.set(`${dx},${dy}`, { type, dir });
  }

  _randomize() {
    const rng = this.world.rng;
    this.cells.clear();
    this.cells.set('0,0', {
      type: rng.chance(50) ? CellType.MOUTH : CellType.PRODUCER, dir: 0,
    });

    const growth = rng.intRange(2, 7);
    for (let i = 0; i < growth; i++) {
      const keys = [...this.cells.keys()];
      const [bx, by] = keys[rng.int(keys.length)].split(',').map(Number);
      const n = DIR_VECTORS[rng.int(4)];
      const dx = bx + n[0];
      const dy = by + n[1];
      if (Math.abs(dx) > RADIUS || Math.abs(dy) > RADIUS) continue;
      if (this.cells.has(`${dx},${dy}`)) continue;
      const type = rng.pick(LIVING_CELL_TYPES);
      this.cells.set(`${dx},${dy}`, { type, dir: isDirectional(type) ? rng.int(4) : 0 });
    }
    this._draw();
    this._refreshInfo();
  }

  /** 把 dx>0 的部分镜像到 dx<0，做出左右对称的身体 */
  _mirror() {
    const adds = [];
    for (const [key, c] of this.cells) {
      const [dx, dy] = key.split(',').map(Number);
      if (dx <= 0) continue;
      adds.push([-dx, dy, c]);
    }
    for (const [dx, dy, c] of adds) {
      const dir = isDirectional(c.type) ? (c.dir === DIR.RIGHT ? DIR.LEFT : c.dir === DIR.LEFT ? DIR.RIGHT : c.dir) : 0;
      this.cells.set(`${dx},${dy}`, { type: c.type, dir });
    }
    this._draw();
    this._refreshInfo();
  }

  _toAnatomy() {
    const anatomy = new Anatomy();
    // 先加中心，保证枢轴存在
    const ordered = [...this.cells.entries()].sort((a, b) => {
      const A = a[0] === '0,0' ? -1 : 0;
      const B = b[0] === '0,0' ? -1 : 0;
      return A - B;
    });
    for (const [key, c] of ordered) {
      const [dx, dy] = key.split(',').map(Number);
      anatomy.addCell(c.type, dx, dy, c.dir);
    }
    anatomy.recompute();
    return anatomy;
  }

  _toBlueprint() {
    const cells = [];
    for (const [key, c] of this.cells) {
      const [dx, dy] = key.split(',').map(Number);
      cells.push({ type: c.type, dx, dy, dir: c.dir });
    }
    return { cells, label: this._toAnatomy().describe(), brain: null };
  }

  // ────────────────────────────── 画布 ──────────────────────────────

  _cellFromEvent(e) {
    const rect = this.canvas.getBoundingClientRect();
    const sx = (e.clientX - rect.left) / (rect.width / this.canvas.width);
    const sy = (e.clientY - rect.top) / (rect.height / this.canvas.height);
    const dx = Math.floor(sx / CELL) - RADIUS;
    const dy = Math.floor(sy / CELL) - RADIUS;
    if (dx < -RADIUS || dx > RADIUS || dy < -RADIUS || dy > RADIUS) return null;
    return [dx, dy];
  }

  _onCanvasDown(e) {
    const c = this._cellFromEvent(e);
    if (!c) return;
    const erase = this.eraseMode || e.button === 2;
    this._paintType = erase ? CellType.EMPTY : this.brush;
    this._applyPaint(c[0], c[1]);
    this.canvas.setPointerCapture(e.pointerId);
    e.preventDefault();
  }

  _onCanvasMove(e) {
    const c = this._cellFromEvent(e);
    if (!c) return;
    this._hover = c;
    if (this._paintType !== null) this._applyPaint(c[0], c[1]);
    else this._draw();
  }

  _applyPaint(dx, dy) {
    if (this._paintType === CellType.EMPTY) {
      if (dx === 0 && dy === 0) { this._draw(); return; }   // 枢轴不给擦
      this.cells.delete(`${dx},${dy}`);
    } else {
      const type = this._paintType;
      this._setCell(dx, dy, type, isDirectional(type) ? this.brushDir : 0);
    }
    this._draw();
    this._refreshInfo();
  }

  _draw() {
    const ctx = this.ctx;
    const W = this.canvas.width;
    const H = this.canvas.height;

    ctx.fillStyle = '#0b1020';
    ctx.fillRect(0, 0, W, H);

    // 网格线
    ctx.strokeStyle = 'rgba(255,255,255,0.07)';
    ctx.lineWidth = 1;
    for (let i = 0; i <= RADIUS * 2 + 1; i++) {
      const p = i * CELL + 0.5;
      ctx.beginPath(); ctx.moveTo(p, 0); ctx.lineTo(p, H); ctx.stroke();
      ctx.beginPath(); ctx.moveTo(0, p); ctx.lineTo(W, p); ctx.stroke();
    }

    // 枢轴高亮
    ctx.strokeStyle = 'rgba(255,255,255,0.22)';
    ctx.strokeRect(RADIUS * CELL + 0.5, RADIUS * CELL + 0.5, CELL - 1, CELL - 1);

    // 细胞
    for (const [key, c] of this.cells) {
      const [dx, dy] = key.split(',').map(Number);
      const x = (dx + RADIUS) * CELL;
      const y = (dy + RADIUS) * CELL;
      ctx.fillStyle = this.colorScheme.colorOf(c.type);
      ctx.fillRect(x + 1, y + 1, CELL - 2, CELL - 2);

      if (isDirectional(c.type)) {
        const v = DIR_VECTORS[c.dir & 3];
        const mx = x + CELL / 2;
        const my = y + CELL / 2;
        ctx.strokeStyle = 'rgba(255,255,255,0.92)';
        ctx.lineWidth = 2;
        ctx.beginPath();
        ctx.moveTo(mx, my);
        ctx.lineTo(mx + v[0] * CELL * 0.34, my + v[1] * CELL * 0.34);
        ctx.stroke();
      }
    }

    // 悬停框
    if (this._hover) {
      const [dx, dy] = this._hover;
      ctx.strokeStyle = 'rgba(56,189,248,0.9)';
      ctx.lineWidth = 2;
      ctx.strokeRect((dx + RADIUS) * CELL + 1, (dy + RADIUS) * CELL + 1, CELL - 2, CELL - 2);
    }
  }

  // ────────────────────────────── 信息与同步 ──────────────────────────────

  _refreshInfo() {
    if (!this.infoEl) return;
    const anatomy = this._toAnatomy();
    this.infoEl.textContent = `${anatomy.cells.length} 个细胞 · ${anatomy.describe()} · 指纹 ${anatomy.fingerprint()}`;
  }

  _syncPalette() {
    for (const { type, btn } of this._paletteBtns) {
      const active = this.eraseMode ? type === CellType.EMPTY : (!this.eraseMode && type === this.brush);
      btn.classList.toggle('is-active', active);
    }
    this.dirRow.hidden = this.eraseMode || !isDirectional(this.brush);
  }

  _syncDir() {
    for (let d = 0; d < 4; d++) this._dirBtns[d].classList.toggle('is-active', d === this.brushDir);
  }

  // ────────────────────────────── 投放 / 保存 ──────────────────────────────

  /** 手动放置：把当前蓝图交给主程序装进放置笔刷，之后点画布放哪里都行 */
  _placeManual() {
    const blueprint = this._toBlueprint();
    if (!blueprint.cells.length) return;
    if (this.onPlace) this.onPlace(blueprint, this.name);
  }

  /** 快捷方式：直接放在当前视野中心 */
  _spawnCentered() {
    const blueprint = this._toBlueprint();
    if (!blueprint.cells.length) return;
    if (this.onSpawn) this.onSpawn(blueprint, this.name);
  }

  _saveToLibrary() {
    const blueprint = this._toBlueprint();
    const entry = this.library.add(blueprint, this.name);
    if (!entry) return;
    this._renderLibrary();
    this._refreshStatus();
    if (this.onLibraryChanged) this.onLibraryChanged();
    if (this.onToast) this.onToast(`已存入生物库：${entry.name}（${blueprint.label}）`);
  }

  _loadEntry(entry) {
    // 走 library.blueprintOf 而不是直接读 entry.cells——库里存的是紧凑数组
    // （[type,dx,dy,dir]），这里要的是 {type,dx,dy,dir} 对象。
    const blueprint = this.library.blueprintOf(entry);
    this.cells.clear();
    for (const c of blueprint.cells) {
      this.cells.set(`${c.dx},${c.dy}`, { type: c.type, dir: c.dir });
    }
    if (!this.cells.has('0,0')) this.cells.set('0,0', { type: CellType.MOUTH, dir: 0 });
    this.name = entry.name;
    this.nameInput.value = entry.name;
    this._draw();
    this._refreshInfo();
  }

  /**
   * 库条目的"放置"：**手动放置**——把这条记录的蓝图装进放置笔刷，
   * 然后在画布上点哪里放哪里。加载过的物种走的是同一条路（蓝图在编辑器里，
   * 点左侧的"手动放置"即可），所以两种来源的体验是一致的。
   */
  _placeEntry(entry) {
    const blueprint = this.library.blueprintOf(entry);
    if (this.onPlace) this.onPlace(blueprint, entry.name);
  }

  /** 库条目的快捷方式：直接放到视野中心 */
  _centerEntry(entry) {
    const blueprint = this.library.blueprintOf(entry);
    if (this.onSpawn) this.onSpawn(blueprint, entry.name);
  }

  _clearLibrary() {
    if (!this.library.count) return;
    if (!window.confirm(`确定要清空生物库里的 ${this.library.count} 条记录吗？`)) return;
    this.library.clear();
    this._renderLibrary();
    this._refreshStatus();
    if (this.onLibraryChanged) this.onLibraryChanged();
  }

  async _exportLibrary() {
    if (!this.library.count) {
      if (this.onToast) this.onToast('生物库是空的，没有可导出的东西');
      return;
    }
    const data = JSON.stringify(this.library.toJSON(), null, 2);
    const filename = `evosim-creatures-${this.library.count}.json`;
    // 桌面版直接写进程序目录；浏览器版退回下载
    const result = await saveText(data, filename, { mime: 'application/json', bom: false });
    if (this.onToast) {
      this.onToast(`已导出 ${this.library.count} 条生物记录 · ${describeSaveTarget(result)}`);
    }
  }

  async _importLibrary() {
    const file = this.fileInput.files && this.fileInput.files[0];
    this.fileInput.value = '';
    if (!file) return;
    try {
      const text = await file.text();
      const added = this.library.merge(JSON.parse(text));
      this._renderLibrary();
      this._refreshStatus();
      if (this.onLibraryChanged) this.onLibraryChanged();
      if (this.onToast) this.onToast(`已从 ${file.name} 导入 ${added} 条生物`);
    } catch (err) {
      if (this.onToast) this.onToast(`导入失败：${err.message}`, true);
    }
  }

  _renderLibrary() {
    if (!this.libraryEl) return;
    this.libraryEl.innerHTML = '';

    if (!this.library.count) {
      const empty = document.createElement('div');
      empty.className = 'se-empty';
      empty.textContent = '还没有保存的生物。用「检视」工具点开一个个体，点「保存生物」即可收进这里。';
      this.libraryEl.appendChild(empty);
      return;
    }

    for (const entry of this.library.entries) {
      const row = document.createElement('div');
      row.className = 'se-entry';

      const info = document.createElement('div');
      info.className = 'se-entry-info';
      const nm = document.createElement('div');
      nm.className = 'se-entry-name';
      nm.textContent = entry.name;
      const meta = document.createElement('div');
      meta.className = 'se-entry-meta';
      const n = entry.cells.length;
      meta.textContent = `${n} 细胞${entry.label ? ' · ' + entry.label : ''}${entry.brain ? ' · 带大脑' : ''}`;
      info.appendChild(nm);
      info.appendChild(meta);
      row.appendChild(info);

      const btns = document.createElement('div');
      btns.className = 'se-entry-btns';
      btns.appendChild(button('放置', '装进放置笔刷，然后在画布上点哪里放哪里', () => this._placeEntry(entry)));
      btns.appendChild(button('居中', '直接放到当前视野中心', () => this._centerEntry(entry)));
      btns.appendChild(button('编辑', '载入到左边的蓝图', () => this._loadEntry(entry)));
      btns.appendChild(button('删除', '从生物库里移除', () => {
        this.library.remove(entry.id);
        this._renderLibrary();
        this._refreshStatus();
        if (this.onLibraryChanged) this.onLibraryChanged();
      }));
      row.appendChild(btns);

      this.libraryEl.appendChild(row);
    }
  }

  /** 外部往库里加了东西（例如检视器里的"保存生物"）之后调用 */
  refreshLibrary() {
    this._renderLibrary();
    this._refreshStatus();
  }

  _refreshStatus() {
    if (!this.statusEl) return;
    const parts = [`生物库 ${this.library.count} 条`];
    if (!this.library.persistent) parts.push('当前环境不支持本地存储，库只在本次会话有效');
    const dir = hostSaveDir();
    parts.push(dir ? `导出保存到程序目录：${dir}` : '导出走浏览器下载');
    this.statusEl.textContent = parts.join(' · ');
  }
}

function button(label, title, onClick) {
  const b = document.createElement('button');
  b.className = 'se-btn';
  b.type = 'button';
  b.textContent = label;
  b.title = title;
  b.addEventListener('click', onClick);
  return b;
}

function sectionTitle(text) {
  const h = document.createElement('h3');
  h.className = 'se-section-title';
  h.textContent = text;
  return h;
}
