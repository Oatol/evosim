import { CellType, cellTypeLabel } from '../life/CellType.js';
import { BRUSH_ORGANISM, BRUSH_CLONE, BRUSH_ERASER } from './Editor.js';
import { seedOriginOfLife, spawnPopulation } from '../life/OrganismFactory.js';

/** 笔刷可选项：三种地形 + 橡皮擦 + 随机生物 + 克隆选中 */
const BRUSHES = [
  { id: CellType.EMPTY, label: '空' },
  { id: CellType.FOOD, label: '食物' },
  { id: CellType.WALL, label: '墙' },
  { id: BRUSH_ERASER, label: '橡皮擦' },
  { id: BRUSH_ORGANISM, label: '随机生物' },
  { id: BRUSH_CLONE, label: '克隆', needsBlueprint: true },
];

/**
 * 左侧工具栏：工具选择、笔刷、生命操作、视图控制。
 */
export class Toolbar {
  constructor(container, editor, camera, renderer, colorScheme, world) {
    this.container = container;
    this.editor = editor;
    this.camera = camera;
    this.renderer = renderer;
    this.colorScheme = colorScheme;
    this.world = world;

    /** 世界被改动后的回调（UI 用来刷新读数） */
    this.onWorldChanged = null;

    /** 请求"把当前检视的个体装进克隆笔刷" */
    this.onCloneSelected = null;

    /** 导出回调 */
    this.onExportFossil = null;
    this.onExportSpecies = null;

    this._toolButtons = [];
    this._swatches = [];

    this._build();
  }

  _section(title) {
    const section = document.createElement('section');
    section.className = 'tool-section';
    const h = document.createElement('h2');
    h.className = 'panel-title';
    h.textContent = title;
    section.appendChild(h);
    this.container.appendChild(section);
    return section;
  }

  _build() {
    this.container.innerHTML = '';

    this._buildTools();
    this._buildBrush();
    this._buildLife();
    this._buildExport();
    this._buildView();

    this._syncTools();
    this._syncSwatches();
  }

  _buildTools() {
    const section = this._section('工具');
    const row = document.createElement('div');
    row.className = 'tool-row';

    for (const tool of [
      { id: 'paint', label: '画笔' },
      { id: 'pan', label: '平移' },
      { id: 'inspect', label: '检视' },
    ]) {
      const btn = document.createElement('button');
      btn.className = 'tool-btn';
      btn.textContent = tool.label;
      btn.addEventListener('click', () => {
        this.editor.setTool(tool.id);
        this._syncTools();
      });
      this._toolButtons.push({ id: tool.id, btn });
      row.appendChild(btn);
    }
    section.appendChild(row);
  }

  _buildBrush() {
    const section = this._section('笔刷');
    const grid = document.createElement('div');
    grid.className = 'swatch-grid';

    for (const brush of BRUSHES) {
      const btn = document.createElement('button');
      btn.className = 'swatch';
      btn.title = brushTitle(brush);

      const dot = document.createElement('span');
      dot.className = 'swatch-dot';
      dot.style.background = this._brushColor(brush.id);

      const text = document.createElement('span');
      text.className = 'swatch-label';
      text.textContent = brush.label;

      btn.appendChild(dot);
      btn.appendChild(text);
      btn.addEventListener('click', () => {
        if (brush.needsBlueprint && !this.editor.cloneBlueprint) {
          // 没有蓝图就先去要一份——比给个灰按钮让人猜要友好
          if (this.onCloneSelected) this.onCloneSelected();
          if (!this.editor.cloneBlueprint) return;
        }
        this.editor.setTool('paint');
        this.editor.setBrushType(brush.id);
        this._syncTools();
        this._syncSwatches();
      });

      this._swatches.push({ id: brush.id, btn, dot, needsBlueprint: !!brush.needsBlueprint });
      grid.appendChild(btn);
    }
    section.appendChild(grid);

    // 克隆笔刷的状态提示：没装蓝图时说清怎么装
    this.brushHint = document.createElement('div');
    this.brushHint.className = 'panel-hint';
    section.appendChild(this.brushHint);

    const sizeRow = document.createElement('div');
    sizeRow.className = 'tool-row tool-row-size';

    const sizeLabel = document.createElement('span');
    sizeLabel.className = 'tool-label';
    sizeLabel.textContent = '大小';

    this.sizeValue = document.createElement('span');
    this.sizeValue.className = 'tool-value';

    this.sizeInput = document.createElement('input');
    this.sizeInput.type = 'range';
    this.sizeInput.min = '1';
    this.sizeInput.max = '12';
    this.sizeInput.step = '1';
    this.sizeInput.value = String(this.editor.brushSize);
    this.sizeInput.addEventListener('input', () => {
      this.editor.setBrushSize(Number(this.sizeInput.value));
      this.sizeValue.textContent = this.sizeInput.value;
    });

    sizeRow.appendChild(sizeLabel);
    sizeRow.appendChild(this.sizeInput);
    sizeRow.appendChild(this.sizeValue);
    section.appendChild(sizeRow);
    this.sizeRow = sizeRow;
    this.sizeValue.textContent = this.sizeInput.value;
  }

  _buildLife() {
    const section = this._section('生命');

    const mk = (row, label, title, fn) => {
      const b = document.createElement('button');
      b.className = 'tool-btn';
      b.textContent = label;
      b.title = title;
      b.addEventListener('click', fn);
      row.appendChild(b);
      return b;
    };

    const row = document.createElement('div');
    row.className = 'tool-row';
    mk(row, '生命起源', '在世界中央投放经典的三细胞生物', () => {
      seedOriginOfLife(this.world);
      this._notifyChanged();
    });
    mk(row, '随机种群', '撒下一片随机小生物', () => {
      spawnPopulation(this.world, 12);
      this._notifyChanged();
    });
    mk(row, '清空生物', '移除全部生物，身体转为食物', () => {
      this.world.clearOrganisms();
      this._notifyChanged();
    });
    section.appendChild(row);

    const row2 = document.createElement('div');
    row2.className = 'tool-row';
    mk(row2, '克隆选中', '把当前检视的个体装进克隆笔刷，之后可以随手投放它', () => {
      if (this.onCloneSelected) this.onCloneSelected();
    });
    section.appendChild(row2);
  }

  _buildExport() {
    const section = this._section('导出');

    const mk = (label, title, fn) => {
      const b = document.createElement('button');
      b.className = 'tool-btn';
      b.textContent = label;
      b.title = title;
      b.addEventListener('click', fn);
      row.appendChild(b);
      return b;
    };

    const row = document.createElement('div');
    row.className = 'tool-row';
    mk('曲线 CSV', '导出演化时间序列（每采样点一行）', () => {
      if (this.onExportFossil) this.onExportFossil();
    });
    mk('物种表 CSV', '导出物种表（含祖先关系，可在外部重建系统发生树）', () => {
      if (this.onExportSpecies) this.onExportSpecies();
    });
    section.appendChild(row);
  }

  _buildView() {
    const section = this._section('视图');
    const row = document.createElement('div');
    row.className = 'tool-row';

    const mk = (label, fn) => {
      const b = document.createElement('button');
      b.className = 'tool-btn';
      b.textContent = label;
      b.addEventListener('click', fn);
      row.appendChild(b);
      return b;
    };

    mk('缩小', () => {
      this.camera.setZoom(this.camera.zoom / 1.4, this.renderer.width, this.renderer.height);
      this.renderer.needsRender = true;
    });
    mk('放大', () => {
      this.camera.setZoom(this.camera.zoom * 1.4, this.renderer.width, this.renderer.height);
      this.renderer.needsRender = true;
    });
    mk('适应窗口', () => this.fitToWindow());

    section.appendChild(row);
  }

  _brushColor(id) {
    if (id === BRUSH_ORGANISM) return '#f0abfc';
    if (id === BRUSH_CLONE) return '#38bdf8';
    if (id === BRUSH_ERASER) return '#cbd5e1';
    return this.colorScheme.colorOf(id);
  }

  _notifyChanged() {
    this.renderer.needsRender = true;
    if (this.onWorldChanged) this.onWorldChanged();
  }

  fitToWindow() {
    const grid = this.world.grid;
    this.camera.fitGrid(grid.cols, grid.rows, this.renderer.width, this.renderer.height, 20);
    this.renderer.requestFullRedraw();
  }

  _syncTools() {
    for (const { id, btn } of this._toolButtons) {
      btn.classList.toggle('is-active', id === this.editor.tool);
    }
  }

  _syncSwatches() {
    const hasBlueprint = !!this.editor.cloneBlueprint;

    for (const { id, btn, needsBlueprint } of this._swatches) {
      btn.classList.toggle('is-active', id === this.editor.brushType);
      if (needsBlueprint) {
        btn.classList.toggle('is-empty', !hasBlueprint);
        btn.title = hasBlueprint
          ? `投放「${this.editor.cloneBlueprint.label}」的个体`
          : '先在画布上检视一个个体，再点这里或"克隆选中"';
      }
    }

    // 笔刷大小只对地形笔刷有意义
    const isTerrain = this.editor.brushType !== BRUSH_ORGANISM
      && this.editor.brushType !== BRUSH_CLONE;
    if (this.sizeRow) this.sizeRow.hidden = !isTerrain;

    if (this.brushHint) {
      if (this.editor.brushType === BRUSH_ERASER) {
        this.brushHint.textContent = '橡皮擦：只擦地形（墙 / 食物），生物细胞一律不动';
      } else if (!hasBlueprint) {
        this.brushHint.textContent = '克隆笔刷：用"检视"工具点一个个体，再点"克隆选中"';
      } else if (this.editor.brushType === BRUSH_CLONE) {
        this.brushHint.textContent = `正在投放：${this.editor.cloneBlueprint.label}`;
      } else {
        this.brushHint.textContent = `已装入蓝图：${this.editor.cloneBlueprint.label}`;
      }
    }
  }

  /** 笔刷被外部改动后刷新高亮与提示（例如装入了新的克隆蓝图） */
  refreshBrush() {
    this._syncSwatches();
  }

  /** 配色方案变化后刷新色块颜色 */
  refreshColors() {
    for (const { id, dot } of this._swatches) {
      dot.style.background = this._brushColor(id);
    }
  }
}

function brushTitle(brush) {
  if (brush.id === BRUSH_ORGANISM) return '点击投放一个随机小生物';
  if (brush.id === BRUSH_CLONE) return '投放指定蓝图的个体（需要先装入一份蓝图）';
  if (brush.id === BRUSH_ERASER) return '橡皮擦：只清除地形（墙 / 食物），不会碰到任何生物细胞';
  return cellTypeLabel(brush.id);
}
