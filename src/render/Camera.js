/**
 * 相机：缩放、平移、坐标变换。
 *
 * 相机状态 = 视口左上角对应的世界坐标 (x, y) + 每格像素数 zoom。
 *   screenX = (col - x) * zoom
 *   screenY = (row - y) * zoom
 */

export const MIN_ZOOM = 0.5;
export const MAX_ZOOM = 40;

export class Camera {
  constructor(options = {}) {
    this.x = 0;          // 视口左上角的世界列坐标（浮点）
    this.y = 0;          // 视口左上角的世界行坐标（浮点）
    this.zoom = options.zoom ?? 4;

    this.follow = null;  // 跟随目标 { col, row }，非空时忽略手动平移
    /** 相机变化回调（渲染层用来请求全量重绘） */
    this.onChange = null;
  }

  worldToScreenX(col) { return (col - this.x) * this.zoom; }
  worldToScreenY(row) { return (row - this.y) * this.zoom; }
  screenToWorldX(px) { return this.x + px / this.zoom; }
  screenToWorldY(py) { return this.y + py / this.zoom; }

  /** 屏幕像素坐标 → 网格坐标（向下取整） */
  screenToCell(px, py) {
    return [
      Math.floor(this.screenToWorldX(px)),
      Math.floor(this.screenToWorldY(py)),
    ];
  }

  /** 当前视口覆盖的格子范围 [c0, r0, c1, r1]（含端点，可能超出边界） */
  visibleRange(width, height) {
    const c0 = Math.floor(this.x) - 1;
    const r0 = Math.floor(this.y) - 1;
    const c1 = Math.ceil(this.x + width / this.zoom) + 1;
    const r1 = Math.ceil(this.y + height / this.zoom) + 1;
    return [c0, r0, c1, r1];
  }

  _changed() {
    if (this.onChange) this.onChange();
  }

  /** 以某个屏幕点为锚点缩放，锚点下的世界坐标保持不动 */
  zoomAt(screenX, screenY, factor) {
    const worldX = this.screenToWorldX(screenX);
    const worldY = this.screenToWorldY(screenY);
    const next = clamp(this.zoom * factor, MIN_ZOOM, MAX_ZOOM);
    if (next === this.zoom) return;
    this.zoom = next;
    this.x = worldX - screenX / this.zoom;
    this.y = worldY - screenY / this.zoom;
    this._changed();
  }

  /** 按屏幕像素平移 */
  panByPixels(dx, dy) {
    this.follow = null;
    this.x -= dx / this.zoom;
    this.y -= dy / this.zoom;
    this._changed();
  }

  /** 直接设置缩放（以视口中心为锚点） */
  setZoom(zoom, width, height) {
    const cx = width / 2;
    const cy = height / 2;
    this.zoomAt(cx, cy, clamp(zoom, MIN_ZOOM, MAX_ZOOM) / this.zoom);
  }

  /** 把整个网格居中显示在给定视口里 */
  fitGrid(cols, rows, width, height, padding = 24) {
    const zx = (width - padding * 2) / cols;
    const zy = (height - padding * 2) / rows;
    this.zoom = clamp(Math.min(zx, zy), MIN_ZOOM, MAX_ZOOM);
    this.x = (cols - width / this.zoom) / 2;
    this.y = (rows - height / this.zoom) / 2;
    this.follow = null;
    this._changed();
  }

  /** 把某个格子居中 */
  centerOn(col, row, width, height) {
    this.x = col - width / (2 * this.zoom);
    this.y = row - height / (2 * this.zoom);
    this._changed();
  }

  /** 每帧调用：若在跟随模式，把目标保持在视口中心 */
  updateFollow(width, height) {
    if (!this.follow) return false;
    this.x = this.follow.col - width / (2 * this.zoom);
    this.y = this.follow.row - height / (2 * this.zoom);
    return true;
  }
}

function clamp(v, min, max) {
  return v < min ? min : v > max ? max : v;
}
