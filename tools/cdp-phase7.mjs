/**
 * 阶段 7 浏览器端到端验证：无头 Chrome + CDP。
 *
 * 逻辑测试（phase7-test.mjs）已经覆盖了注册表、细胞属性、生命值、生物库的
 * 纯逻辑。这里只验证"在真实浏览器里能不能用"：
 *   1. 物种工坊能打开、画布画出了蓝图、点格子能放细胞
 *   2. 随机生成 / 清空 / 投放 / 存入生物库 都真的起作用
 *   3. 检视器里的"保存生物"能把个体（含大脑）收进库
 *   4. 库的 编辑 / 删除 / 导出 可用
 *   5. 参数面板多出"细胞属性"分组，改一个属性真的写进了 Config
 *   6. 全程无控制台错误
 *
 * 注意：python -m http.server 不发 Cache-Control，Chrome 会启发式缓存 JS。
 * 复用 profile 会拿到上一版代码，所以每次跑都从干净 profile 开始。
 */

import { spawn, execSync } from 'node:child_process';
import { setTimeout as sleep } from 'node:timers/promises';
import { rmSync, mkdirSync, readdirSync } from 'node:fs';

const CHROME = 'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe';
const BASE_PROFILE = 'C:\\Users\\安\\AppData\\Local\\Temp\\chrome-evosim-p7';
const DOWNLOADS = 'C:\\Users\\安\\AppData\\Local\\Temp\\evosim-downloads-p7';
const URL = 'http://127.0.0.1:8131/';
const PORT = 9228;

function sleepSync(ms) {
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);
}

function resetProfile(base) {
  for (let i = 0; i < 8; i++) {
    try { rmSync(base, { recursive: true, force: true }); return base; }
    catch { sleepSync(400); }
  }
  const alt = `${base}-${process.pid}`;
  try { rmSync(alt, { recursive: true, force: true }); } catch { /* 忽略 */ }
  return alt;
}

function forceKill(pid) {
  if (!pid) return;
  try { execSync(`taskkill /F /T /PID ${pid}`, { stdio: 'ignore' }); } catch { /* 已经退了 */ }
}

function freePort(port) {
  try {
    const out = execSync('netstat -ano -p TCP', { encoding: 'utf8' });
    const pids = new Set();
    for (const line of out.split('\n')) {
      if (!line.includes('LISTENING') || !line.includes(`:${port} `)) continue;
      const m = line.trim().match(/(\d+)\s*$/);
      if (m) pids.add(m[1]);
    }
    for (const pid of pids) forceKill(pid);
  } catch { /* 端口没被占 */ }
}

const PROFILE = resetProfile(BASE_PROFILE);
mkdirSync(PROFILE, { recursive: true });
rmSync(DOWNLOADS, { recursive: true, force: true });
mkdirSync(DOWNLOADS, { recursive: true });
freePort(PORT);

let failures = 0;
let checks = 0;
function check(name, condition, extra = '') {
  checks++;
  const ok = !!condition;
  if (!ok) failures++;
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}${extra ? '  → ' + extra : ''}`);
}

const chrome = spawn(CHROME, [
  '--headless=new', '--disable-gpu', '--no-sandbox', '--no-first-run',
  '--disable-dev-shm-usage',
  `--remote-debugging-port=${PORT}`,
  `--user-data-dir=${PROFILE}`,
  '--window-size=1600,950',
  URL,
], { stdio: 'ignore' });

async function getPageTarget() {
  for (let i = 0; i < 60; i++) {
    try {
      const res = await fetch(`http://127.0.0.1:${PORT}/json/list`);
      const list = await res.json();
      const page = list.find((t) => t.type === 'page' && t.webSocketDebuggerUrl);
      if (page) return page;
    } catch { /* 还没起来 */ }
    await sleep(250);
  }
  throw new Error('CDP 端口未就绪');
}

function makeClient(wsUrl) {
  const ws = new WebSocket(wsUrl);
  let nextId = 1;
  const pending = new Map();
  const events = [];
  ws.addEventListener('message', (ev) => {
    const msg = JSON.parse(ev.data);
    if (msg.id && pending.has(msg.id)) {
      const { resolve, reject } = pending.get(msg.id);
      pending.delete(msg.id);
      if (msg.error) reject(new Error(JSON.stringify(msg.error)));
      else resolve(msg.result);
    } else if (msg.method) events.push(msg);
  });
  const ready = new Promise((resolve, reject) => {
    ws.addEventListener('open', resolve);
    ws.addEventListener('error', () => reject(new Error('WebSocket 连接失败')));
  });
  const send = (method, params = {}) => new Promise((resolve, reject) => {
    const id = nextId++;
    pending.set(id, { resolve, reject });
    ws.send(JSON.stringify({ id, method, params }));
  });
  return { ws, ready, send, events };
}

async function main() {
  const target = await getPageTarget();
  const client = makeClient(target.webSocketDebuggerUrl);
  await client.ready;
  await client.send('Runtime.enable');
  await client.send('Log.enable');
  await client.send('Page.enable');
  await client.send('Browser.setDownloadBehavior', {
    behavior: 'allow', downloadPath: DOWNLOADS, eventsEnabled: true,
  });

  const evaluate = async (expression) => {
    const r = await client.send('Runtime.evaluate', {
      expression, returnByValue: true, awaitPromise: true,
    });
    if (r.exceptionDetails) throw new Error('页面内异常: ' + JSON.stringify(r.exceptionDetails.exception));
    return r.result.value;
  };

  const clickAt = async (x, y) => {
    await client.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y, button: 'none', buttons: 0 });
    await client.send('Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', buttons: 1, clickCount: 1 });
    await client.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x, y, button: 'left', buttons: 0, clickCount: 1 });
    await sleep(200);
  };

  const clickEl = (selector) => evaluate(
    `(() => { const el = document.querySelector(${JSON.stringify(selector)}); if (!el) return false; el.click(); return true; })()`);

  /** 点面板里文字为 text 的按钮 */
  const clickText = (selector, text) => evaluate(
    `(() => {
       const els = [...document.querySelectorAll(${JSON.stringify(selector)})];
       const el = els.find((b) => b.textContent.trim() === ${JSON.stringify(text)});
       if (!el) return false; el.click(); return true;
     })()`);

  await sleep(3000);

  // ── 1. 启动状态 ──
  console.log('=== 1. 启动状态 ===');
  const boot = await evaluate(`(() => {
    const { speciesEditor, library, world, Config } = window.evosim;
    return {
      tick: world.tick,
      organisms: world.organisms.length,
      badge: document.getElementById('stage-badge').textContent,
      hasEditorBtn: !!document.getElementById('btn-species-editor'),
      editorOpen: speciesEditor.isOpen,
      libraryCount: library.count,
      cellAttrs: Object.keys(Config.cells).length,
    };
  })()`);
  check('页面跑起来了', boot.tick >= 0, `tick ${boot.tick}`);
  check('世界里有生物', boot.organisms > 0, `${boot.organisms} 个`);
  check('徽章已填充阶段号', /^阶段 \d/.test(boot.badge), boot.badge);
  check('顶栏有"物种工坊"按钮', boot.hasEditorBtn === true);
  check('启动时物种工坊是关的', boot.editorOpen === false);
  check('启动时生物库是空的', boot.libraryCount === 0);
  check('Config 里有 9 种身体细胞的属性', boot.cellAttrs === 9, `${boot.cellAttrs} 种`);

  // ── 2. 打开物种工坊 ──
  console.log('\n=== 2. 打开物种工坊 ===');
  await clickEl('#btn-species-editor');
  await sleep(300);
  const opened = await evaluate(`(() => {
    const { speciesEditor } = window.evosim;
    const canvas = document.querySelector('.se-canvas');
    const ctx = canvas.getContext('2d');
    const img = ctx.getImageData(0, 0, canvas.width, canvas.height).data;
    let ink = 0;
    for (let i = 0; i < img.length; i += 4) {
      if (img[i] > 30 || img[i + 1] > 30 || img[i + 2] > 60) ink++;
    }
    const palette = document.querySelectorAll('.se-swatch').length;
    return {
      isOpen: speciesEditor.isOpen,
      hidden: speciesEditor.root.hidden,
      cells: speciesEditor.cells.size,
      ink,
      palette,
      hasNameInput: !!document.querySelector('.se-name-input'),
      libraryEntries: document.querySelectorAll('.se-entry').length,
      info: speciesEditor.infoEl.textContent,
    };
  })()`);
  check('物种工坊打开了', opened.isOpen === true && opened.hidden === false);
  check('蓝图画布画出了内容', opened.ink > 500, `${opened.ink} px`);
  check('默认蓝图是 3 个细胞', opened.cells === 3, `${opened.cells} 个`);
  check('细胞类型调色板有 10 项（9 种细胞 + 擦除）', opened.palette === 10, `${opened.palette} 项`);
  check('有名字输入框', opened.hasNameInput === true);
  check('空库显示占位提示', opened.libraryEntries === 0);
  check('信息栏显示描述与指纹', /细胞.*指纹/.test(opened.info), opened.info);

  // ── 3. 在画布上画细胞 ──
  console.log('\n=== 3. 手绘蓝图 ===');
  const paint = await evaluate(`(() => {
    const { speciesEditor } = window.evosim;
    const canvas = document.querySelector('.se-canvas');
    const rect = canvas.getBoundingClientRect();
    const R = 7;
    const cellPx = rect.width / (R * 2 + 1);
    const at = (dx, dy) => ({
      x: rect.left + (dx + R + 0.5) * cellPx,
      y: rect.top + (dy + R + 0.5) * cellPx,
    });
    return { before: speciesEditor.cells.size, p1: at(2, 0), p2: at(2, 1) };
  })()`);
  await clickAt(paint.p1.x, paint.p1.y);
  await clickAt(paint.p2.x, paint.p2.y);
  const painted = await evaluate(`(() => {
    const { speciesEditor } = window.evosim;
    return { cells: speciesEditor.cells.size, has20: speciesEditor.cells.has('2,0'),
             has21: speciesEditor.cells.has('2,1') };
  })()`);
  check('点画布放上了细胞', painted.cells === paint.before + 2,
    `${paint.before} → ${painted.cells}`);
  check('放到了点击的位置', painted.has20 && painted.has21);

  // 切换笔刷成"移动器"再画一格
  await clickText('.se-swatch', '移动器');
  await sleep(120);
  const p3 = await evaluate(`(() => {
    const canvas = document.querySelector('.se-canvas');
    const rect = canvas.getBoundingClientRect();
    const cellPx = rect.width / 15;
    return { x: rect.left + (3 + 7.5) * cellPx, y: rect.top + (0 + 7.5) * cellPx };
  })()`);
  await clickAt(p3.x, p3.y);
  const brushChanged = await evaluate(`(() => {
    const { speciesEditor } = window.evosim;
    const c = speciesEditor.cells.get('3,0');
    return { type: c ? c.type : null, brush: speciesEditor.brush };
  })()`);
  check('切换笔刷后画出的是新类型', brushChanged.type === 5, `type=${brushChanged.type}`);

  // ── 4. 随机生成 / 清空 ──
  console.log('\n=== 4. 随机生成与清空 ===');
  await clickText('.se-btn', '随机生成');
  await sleep(150);
  const random = await evaluate(`(() => {
    const { speciesEditor } = window.evosim;
    return { cells: speciesEditor.cells.size, hasCenter: speciesEditor.cells.has('0,0') };
  })()`);
  check('随机生成出至少 1 个细胞', random.cells >= 1, `${random.cells} 个`);
  check('随机生成后中心格仍在', random.hasCenter === true);

  await clickText('.se-btn', '清空');
  await sleep(150);
  const cleared = await evaluate(`(() => {
    const { speciesEditor } = window.evosim;
    return { cells: speciesEditor.cells.size, hasCenter: speciesEditor.cells.has('0,0') };
  })()`);
  check('清空后只剩中心格', cleared.cells === 1 && cleared.hasCenter === true);

  // ── 5. 投放：放到视野中心 + 手动放置 ──
  console.log('\n=== 5. 投放 ===');
  const spawn = await evaluate(`(() => {
    const { world, engine, camera, renderer } = window.evosim;
    engine.pause();
    // 把视野中心挪到一块空地，保证放得下
    const grid = world.grid;
    let spot = null;
    for (let row = 6; row < grid.rows - 6 && !spot; row += 2) {
      for (let col = 6; col < grid.cols - 6; col += 2) {
        let clear = true;
        for (let dr = -3; dr <= 3 && clear; dr++)
          for (let dc = -3; dc <= 3; dc++)
            if (grid.types[grid.index(col + dc, row + dr)] !== 0) { clear = false; break; }
        if (clear) { spot = [col, row]; break; }
      }
    }
    camera.x = spot[0] - renderer.width / (2 * camera.zoom);
    camera.y = spot[1] - renderer.height / (2 * camera.zoom);
    return { before: world.organisms.length };
  })()`);
  await clickText('.se-btn', '放到视野中心');
  await sleep(250);
  const centered = await evaluate(`(() => {
    const { world } = window.evosim;
    const last = world.organisms[world.organisms.length - 1];
    return { after: world.organisms.length, lastFp: last ? last.speciesFingerprint : null };
  })()`);
  check('"放到视野中心"让世界多了一个个体', centered.after === spawn.before + 1,
    `${spawn.before} → ${centered.after}`);

  // ── 5b. 手动放置 ──
  console.log('\n=== 5b. 手动放置 ===');
  await clickText('.se-btn', '手动放置');
  await sleep(200);
  const armed = await evaluate(`(() => {
    const { editor, speciesEditor } = window.evosim;
    return {
      placing: editor.isPlacing,
      brush: editor.brushType,
      hasBlueprint: !!editor.cloneBlueprint,
      editorOpen: speciesEditor.isOpen,
      btnActive: document.getElementById('btn-species-editor').classList.contains('is-active'),
      count: window.evosim.world.organisms.length,
    };
  })()`);
  check('"手动放置"进入了放置模式', armed.placing === true && armed.hasBlueprint === true,
    `笔刷=${armed.brush}`);
  check('手动放置会收起工坊、让出画布', armed.editorOpen === false);
  check('顶栏工坊按钮同步取消高亮', armed.btnActive === false);

  // 找一块空地作为落点，用真实鼠标点下去
  const placeSpot = await evaluate(`(() => {
    const { world, camera } = window.evosim;
    const canvas = document.getElementById('world-canvas');
    const rect = canvas.getBoundingClientRect();
    const grid = world.grid;
    for (let row = 8; row < grid.rows - 8; row += 3) {
      for (let col = 8; col < grid.cols - 8; col += 3) {
        let clear = true;
        for (let dr = -4; dr <= 4 && clear; dr++)
          for (let dc = -4; dc <= 4; dc++)
            if (grid.types[grid.index(col + dc, row + dr)] !== 0) { clear = false; break; }
        if (!clear) continue;
        const sx = camera.worldToScreenX(col) + camera.zoom / 2;
        const sy = camera.worldToScreenY(row) + camera.zoom / 2;
        if (sx < 8 || sy < 8 || sx > rect.width - 8 || sy > rect.height - 8) continue;
        return { x: rect.left + sx, y: rect.top + sy, col, row };
      }
    }
    return null;
  })()`);
  check('能找到一块可放置的空地', !!placeSpot,
    placeSpot ? `(${placeSpot.col}, ${placeSpot.row})` : '没找到');

  await clickAt(placeSpot.x, placeSpot.y);
  const placed = await evaluate(`(() => {
    const { world, editor } = window.evosim;
    let near = 0;
    for (const o of world.organisms) {
      if (Math.abs(o.col - ${placeSpot.col}) <= 6 && Math.abs(o.row - ${placeSpot.row}) <= 6) near++;
    }
    return { after: world.organisms.length, near, placing: editor.isPlacing };
  })()`);
  check('点画布真的把个体放到了那个位置',
    placed.after === centered.after + 1 && placed.near > 0,
    `${centered.after} → ${placed.after}，落点附近 ${placed.near} 个`);
  check('放完之后仍在放置模式（可以接着放）', placed.placing === true);

  // Esc 退出放置模式
  await client.send('Input.dispatchKeyEvent', {
    type: 'rawKeyDown', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27, nativeVirtualKeyCode: 27,
  });
  await client.send('Input.dispatchKeyEvent', {
    type: 'keyUp', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27, nativeVirtualKeyCode: 27,
  });
  await sleep(250);
  const afterEscPlace = await evaluate(`(() => {
    const { editor } = window.evosim;
    return { placing: editor.isPlacing, brush: editor.brushType };
  })()`);
  check('Esc 退出放置模式', afterEscPlace.placing === false, `笔刷=${afterEscPlace.brush}`);

  // ── 6. 存入生物库 ──
  console.log('\n=== 6. 生物库：存入 / 编辑 / 删除 ===');
  await evaluate(`(() => {
    const { speciesEditor } = window.evosim;
    speciesEditor.nameInput.value = '手绘小虫';
    speciesEditor.nameInput.dispatchEvent(new Event('input', { bubbles: true }));
  })()`);
  await clickText('.se-btn', '存入生物库');
  await sleep(200);
  const saved = await evaluate(`(() => {
    const { library } = window.evosim;
    const rows = [...document.querySelectorAll('.se-entry')];
    return {
      count: library.count,
      rows: rows.length,
      firstName: rows.length ? rows[0].querySelector('.se-entry-name').textContent : null,
      firstMeta: rows.length ? rows[0].querySelector('.se-entry-meta').textContent : null,
    };
  })()`);
  check('存入生物库后库里有 1 条', saved.count === 1 && saved.rows === 1, `${saved.count} 条`);
  check('库里的名字正确', saved.firstName === '手绘小虫', saved.firstName);
  check('库里没有大脑标记（手绘）', !/带大脑/.test(saved.firstMeta), saved.firstMeta);

  // 编辑：先改蓝图再点"编辑"，蓝图应被载回
  await clickText('.se-btn', '清空');
  await sleep(120);
  await clickText('.se-btn', '编辑');
  await sleep(150);
  const loaded = await evaluate(`(() => {
    const { speciesEditor } = window.evosim;
    return { cells: speciesEditor.cells.size, name: speciesEditor.nameInput.value };
  })()`);
  check('点"编辑"把记录载回蓝图', loaded.cells === 1 && loaded.name === '手绘小虫',
    `${loaded.cells} 细胞 · ${loaded.name}`);

  // ── 7. 检视器"保存生物"（带大脑）──
  console.log('\n=== 7. 检视器保存生物 ===');
  const saveFromInspector = await evaluate(`(() => {
    const { world, editor, inspector, library } = window.evosim;
    const org = world.organisms[0];
    editor.onInspect(org);
    const before = library.count;
    // 点检视器头部的"保存生物"
    const btn = [...document.querySelectorAll('.inspector-action')]
      .find((b) => b.textContent.trim() === '保存生物');
    if (btn) btn.click();
    return { before, after: library.count, orgId: org.id, fp: org.speciesFingerprint };
  })()`);
  check('检视器里有"保存生物"按钮并能点', saveFromInspector.after === saveFromInspector.before + 1,
    `${saveFromInspector.before} → ${saveFromInspector.after}`);
  const brainSaved = await evaluate(`(() => {
    const { library } = window.evosim;
    const last = library.entries[library.entries.length - 1];
    return { name: last.name, hasBrain: !!last.brain, cells: last.cells.length };
  })()`);
  check('从检视器存的记录带大脑', brainSaved.hasBrain === true, brainSaved.name);
  check('从检视器存的记录有身体', brainSaved.cells > 0, `${brainSaved.cells} 细胞`);

  // ── 8. 导出生物库 ──
  console.log('\n=== 8. 导出生物库 ===');
  await evaluate(`window.evosim.speciesEditor.refreshLibrary()`);
  await clickText('.se-btn', '导出文件');
  let files = [];
  for (let i = 0; i < 40 && files.length === 0; i++) {
    await sleep(250);
    files = readdirSync(DOWNLOADS).filter((f) => f.startsWith('evosim-creatures'));
  }
  check('点"导出文件"下载了 JSON', files.length > 0, files[0] || '没等到文件');

  // ── 9. 参数面板的"细胞属性"分组 ──
  console.log('\n=== 9. 参数面板：细胞属性 ===');
  await evaluate(`window.evosim.speciesEditor.close()`);
  await clickEl('#btn-params');
  await sleep(250);
  const panel = await evaluate(`(() => {
    const groups = [...document.querySelectorAll('.param-group')];
    const cellsGroup = groups.find((g) => g.querySelector('.param-group-head span').textContent === '细胞属性');
    if (!cellsGroup) return { found: false };
    cellsGroup.open = true;
    const labels = [...cellsGroup.querySelectorAll('.param-label')].map((e) => e.textContent);
    const sliders = cellsGroup.querySelectorAll('.param-slider');
    return { found: true, rows: labels.length, sliders: sliders.length, sample: labels.slice(0, 5) };
  })()`);
  check('参数面板有"细胞属性"分组', panel.found === true);
  check('分组里有 9 种 × 5 项 = 45 行', panel.rows === 45, `${panel.rows} 行`);
  check('每行都有滑块', panel.sliders === 45, `${panel.sliders} 个`);
  check('标签带细胞类型前缀', panel.found && /·/.test(panel.sample[0]), panel.sample.join(' / '));

  // 真的改一个属性
  const changed = await evaluate(`(() => {
    const { Config } = window.evosim;
    const before = Config.cells.KILLER.damage;
    const groups = [...document.querySelectorAll('.param-group')];
    const cellsGroup = groups.find((g) => g.querySelector('.param-group-head span').textContent === '细胞属性');
    cellsGroup.open = true;
    const row = [...cellsGroup.querySelectorAll('.param-row')]
      .find((r) => r.querySelector('.param-label').textContent === '杀手 · 伤害');
    const slider = row.querySelector('.param-slider');
    slider.value = '7';
    slider.dispatchEvent(new Event('input', { bubbles: true }));
    return { before, after: Config.cells.KILLER.damage };
  })()`);
  check('拖动滑块真的写进了 Config.cells', changed.after === 7,
    `KILLER.damage ${changed.before} → ${changed.after}`);

  // 恢复默认
  const reset = await evaluate(`(() => {
    const { Config } = window.evosim;
    const groups = [...document.querySelectorAll('.param-group')];
    const cellsGroup = groups.find((g) => g.querySelector('.param-group-head span').textContent === '细胞属性');
    const btn = cellsGroup.querySelector('.param-reset');
    btn.click();
    return { damage: Config.cells.KILLER.damage, hp: Config.cells.MOUTH.hp };
  })()`);
  check('"恢复默认"把细胞属性还原', reset.damage === 1, `KILLER.damage=${reset.damage}`);

  // ── 10. Esc 关闭 ──
  console.log('\n=== 10. Esc 关闭 ===');
  await evaluate(`window.evosim.controlPanel.close()`);
  await evaluate(`window.evosim.inspector.close()`);
  await clickEl('#btn-species-editor');
  await sleep(200);
  const reopened = await evaluate(`window.evosim.speciesEditor.isOpen`);
  check('重新打开物种工坊', reopened === true);

  await client.send('Input.dispatchKeyEvent', {
    type: 'rawKeyDown', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27, nativeVirtualKeyCode: 27,
  });
  await client.send('Input.dispatchKeyEvent', {
    type: 'keyUp', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27, nativeVirtualKeyCode: 27,
  });
  await sleep(300);
  const afterEsc = await evaluate(`(() => {
    const { speciesEditor } = window.evosim;
    return { isOpen: speciesEditor.isOpen, hidden: speciesEditor.root.hidden };
  })()`);
  check('Esc 关闭物种工坊', afterEsc.isOpen === false && afterEsc.hidden === true);

  // ── 11. 性能 ──
  console.log('\n=== 11. 性能 ===');
  const perf = await evaluate(`(() => {
    const { engine, world } = window.evosim;
    engine.setSpeed(1);
    engine.start();
    return new Promise((resolve) => {
      let bestFps = 0;
      let bestTps = 0;
      let n = 0;
      const tick = () => {
        n++;
        if (engine.fps > bestFps) bestFps = engine.fps;
        if (engine.tps > bestTps) bestTps = engine.tps;
        if (n < 240) requestAnimationFrame(tick);
        else resolve({ fps: bestFps, tps: bestTps, running: engine.running,
                       organisms: world.organisms.length });
      };
      requestAnimationFrame(tick);
    });
  })()`);
  check('引擎在跑', perf.running === true);
  check('帧率健康', perf.fps >= 50, `${perf.fps.toFixed(1)} fps`);
  check('tick 速率达标', perf.tps >= 55, `${perf.tps.toFixed(1)} tick/s`);

  // ── 12. 控制台 ──
  console.log('\n=== 12. 控制台 ===');
  const errors = client.events.filter((e) =>
    (e.method === 'Log.entryAdded' && e.params.entry.level === 'error')
    || e.method === 'Runtime.exceptionThrown');
  check('无控制台错误', errors.length === 0,
    errors.length ? errors.map((e) => JSON.stringify(e.params).slice(0, 240)).join(' | ') : '干净');

  console.log(`\n${'='.repeat(52)}`);
  if (failures === 0) console.log(`全部通过 ✓  (${checks} 项)`);
  else console.log(`通过 ${checks - failures} 项，失败 ${failures} 项 ✗`);

  client.ws.close();
  forceKill(chrome.pid);
  process.exit(failures === 0 ? 0 : 1);
}

main().catch((err) => {
  console.error('测试崩溃:', err.message);
  forceKill(chrome.pid);
  process.exit(1);
});
