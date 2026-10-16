import { CellType } from '../life/CellType.js';
import { spawnRandomOrganism, spawnFromBlueprint } from '../life/OrganismFactory.js';

/** 特殊笔刷：投放一个随机小生物（不是细胞类型，所以单独用字符串标识） */
export const BRUSH_ORGANISM = 'organism';

/** 特殊笔刷：投放一份指定蓝图（克隆笔刷，蓝图由检视器给出） */
export const BRUSH_CLONE = 'clone';

/** 特殊笔刷：橡皮擦——只清地形（墙 / 食物），生物细胞一律不动 */
export const BRUSH_ERASER = 'eraser';

/**
 * 画布输入控制：画笔、平移、缩放、检视。
 *
 * 四种工具共用同一套指针事件，靠 this.tool 分流。
 */
export class Editor {
  constructor(canvas, world, camera, renderer) {
    this.canvas = canvas;
    this.world = world;
    this.camera = camera;
    this.renderer = renderer;

    this.tool = 'paint';            // 'paint' | 'pan' | 'inspect'
    this.brushType = CellType.WALL;
    this.brushSize = 2;

    /** 克隆笔刷的蓝图；null 表示还没选过 */
    this.cloneBlueprint = null;

    this._panning = false;
    this._painting = false;
    this._lastPixel = null;
    this._lastCell = null;
    this._lastPaintCell = null;

    /** 一次拖动里成功放置 / 放不下的个数（松开时汇报，用于给用户反馈） */
    this._placed = 0;
    this._missed = 0;

    /** 绘制发生后的回调（UI 用来刷新读数） */
    this.onPaint = null;

    /** 放置结果回调 (placed, missed) => void */
    this.onPlaceFeedback = null;

    /** 检视点击的回调。参数是生物对象；点到空地时传 null */
    this.onInspect = null;

    /** 笔刷被外部改动后（例如切到克隆笔刷）的回调，工具栏用它同步高亮 */
    this.onBrushChanged = null;

    this._bind();
  }

  setTool(tool) {
    this.tool = tool;
    this._applyCursor();
  }

  setBrushType(type) {
    this.brushType = type;
    this._applyCursor();
    if (this.onBrushChanged) this.onBrushChanged(type);
  }

  /** 是否处于"放置模式"（装好了蓝图、正在等用户点画布） */
  get isPlacing() {
    return this.brushType === BRUSH_CLONE && !!this.cloneBlueprint;
  }

  setBrushSize(size) {
    this.brushSize = Math.max(1, Math.min(24, size | 0));
    this._refreshCursor(this._lastCell);
  }

  /**
   * 装入一份克隆蓝图并切到克隆笔刷。
   * 之后每次点击都会在那个位置投放这个物种的个体。
   */
  setCloneBlueprint(blueprint) {
    this.cloneBlueprint = blueprint;
    this.setTool('paint');
    this.setBrushType(BRUSH_CLONE);
  }

  /** 取消放置模式（把蓝图丢掉、笔刷换回墙） */
  cancelClone() {
    this.cloneBlueprint = null;
    if (this.brushType === BRUSH_CLONE) {
      this.brushType = CellType.WALL;
    }
    this._applyCursor();
    if (this.onBrushChanged) this.onBrushChanged(this.brushType);
  }

  /** 让光标反映当前工具——平移时会临时覆盖成 grabbing，松开要还回来 */
  _applyCursor() {
    if (this.tool === 'inspect') {
      this.canvas.style.cursor = 'pointer';
      return;
    }
    // 放置类笔刷给十字准星，一眼能看出"现在点下去会投放东西"
    const placing = this.brushType === BRUSH_CLONE || this.brushType === BRUSH_ORGANISM;
    this.canvas.style.cursor = placing && this.tool === 'paint' ? 'crosshair' : '';
  }

  // ────────────────────────────── 事件绑定 ──────────────────────────────

  _bind() {
    const canvas = this.canvas;

    canvas.addEventListener('pointerdown', this._onPointerDown);
    canvas.addEventListener('pointermove', this._onPointerMove);
    canvas.addEventListener('pointerup', this._onPointerUp);
    canvas.addEventListener('pointercancel', this._onPointerUp);
    canvas.addEventListener('pointerleave', this._onPointerLeave);
    canvas.addEventListener('wheel', this._onWheel, { passive: false });
    canvas.addEventListener('contextmenu', (e) => e.preventDefault());
  }

  _localPoint(event) {
    const rect = this.canvas.getBoundingClientRect();
    return [event.clientX - rect.left, event.clientY - rect.top];
  }

  _onPointerDown = (event) => {
    // 平移：中键 / 右键 / Alt+左键 / 平移工具
    if (event.button === 1 || event.button === 2 || this.tool === 'pan' || event.altKey) {
      this._panning = true;
      this._lastPixel = this._localPoint(event);
      this.canvas.setPointerCapture(event.pointerId);
      this.canvas.style.cursor = 'grabbing';
      event.preventDefault();
      return;
    }
    if (event.button !== 0) return;

    const [px, py] = this._localPoint(event);
    const [col, row] = this.camera.screenToCell(px, py);

    // 检视工具：单击即选中，不进入拖拽状态
    if (this.tool === 'inspect') {
      this._inspectAt(col, row);
      return;
    }

    this._painting = true;
    this.canvas.setPointerCapture(event.pointerId);
    this._lastCell = [col, row];
    this._placed = 0;
    this._missed = 0;
    this._paintAt(col, row);
  };

  _onPointerMove = (event) => {
    const [px, py] = this._localPoint(event);
    const [col, row] = this.camera.screenToCell(px, py);

    if (this._panning) {
      const dx = px - this._lastPixel[0];
      const dy = py - this._lastPixel[1];
      this._lastPixel = [px, py];
      this.camera.panByPixels(dx, dy);
      return;
    }

    this._lastCell = [col, row];
    this._refreshCursor([col, row]);

    if (this._painting) {
      if (this._lastPaintCell) {
        this._strokeLine(this._lastPaintCell[0], this._lastPaintCell[1], col, row);
      } else {
        this._paintAt(col, row);
      }
      this._lastPaintCell = [col, row];
    }
  };

  _onPointerUp = (event) => {
    if (this._panning) {
      this._panning = false;
      this._applyCursor();
      if (this.canvas.hasPointerCapture(event.pointerId)) {
        this.canvas.releasePointerCapture(event.pointerId);
      }
      return;
    }
    if (this._painting) {
      this._painting = false;
      this._lastPaintCell = null;
      if (this.canvas.hasPointerCapture(event.pointerId)) {
        this.canvas.releasePointerCapture(event.pointerId);
      }
      if (this._placed > 0 || this._missed > 0) {
        const placed = this._placed;
        const missed = this._missed;
        this._placed = 0;
        this._missed = 0;
        if (this.onPlaceFeedback) this.onPlaceFeedback(placed, missed);
      }
      if (this.onPaint) this.onPaint();
    }
  };

  _onPointerLeave = () => {
    this.renderer.cursor = null;
    this.renderer.needsRender = true;
    this._lastCell = null;
  };

  _onWheel = (event) => {
    event.preventDefault();
    const [px, py] = this._localPoint(event);
    const factor = Math.pow(1.0015, -event.deltaY);
    this.camera.zoomAt(px, py, factor);
    this.renderer.needsRender = true;
  };

  // ────────────────────────────── 绘制 ──────────────────────────────

  _refreshCursor(cell) {
    if (!cell) {
      this.renderer.cursor = null;
      return;
    }
    this.renderer.cursor = { col: cell[0], row: cell[1], size: this.brushSize };
    this.renderer.needsRender = true;
  }

  _paintAt(col, row) {
    // 生物笔刷：一次事件只投放一个，避免大笔刷瞬间刷出成百上千个生物
    if (this.brushType === BRUSH_ORGANISM) {
      if (spawnRandomOrganism(this.world, col, row)) this._placed++;
      else this._missed++;
      return;
    }

    // 克隆/放置笔刷：把装好的蓝图投放到这一格
    if (this.brushType === BRUSH_CLONE) {
      if (!this.cloneBlueprint) return;
      if (spawnFromBlueprint(this.world, col, row, this.cloneBlueprint)) this._placed++;
      else this._missed++;
      return;
    }

    const size = this.brushSize;
    const offset = Math.floor((size - 1) / 2);
    const c0 = col - offset;
    const r0 = row - offset;
    const erasing = this.brushType === BRUSH_ERASER;
    for (let dr = 0; dr < size; dr++) {
      for (let dc = 0; dc < size; dc++) {
        const cc = c0 + dc;
        const rr = r0 + dr;
        if (erasing) this._eraseCell(cc, rr);
        else this.world.setCell(cc, rr, this.brushType, -1);
      }
    }
  }

  /**
   * 橡皮擦的单格操作：只清"地形"，生物细胞一律跳过。
   *
   * 判据是 `owners[index] < 0`——生物写格子时会登记 owner，地形不会。
   * 用 owner 而不是"类型是不是 EMPTY/FOOD/WALL"，是因为前者不依赖类型的
   * 枚举顺序，将来加新的地形类型（岩石/水/岩浆…）也不用改这里。
   *
   * 这也是"橡皮擦"和"空笔刷"的区别：空笔刷是无差别覆盖（会把生物身体抹掉、
   * 留下一片废墟），橡皮擦只擦地形，正好用来给生物开路/清场而不伤到它们。
   */
  _eraseCell(col, row) {
    const grid = this.world.grid;
    const index = grid.index(col, row);
    if (index < 0) return;
    if (grid.owners[index] >= 0) return;      // 有主的格子 = 生物身体，跳过
    this.world.setCellAt(index, CellType.EMPTY, -1);
  }

  /** 检视：点到的格子上有生物就选中它，否则报告"点空了" */
  _inspectAt(col, row) {
    const grid = this.world.grid;
    const index = grid.index(col, row);
    let organism = null;

    if (index >= 0) {
      const ownerId = grid.owners[index];
      if (ownerId >= 0) organism = this.world.organismById(ownerId) || null;
    }

    if (this.onInspect) this.onInspect(organism);
  }

  /** 两格之间补线，避免快速拖动时出现断点 */
  _strokeLine(c1, r1, c2, r2) {
    let x = c1;
    let y = r1;
    const dx = Math.abs(c2 - c1);
    const dy = -Math.abs(r2 - r1);
    const sx = c1 < c2 ? 1 : -1;
    const sy = r1 < r2 ? 1 : -1;
    let err = dx + dy;

    for (let guard = 0; guard < 4096; guard++) {
      this._paintAt(x, y);
      if (x === c2 && y === r2) break;
      const e2 = 2 * err;
      if (e2 >= dy) { err += dy; x += sx; }
      if (e2 <= dx) { err += dx; y += sy; }
    }
  }
}
