import { isDirectional, CELL_TYPE_COUNT } from '../life/CellType.js';
import { speciesRGB, speciesColor } from './ColorScheme.js';

/**
 * 渲染器：离屏静态层 + 脏格更新 + 主画布缩放绘制。
 *
 * 架构：
 *   ┌─ 离屏层（layer）─┐   1 像素 = 1 格，尺寸恒为 cols × rows
 *   │  只更新发生变化的 │   full 重建用 ImageData 一次性写入
 *   │  格子（脏格集合） │   增量更新用 fillRect 逐个补
 *   └────────┬─────────┘
 *            │ drawImage（不插值，整数像素对齐）
 *            ▼
 *   ┌─ 主画布（canvas）─┐  按相机 zoom 缩放绘制；开启环绕时平铺
 *   └──────────────────┘
 *
 * 这样缩放的代价只是 drawImage，与格子数量无关；
 * 而每 tick 的代价只与"变化了多少格"成正比，与网格面积无关。
 */
export class Renderer {
  constructor(canvas, world, camera, colorScheme) {
    this.canvas = canvas;
    this.ctx = canvas.getContext('2d', { alpha: false });
    this.world = world;
    this.camera = camera;
    this.colorScheme = colorScheme;

    // 离屏静态层
    this.layer = document.createElement('canvas');
    this.layerCtx = this.layer.getContext('2d', { alpha: false });

    this.dirty = new Set();
    this.layerNeedsRebuild = true;

    this.width = 1;
    this.height = 1;
    this.dpr = 1;

    this.showEyes = true;
    this.cursor = null;          // { col, row, size }
    this.highlight = null;       // 被检视的生物，画一圈高亮（不属于世界状态）
    this.highlightFingerprint = null;  // 被选中的物种：高亮它的全部存活个体
    /**
     * 着色模式：
     *   'type'    身体细胞按"类型"上色（默认，看得出嘴/生产者/杀手…）
     *   'species' 身体细胞按"所属物种"上色（看得出物种分布与领地）
     * 地形（食物/墙）两种模式下都按类型上色。
     */
    this.colorMode = 'type';
    this._speciesPal = new Map();   // fingerprint -> Uint32（全量重建用）

    this._palette = null;
    this._rebuildLayerCanvas();
    this._bindWorld();
  }

  /** 切换着色模式（会触发一次全量重建） */
  setColorMode(mode) {
    if (this.colorMode === mode) return;
    this.colorMode = mode;
    this._speciesPal.clear();
    this.requestFullRedraw();
  }

  _bindWorld() {
    this.world.on('cellChanged', (index) => this.markDirty(index));
    this.world.on('fullRedraw', () => this.requestFullRedraw());
    this.world.on('reset', () => this.requestFullRedraw());
    this.camera.onChange = () => { this.needsRender = true; };
  }

  /** 世界尺寸变化后调用：重建离屏层并请求全量重绘 */
  notifyWorldResized() {
    this._rebuildLayerCanvas();
    this.needsRender = true;
  }

  // ────────────────────────────── 尺寸 ──────────────────────────────

  /** 跟随容器尺寸，处理 devicePixelRatio */
  resize() {
    const rect = this.canvas.parentElement.getBoundingClientRect();
    const cssW = Math.max(1, Math.floor(rect.width));
    const cssH = Math.max(1, Math.floor(rect.height));
    const dpr = window.devicePixelRatio || 1;

    this.dpr = dpr;
    this.width = cssW;
    this.height = cssH;
    this.canvas.width = Math.floor(cssW * dpr);
    this.canvas.height = Math.floor(cssH * dpr);
    this.canvas.style.width = cssW + 'px';
    this.canvas.style.height = cssH + 'px';

    this.needsRender = true;
    return this;
  }

  // ────────────────────────────── 脏格 ──────────────────────────────

  markDirty(index) {
    this.dirty.add(index);
    this.needsRender = true;
  }

  requestFullRedraw() {
    this.layerNeedsRebuild = true;
    this.needsRender = true;
  }

  /** 配色方案切换后调用 */
  refreshPalette() {
    this._palette = null;
    this.requestFullRedraw();
  }

  // ────────────────────────────── 离屏层 ──────────────────────────────

  _rebuildLayerCanvas() {
    const grid = this.world.grid;
    this.layer.width = grid.cols;
    this.layer.height = grid.rows;
    this.layerCtx.imageSmoothingEnabled = false;
    this._palette = null;
    this.layerNeedsRebuild = true;
  }

  _hexToUint32(hex) {
    // 输出为小端 ABGR，可直接写进 ImageData 的 Uint32 视图
    const r = parseInt(hex.slice(1, 3), 16);
    const g = parseInt(hex.slice(3, 5), 16);
    const b = parseInt(hex.slice(5, 7), 16);
    return ((255 << 24) | (b << 16) | (g << 8) | r) >>> 0;
  }

  _ensurePalette() {
    if (this._palette) return this._palette;
    const pal = new Uint32Array(CELL_TYPE_COUNT);
    for (let t = 0; t < CELL_TYPE_COUNT; t++) {
      pal[t] = this._hexToUint32(this.colorScheme.colorOf(t));
    }
    this._palette = pal;
    return pal;
  }

  /** 全量重建离屏层（用 ImageData 一次性写入，最快） */
  _rebuildLayer() {
    const grid = this.world.grid;
    if (this.layer.width !== grid.cols || this.layer.height !== grid.rows) {
      this.layer.width = grid.cols;
      this.layer.height = grid.rows;
      this.layerCtx.imageSmoothingEnabled = false;
    }

    const pal = this._ensurePalette();
    const img = this.layerCtx.createImageData(grid.cols, grid.rows);
    const buf = new Uint32Array(img.data.buffer);
    const types = grid.types;

    if (this.colorMode === 'species') {
      const owners = grid.owners;
      const pal2 = this._speciesPal;
      for (let i = 0; i < types.length; i++) {
        const ownerId = owners[i];
        if (ownerId >= 0) {
          const org = this.world.organismById(ownerId);
          if (org) {
            let v = pal2.get(org.speciesFingerprint);
            if (v === undefined) {
              const [r, g, b] = speciesRGB(org.speciesFingerprint);
              v = ((255 << 24) | (b << 16) | (g << 8) | r) >>> 0;
              pal2.set(org.speciesFingerprint, v);
            }
            buf[i] = v;
            continue;
          }
        }
        buf[i] = pal[types[i]];
      }
    } else {
      for (let i = 0; i < types.length; i++) buf[i] = pal[types[i]];
    }

    this.layerCtx.putImageData(img, 0, 0);
    this.layerNeedsRebuild = false;
    this.dirty.clear();
  }

  /** 增量更新：只重绘脏格 */
  _paintDirty() {
    if (this.dirty.size === 0) return;
    const lctx = this.layerCtx;
    const grid = this.world.grid;
    const cols = grid.cols;
    const types = grid.types;
    const owners = grid.owners;
    const bySpecies = this.colorMode === 'species';
    let lastStyle = null;

    for (const index of this.dirty) {
      const t = types[index];
      let style = null;

      if (bySpecies) {
        const ownerId = owners[index];
        if (ownerId >= 0) {
          const org = this.world.organismById(ownerId);
          if (org) style = speciesColor(org.speciesFingerprint);
        }
      }
      if (style === null) style = this.colorScheme.colorOf(t);

      if (style !== lastStyle) {
        lctx.fillStyle = style;
        lastStyle = style;
      }
      const col = index % cols;
      const row = (index / cols) | 0;
      lctx.fillRect(col, row, 1, 1);
    }
    this.dirty.clear();
  }

  // ────────────────────────────── 主渲染 ──────────────────────────────

  render() {
    const ctx = this.ctx;
    const cam = this.camera;
    const grid = this.world.grid;
    const { width, height } = this;

    if (this.layerNeedsRebuild) this._rebuildLayer();
    else this._paintDirty();

    ctx.setTransform(this.dpr, 0, 0, this.dpr, 0, 0);
    ctx.imageSmoothingEnabled = false;

    ctx.fillStyle = this.colorScheme.background;
    ctx.fillRect(0, 0, width, height);

    const zoom = cam.zoom;
    const originX = cam.worldToScreenX(0);
    const originY = cam.worldToScreenY(0);
    const tileW = grid.cols * zoom;
    const tileH = grid.rows * zoom;

    if (grid.wrap) {
      this._drawTiled(ctx, originX, originY, tileW, tileH, width, height);
    } else {
      ctx.drawImage(this.layer, originX, originY, tileW, tileH);
    }

    if (this.showEyes && zoom >= 6) this._drawEyeSlits(ctx);

    this._drawHighlight(ctx);
    this._drawCursor(ctx);

    this.needsRender = false;
  }

  /**
   * 环绕世界：把离屏层平铺出去。
   *
   * **只在"世界比视口大"的方向平铺**——那时平铺的作用是让跨环绕边界平移时
   * 不出现空洞，最多补到 2 块。
   *
   * 世界比视口小的方向**不平铺**：缩小时把整个世界重复铺满屏幕，会让人以为
   * "世界被复制了很多份"（用户实际反馈过这个问题），而缩小真正想看的只是
   * "整个世界长什么样"。这时就画一块，四周留背景色。
   */
  _drawTiled(ctx, originX, originY, tileW, tileH, width, height) {
    const tileX = tileW >= width;
    const tileY = tileH >= height;

    if (!tileX && !tileY) {
      ctx.drawImage(this.layer, originX, originY, tileW, tileH);
      return;
    }

    // 需要平铺的方向按视口算范围（跨边界时会是 -1 / +1，最多 2 块）；
    // 不需要平铺的方向固定在 0，即世界自己的位置。
    const startCol = tileX ? Math.floor((0 - originX) / tileW) : 0;
    const endCol = tileX ? Math.ceil((width - originX) / tileW) : 0;
    const startRow = tileY ? Math.floor((0 - originY) / tileH) : 0;
    const endRow = tileY ? Math.ceil((height - originY) / tileH) : 0;

    for (let ty = startRow; ty <= endRow; ty++) {
      const dy = originY + ty * tileH;
      for (let tx = startCol; tx <= endCol; tx++) {
        const dx = originX + tx * tileW;
        ctx.drawImage(this.layer, dx, dy, tileW, tileH);
      }
    }
  }

  /**
   * 高倍率下给眼睛加一道朝向细缝。
   * 朝向来自所属生物的身体蓝图（局部朝向 + 身体旋转），所以这条细缝
   * 是能看出生物"面朝哪边"的——观察捕食行为时很有用。
   */
  _drawEyeSlits(ctx) {
    const cam = this.camera;
    const world = this.world;
    const grid = world.grid;
    const [c0, r0, c1, r1] = cam.visibleRange(this.width, this.height);
    const size = cam.zoom;

    const thickness = Math.max(1, size * 0.26);
    const length = Math.max(1, size * 0.44);
    const inset = size * 0.12;

    ctx.fillStyle = this.colorScheme.pupil;

    const cc0 = Math.max(c0, grid.wrap ? -grid.cols : 0);
    const cc1 = Math.min(c1, grid.wrap ? grid.cols * 2 : grid.cols - 1);
    const rr0 = Math.max(r0, grid.wrap ? -grid.rows : 0);
    const rr1 = Math.min(r1, grid.wrap ? grid.rows * 2 : grid.rows - 1);

    for (let row = rr0; row <= rr1; row++) {
      for (let col = cc0; col <= cc1; col++) {
        const index = grid.index(col, row);
        if (index < 0 || !isDirectional(grid.types[index])) continue;

        const ownerId = grid.owners[index];
        const org = ownerId >= 0 ? world.organismById(ownerId) : null;
        // 没有主人的眼睛（例如手绘残留）默认朝上
        const dir = org ? org.eyeDirectionAt(col, row) : 0;
        if (dir < 0) continue;

        const x = Math.round(cam.worldToScreenX(col));
        const y = Math.round(cam.worldToScreenY(row));
        const cx = x + (size - length) / 2;
        const cy = y + (size - length) / 2;

        switch (dir) {
          case 0: ctx.fillRect(cx, y + inset, length, thickness); break;
          case 2: ctx.fillRect(cx, y + size - inset - thickness, length, thickness); break;
          case 3: ctx.fillRect(x + inset, cy, thickness, length); break;
          default: ctx.fillRect(x + size - inset - thickness, cy, thickness, length); break;
        }
      }
    }
  }

  _drawCursor(ctx) {
    if (!this.cursor) return;
    const cam = this.camera;
    const { col, row, size } = this.cursor;
    const half = (size - 1) / 2;
    const x = Math.round(cam.worldToScreenX(col - half));
    const y = Math.round(cam.worldToScreenY(row - half));
    const s = Math.round(size * cam.zoom);
    ctx.strokeStyle = 'rgba(255,255,255,0.75)';
    ctx.lineWidth = 1;
    ctx.strokeRect(x + 0.5, y + 0.5, s - 1, s - 1);
  }

  /**
   * 选中生物的高亮。画在主画布上而不是离屏层里——
   * 高亮是"观察者的标记"，不属于世界状态，不该被写进静态层。
   */
  _drawHighlight(ctx) {
    const cam = this.camera;
    const zoom = cam.zoom;

    // ── 物种高亮：把该物种的全部存活个体圈出来 ──
    // 这是物种树的"点击联动"：树上选一个物种，画布上就能看到它占了哪片地。
    if (this.highlightFingerprint) {
      const orgs = this.world.organisms;
      ctx.save();
      ctx.strokeStyle = 'rgba(248,250,252,0.9)';
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      let drawn = 0;
      for (let i = 0; i < orgs.length && drawn < 4000; i++) {
        const o = orgs[i];
        if (!o.alive || o.speciesFingerprint !== this.highlightFingerprint) continue;
        const cx = cam.worldToScreenX(o.col) + zoom / 2;
        const cy = cam.worldToScreenY(o.row) + zoom / 2;
        const r = Math.max(zoom * (o.anatomy.radius + 1.1), 5);
        ctx.moveTo(cx + r, cy);
        ctx.arc(cx, cy, r, 0, Math.PI * 2);
        drawn++;
      }
      ctx.stroke();
      ctx.restore();
    }

    const org = this.highlight;
    if (!org || !org.alive) return;

    const cells = org.anatomy.cells;

    ctx.save();
    ctx.strokeStyle = 'rgba(56,189,248,0.95)';
    ctx.lineWidth = Math.max(1.5, Math.min(zoom * 0.1, 3));

    // 放大时逐格描边，缩小时只留中心圆环（否则描边会糊成一团）
    if (zoom >= 3) {
      ctx.beginPath();
      for (let i = 0; i < cells.length; i++) {
        const cell = cells[i];
        const x = cam.worldToScreenX(org.realCol(cell));
        const y = cam.worldToScreenY(org.realRow(cell));
        ctx.rect(x + 0.5, y + 0.5, zoom - 1, zoom - 1);
      }
      ctx.stroke();
    }

    const cx = cam.worldToScreenX(org.col) + zoom / 2;
    const cy = cam.worldToScreenY(org.row) + zoom / 2;
    const r = Math.max(zoom * (org.anatomy.radius + 1.3), 10);
    ctx.beginPath();
    ctx.arc(cx, cy, r, 0, Math.PI * 2);
    ctx.stroke();
    ctx.restore();
  }

  /** 把某个格子居中显示 */
  centerOnCell(col, row) {
    this.camera.centerOn(col, row, this.width, this.height);
    this.requestFullRedraw();
  }
}
