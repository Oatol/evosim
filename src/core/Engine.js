/**
 * 引擎：主循环、时间控制、倍速、单步。
 *
 * 渲染走 requestAnimationFrame，模拟走固定步长累加器——
 * 两者解耦，所以倍速不会影响渲染帧率，低倍速下也不会掉帧。
 */

export const SPEED_STEPS = [1, 2, 5, 10, 50];

export class Engine {
  constructor(world, options = {}) {
    this.world = world;
    this.baseTps = options.baseTicksPerSecond ?? 60;
    this.maxStepsPerFrame = options.maxStepsPerFrame ?? 400;

    this.speed = 1;
    this.running = false;

    this._accumulator = 0;
    this._lastTime = 0;
    this._rafId = null;

    // 平滑后的读数
    this.fps = 0;
    this.tps = 0;
    this._ticksThisSecond = 0;
    this._secondTimer = 0;
    this._framesThisSecond = 0;

    /** 每次 tick 后回调（UI 刷新读数用） */
    this.onTick = null;
    /** 每帧渲染前回调 */
    this.onFrame = null;
    /** 每帧渲染后回调 */
    this.onAfterFrame = null;
  }

  get ticksPerSecond() {
    return this.baseTps * this.speed;
  }

  /**
   * 开始推进模拟。
   *
   * **渲染循环只在第一次启动时开启，之后一直跑**，`running` 只决定
   * "要不要推进世界"。早期版本在 pause() 里 cancelAnimationFrame，
   * 结果暂停后 onFrame 再也不会触发——画布上的准星、高亮、浮窗全部
   * 冻在最后一帧（用户实际反馈："暂停时准星也暂停了"）。
   */
  start() {
    if (this.running) return;
    this.running = true;
    this._lastTime = performance.now();
    this._accumulator = 0;
    this._ensureLoop();
  }

  /** 暂停模拟：世界不再前进，但每帧照常渲染（观察、绘制、拖浮窗都要用） */
  pause() {
    this.running = false;
    this._accumulator = 0;
    this._ensureLoop();
  }

  /** 彻底停掉渲染循环（页面销毁时用，正常玩法不需要） */
  stop() {
    this.running = false;
    if (this._rafId !== null) {
      cancelAnimationFrame(this._rafId);
      this._rafId = null;
    }
  }

  _ensureLoop() {
    if (this._rafId !== null) return;
    this._lastTime = performance.now();
    this._rafId = requestAnimationFrame(this._frame);
  }

  toggle() {
    if (this.running) this.pause();
    else this.start();
  }

  setSpeed(multiplier) {
    this.speed = multiplier;
    this._accumulator = 0;
  }

  setBaseTps(tps) {
    this.baseTps = tps;
    this._accumulator = 0;
  }

  /** 单步：推进一个 tick 并立刻渲染 */
  stepOnce() {
    this.world.step();
    if (this.onTick) this.onTick();
    if (this.onFrame) this.onFrame();
    if (this.onAfterFrame) this.onAfterFrame();
  }

  /** 推进 n 个 tick */
  stepMany(n) {
    for (let i = 0; i < n; i++) this.world.step();
    if (this.onTick) this.onTick();
    if (this.onFrame) this.onFrame();
    if (this.onAfterFrame) this.onAfterFrame();
  }

  _frame = (now) => {
    this._rafId = null;

    const dt = Math.min(now - this._lastTime, 250); // 切后台回来时不要补算太久
    this._lastTime = now;

    // ── 模拟：固定步长累加（暂停时 steps 恒为 0，但下面的渲染照常走）──
    let steps = 0;
    if (this.running && this.speed > 0) {
      this._accumulator += (dt / 1000) * this.ticksPerSecond;
      steps = Math.floor(this._accumulator);
      if (steps > this.maxStepsPerFrame) {
        steps = this.maxStepsPerFrame;
        this._accumulator = 0;   // 追不上就丢弃，避免雪崩
      } else {
        this._accumulator -= steps;
      }
    }

    for (let i = 0; i < steps; i++) {
      this.world.step();
      if (this.onTick) this.onTick();
    }
    this._ticksThisSecond += steps;

    // ── 渲染：每帧一次 ──
    if (this.onFrame) this.onFrame();
    this._framesThisSecond++;

    // ── 读数统计（每秒刷新一次） ──
    this._secondTimer += dt;
    if (this._secondTimer >= 500) {
      const scale = 1000 / this._secondTimer;
      this.fps = this._framesThisSecond * scale;
      this.tps = this._ticksThisSecond * scale;
      this._framesThisSecond = 0;
      this._ticksThisSecond = 0;
      this._secondTimer = 0;
    }

    if (this.onAfterFrame) this.onAfterFrame();

    this._rafId = requestAnimationFrame(this._frame);
  };
}
