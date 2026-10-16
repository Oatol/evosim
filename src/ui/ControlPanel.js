import { CONFIG_SECTIONS, CONFIG_FIELDS, DEFAULT_CONFIG } from '../core/config.js';

/**
 * 参数面板 —— 设计文档 §9.2。
 *
 * 全部控件由 `CONFIG_FIELDS` 的元数据生成，不手写 45 个滑块：
 * 参数表是唯一事实来源，加一个参数只需要改 config.js。
 *
 * 做成**抽屉**而不是常驻侧栏：参数有 40 多项，塞进左栏会把
 * 画笔/笔刷挤出屏幕——而"一边画一边调参"正是这个沙盒的核心玩法。
 * 抽屉展开时画布仍然可见、可交互。
 */

/** step 决定显示几位小数：0.1 → 1 位，0.005 → 3 位 */
function decimalsOf(step) {
  const s = String(step);
  const dot = s.indexOf('.');
  return dot < 0 ? 0 : s.length - dot - 1;
}

/**
 * 字段键支持一层点号路径，例如 `MOUTH.hp`。
 *
 * 细胞属性表是"每种类型一组属性"的嵌套结构（Config.cells.MOUTH.hp），
 * 但参数面板的元数据是平铺的键值对。用点号把两者接起来，
 * 面板就不必为嵌套结构写特殊分支。
 */
function readPath(obj, key) {
  const dot = key.indexOf('.');
  return dot < 0 ? obj[key] : obj[key.slice(0, dot)][key.slice(dot + 1)];
}

function writePath(obj, key, value) {
  const dot = key.indexOf('.');
  if (dot < 0) { obj[key] = value; return; }
  obj[key.slice(0, dot)][key.slice(dot + 1)] = value;
}

export class ControlPanel {
  constructor(container, config, options = {}) {
    this.container = container;
    this.config = config;

    /** (sectionKey, fieldKey, value) => void —— 每次改动后调用 */
    this.onChange = options.onChange || null;
    /** 世界尺寸这类"延迟生效"参数的待应用值 */
    this.onDeferredChange = options.onDeferredChange || null;
    /** 世界尺寸的"应用"按钮 */
    this.onResizeRequest = options.onResizeRequest || null;
    /** 抽屉开合 */
    this.onToggle = null;

    this._rows = new Map();      // "section.key" -> { input, valueEl, sync }
    this._groups = new Map();
    /**
     * 延迟生效参数的待应用值。
     *
     * 为什么不让它直接写 Config：网格尺寸在"世界真的重建"之前是不成立的，
     * 如果 Config.world.cols 先变成 300 而 grid.cols 还是 200，
     * 两处不一致会让后续读 Config 的代码拿到错的数字。
     * 所以待应用值单独存着，等重建完再清掉。
     */
    this._pending = new Map();

    this._build();
  }

  get isOpen() {
    return !this.root.hidden;
  }

  open() {
    this.root.hidden = false;
    if (this.onToggle) this.onToggle(true);
  }

  close() {
    this.root.hidden = true;
    if (this.onToggle) this.onToggle(false);
  }

  toggle() {
    if (this.isOpen) this.close();
    else this.open();
  }

  // ────────────────────────────── DOM ──────────────────────────────

  _build() {
    const root = document.createElement('aside');
    root.className = 'param-drawer';
    root.hidden = true;

    const head = document.createElement('header');
    head.className = 'param-head';

    const titles = document.createElement('div');
    const title = document.createElement('div');
    title.className = 'param-title';
    title.textContent = '参数';
    const sub = document.createElement('div');
    sub.className = 'param-sub';
    sub.textContent = '改动实时生效，无需重启';
    titles.appendChild(title);
    titles.appendChild(sub);

    const close = document.createElement('button');
    close.className = 'param-close';
    close.textContent = '×';
    close.title = '关闭（Esc）';
    close.addEventListener('click', () => this.close());

    head.appendChild(titles);
    head.appendChild(close);

    const body = document.createElement('div');
    body.className = 'param-body';

    for (const section of CONFIG_SECTIONS) {
      const fields = CONFIG_FIELDS[section.key];
      if (!fields) continue;
      body.appendChild(this._buildGroup(section, fields));
    }

    root.appendChild(head);
    root.appendChild(body);
    this.container.appendChild(root);

    this.root = root;
    this.body = body;
  }

  _buildGroup(section, fields) {
    const group = document.createElement('details');
    group.className = 'param-group';
    // 默认只展开前两组：全展开会有两千多像素高，找参数比调参数还费劲
    group.open = section.key === 'world' || section.key === 'life';

    const summary = document.createElement('summary');
    summary.className = 'param-group-head';

    const label = document.createElement('span');
    label.textContent = section.label;
    summary.appendChild(label);

    const reset = document.createElement('button');
    reset.className = 'param-reset';
    reset.type = 'button';
    reset.textContent = '恢复默认';
    reset.title = `把「${section.label}」组的参数恢复为出厂默认值`;
    reset.addEventListener('click', (e) => {
      e.preventDefault();          // 别让点击冒泡成 details 的开合
      e.stopPropagation();
      this.resetSection(section.key);
    });
    summary.appendChild(reset);

    group.appendChild(summary);

    const rows = document.createElement('div');
    rows.className = 'param-rows';
    for (const key of Object.keys(fields)) {
      rows.appendChild(this._buildRow(section.key, key, fields[key]));
    }
    group.appendChild(rows);

    this._groups.set(section.key, group);
    return group;
  }

  _buildRow(sectionKey, key, meta) {
    const id = `${sectionKey}.${key}`;
    const row = document.createElement('div');
    row.className = 'param-row';

    const head = document.createElement('div');
    head.className = 'param-row-head';

    const label = document.createElement('span');
    label.className = 'param-label';
    label.textContent = meta.label;
    head.appendChild(label);

    const valueEl = document.createElement('span');
    valueEl.className = 'param-value';
    head.appendChild(valueEl);

    row.appendChild(head);

    if (meta.type === 'bool') {
      row.classList.add('param-row-bool');

      const toggle = document.createElement('button');
      toggle.className = 'param-switch';
      toggle.type = 'button';
      toggle.setAttribute('role', 'switch');

      const sync = () => {
        const on = !!readPath(this.config[sectionKey], key);
        toggle.classList.toggle('is-on', on);
        toggle.setAttribute('aria-checked', String(on));
        valueEl.textContent = on ? '开' : '关';
      };

      toggle.addEventListener('click', () => {
        writePath(this.config[sectionKey], key, !readPath(this.config[sectionKey], key));
        sync();
        this._emit(sectionKey, key);
      });

      row.appendChild(toggle);
      sync();
      this._rows.set(id, { sync, kind: 'bool' });
    } else {
      const decimals = decimalsOf(meta.step);
      const isDeferredResize = meta.deferred === 'resize';

      const readValue = () => (this._pending.has(id)
        ? this._pending.get(id)
        : readPath(this.config[sectionKey], key));

      if (meta.readonly) {
        row.classList.add('param-row-readonly');
        const note = document.createElement('div');
        note.className = 'param-note';
        note.textContent = meta.note || '只读';
        row.appendChild(note);
        this._rows.set(id, {
          kind: 'readonly',
          sync: () => { valueEl.textContent = String(readPath(this.config[sectionKey], key)); },
        });
      } else {
        const slider = document.createElement('input');
        slider.type = 'range';
        slider.className = 'param-slider';
        slider.min = String(meta.min);
        slider.max = String(meta.max);
        slider.step = String(meta.step);
        slider.value = String(readPath(this.config[sectionKey], key));

        const sync = () => {
          const v = readValue();
          slider.value = String(v);
          valueEl.textContent = decimals > 0 ? Number(v).toFixed(decimals) : String(v);
        };

        slider.addEventListener('input', () => {
          const v = Number(slider.value);
          if (isDeferredResize) {
            this._pending.set(id, v);
            valueEl.textContent = decimals > 0 ? v.toFixed(decimals) : String(v);
            if (this.onDeferredChange) this.onDeferredChange(sectionKey, key, v);
            return;
          }
          this._write(sectionKey, key, v);
          sync();
          this._emit(sectionKey, key);
        });

        row.appendChild(slider);

        if (meta.deferred) {
          const apply = document.createElement('button');
          apply.className = 'param-apply';
          apply.type = 'button';
          apply.textContent = isDeferredResize ? '应用并重建世界' : '应用';
          apply.addEventListener('click', () => {
            if (isDeferredResize && this.onResizeRequest) this.onResizeRequest();
            else this._emit(sectionKey, key);
          });
          row.appendChild(apply);
        }

        this._rows.set(id, { sync, kind: 'number', deferred: isDeferredResize });
      }
    }

    // readonly 分支已经自己渲染过 note，别重复加一遍
    if (meta.note && !meta.readonly) {
      const note = document.createElement('div');
      note.className = 'param-note';
      note.textContent = meta.note;
      row.appendChild(note);
    }

    const entry = this._rows.get(id);
    if (entry && entry.sync) entry.sync();
    return row;
  }

  // ────────────────────────────── 行为 ──────────────────────────────

  _read(sectionKey, key) {
    return readPath(this.config[sectionKey], key);
  }

  _write(sectionKey, key, value) {
    writePath(this.config[sectionKey], key, value);
  }

  _emit(sectionKey, key) {
    if (this.onChange) this.onChange(sectionKey, key, this._read(sectionKey, key));
  }

  /** 把某一组参数恢复成出厂默认值，并逐个触发副作用 */
  resetSection(sectionKey) {
    const defaults = DEFAULT_CONFIG[sectionKey];
    const fields = CONFIG_FIELDS[sectionKey];
    if (!defaults || !fields) return 0;

    let n = 0;
    for (const key of Object.keys(fields)) {
      // 延迟生效的字段（网格尺寸）连带把待应用值一起清掉
      if (fields[key].deferred === 'resize') this._pending.delete(`${sectionKey}.${key}`);

      const fallback = readPath(defaults, key);
      if (this._read(sectionKey, key) === fallback) {
        const row = this._rows.get(`${sectionKey}.${key}`);
        if (row && row.sync) row.sync();
        continue;
      }
      this._write(sectionKey, key, fallback);
      const row = this._rows.get(`${sectionKey}.${key}`);
      if (row && row.sync) row.sync();
      this._emit(sectionKey, key);
      n++;
    }
    return n;
  }

  /** 外部改了 Config（比如读档）之后，把控件刷新回真实值 */
  syncAll() {
    this.clearPending();
    for (const [id, row] of this._rows) {
      if (!row.sync) continue;
      const dot = id.indexOf('.');
      const sectionKey = id.slice(0, dot);
      if (this.config[sectionKey] === undefined) continue;
      row.sync();
    }
  }

  /** 世界真的重建之后调用：待应用值已经变成事实了 */
  clearPending() {
    this._pending.clear();
  }

  /** 展开指定分组（例如读档后跳到"世界"组） */
  openGroup(sectionKey) {
    const g = this._groups.get(sectionKey);
    if (g) g.open = true;
  }
}
