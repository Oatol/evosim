/**
 * 右侧读数面板。
 *
 * 阶段 1 展示世界状态，阶段 2 加上生物群落的聚合读数，
 * 阶段 3 加上物种与变异率读数。
 *
 * 曲线在 Charts 里、显示在面板上方，所以这里只留演化相关的关键数字，
 * 不重要的（总格数、非空格）已经删掉——它们会把曲线挤出首屏。
 */

const WORLD_ROWS = [
  { key: 'organisms', label: '生物数' },
  { key: 'body', label: '身体细胞' },
  { key: 'food', label: '食物' },
  { key: 'wall', label: '墙' },
];

const BIO_ROWS = [
  { key: 'avgCells', label: '平均体型', digits: 2 },
  { key: 'maxCells', label: '最大体型' },
  { key: 'movers', label: '会移动的' },
  { key: 'eyes', label: '有眼睛的' },
  { key: 'killers', label: '有杀手的' },
  { key: 'avgAge', label: '平均年龄', digits: 0 },
  { key: 'avgMutability', label: '平均变异率', digits: 2 },
  { key: 'species', label: '存活物种' },
  { key: 'totalSpecies', label: '物种总数' },
];

export class Stats {
  constructor(container, world, renderer) {
    this.container = container;
    this.world = world;
    this.renderer = renderer;
    this._valueEls = new Map();
    this._build();
    this.refresh();
  }

  _build() {
    const el = this.container;
    el.innerHTML = '';
    this._valueEls.clear();

    this._section(el, '世界读数', WORLD_ROWS);
    this._divider(el);
    this._section(el, '生物群落', BIO_ROWS);
    this._divider(el);

    const title = document.createElement('h2');
    title.className = 'panel-title';
    title.textContent = '视图';
    el.appendChild(title);

    const view = document.createElement('dl');
    view.className = 'stat-list';
    const dt = document.createElement('dt');
    dt.textContent = '缩放';
    this.zoomEl = document.createElement('dd');
    view.appendChild(dt);
    view.appendChild(this.zoomEl);
    el.appendChild(view);

    const hint = document.createElement('p');
    hint.className = 'panel-hint';
    hint.textContent = '左键绘制 · 右键平移 · 滚轮缩放 · 空格播放/暂停 · S 单步 · F 适应窗口 · P 参数 · T 物种树 · G 物种工坊 · 用「检视」点开生物看它的大脑';
    el.appendChild(hint);
  }

  _section(el, title, rows) {
    const h = document.createElement('h2');
    h.className = 'panel-title';
    h.textContent = title;
    el.appendChild(h);

    const list = document.createElement('dl');
    list.className = 'stat-list';
    for (const row of rows) {
      const dt = document.createElement('dt');
      dt.textContent = row.label;
      const dd = document.createElement('dd');
      dd.textContent = '—';
      list.appendChild(dt);
      list.appendChild(dd);
      this._valueEls.set(row.key, { el: dd, digits: row.digits });
    }
    el.appendChild(list);
  }

  _divider(el) {
    const d = document.createElement('div');
    d.className = 'panel-divider';
    el.appendChild(d);
  }

  refresh() {
    const world = this.world.snapshotStats();
    const bio = this.world.organismStats();
    const species = {
      species: this.world.species.aliveCount,
      totalSpecies: this.world.species.list.length,
    };

    for (const [key, entry] of this._valueEls) {
      const raw = world[key] ?? bio[key] ?? species[key];
      if (raw === undefined) continue;
      entry.el.textContent = entry.digits !== undefined
        ? Number(raw).toFixed(entry.digits)
        : Number(raw).toLocaleString('en-US');
    }

    if (this.zoomEl) {
      this.zoomEl.textContent = this.renderer.camera.zoom.toFixed(2) + ' px/格';
    }
  }
}
