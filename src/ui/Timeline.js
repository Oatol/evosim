import { SPEED_STEPS } from '../core/Engine.js';

/**
 * 时间轴：播放 / 暂停 / 单步 / 倍速 / 重置。
 */
export class Timeline {
  constructor(container, engine, world) {
    this.container = container;
    this.engine = engine;
    this.world = world;

    this._speedButtons = [];
    this._build();
    this.refresh();
  }

  _build() {
    const el = this.container;
    el.innerHTML = '';

    // ── 播放控制 ──
    const transport = document.createElement('div');
    transport.className = 'tl-group';

    this.playButton = document.createElement('button');
    this.playButton.className = 'tl-btn tl-btn-primary';
    this.playButton.textContent = '播放';
    this.playButton.addEventListener('click', () => {
      this.engine.toggle();
      this.refresh();
    });
    transport.appendChild(this.playButton);

    this.stepButton = document.createElement('button');
    this.stepButton.className = 'tl-btn';
    this.stepButton.textContent = '单步';
    this.stepButton.title = '推进一个 tick';
    this.stepButton.addEventListener('click', () => {
      this.engine.pause();
      this.engine.stepOnce();
      this.refresh();
    });
    transport.appendChild(this.stepButton);

    this.resetButton = document.createElement('button');
    this.resetButton.className = 'tl-btn';
    this.resetButton.textContent = '重置';
    this.resetButton.title = '清空世界并归零 tick';
    this.resetButton.addEventListener('click', () => {
      this.engine.pause();
      this.world.reset();
      this.refresh();
    });
    transport.appendChild(this.resetButton);

    el.appendChild(transport);

    // ── 倍速 ──
    const speeds = document.createElement('div');
    speeds.className = 'tl-group';
    const speedLabel = document.createElement('span');
    speedLabel.className = 'tl-label';
    speedLabel.textContent = '倍速';
    speeds.appendChild(speedLabel);

    for (const step of SPEED_STEPS) {
      const btn = document.createElement('button');
      btn.className = 'tl-btn tl-btn-speed';
      btn.textContent = '×' + step;
      btn.addEventListener('click', () => {
        this.engine.setSpeed(step);
        this.refresh();
      });
      this._speedButtons.push({ step, btn });
      speeds.appendChild(btn);
    }
    el.appendChild(speeds);

    // ── 读数 ──
    const readout = document.createElement('div');
    readout.className = 'tl-group tl-readout';

    this.tickLabel = document.createElement('span');
    this.tickLabel.className = 'tl-stat';
    readout.appendChild(this.tickLabel);

    this.rateLabel = document.createElement('span');
    this.rateLabel.className = 'tl-stat tl-stat-dim';
    readout.appendChild(this.rateLabel);

    el.appendChild(readout);
  }

  /** 根据引擎状态同步按钮外观与读数 */
  refresh() {
    const running = this.engine.running;
    this.playButton.textContent = running ? '暂停' : '播放';
    this.playButton.classList.toggle('is-running', running);

    for (const { step, btn } of this._speedButtons) {
      btn.classList.toggle('is-active', step === this.engine.speed);
    }

    this.tickLabel.textContent = `tick ${this.world.tick.toLocaleString('en-US')}`;
  }

  /** 每帧调用的轻量读数刷新（只更新变化的文本） */
  refreshRates() {
    const tps = Math.round(this.engine.tps);
    const fps = Math.round(this.engine.fps);
    this.rateLabel.textContent = `${tps} tick/s · ${fps} fps`;
  }
}
