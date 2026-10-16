/**
 * 单文件产物验证：确认 dist/evosim.html 真的能独立运行。
 *
 * 支持两种目标：
 *   node tools/verify-bundle.mjs                       → 用 file:// 打开 dist/evosim.html
 *   node tools/verify-bundle.mjs http://127.0.0.1:PORT/  → 打开一个已起的服务（验证 exe）
 *
 * 这是"双击即玩"这条交付路径的验收：没有服务器、没有模块请求，
 * 全部逻辑都在一个 HTML 里。重点看：
 *   1. 页面无控制台错误地跑起来（ES Modules 若没打干净，file:// 下会直接白屏）
 *   2. 世界在演化、画布真的画出了东西
 *   3. 存档/读档、参数面板、检视器这些交互都还在
 *   4. localStorage 不可用时，快存走的是友好报错而不是未捕获异常
 */

import { spawn } from 'node:child_process';
import { setTimeout as sleep } from 'node:timers/promises';
import { rmSync, mkdirSync, existsSync } from 'node:fs';
import { resolve, dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const ARG = process.argv[2];
const IS_HTTP = !!ARG && /^https?:\/\//.test(ARG);
const HTML = join(ROOT, 'dist/evosim.html');
const CHROME = 'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe';
const PROFILE = 'C:\\Users\\安\\AppData\\Local\\Temp\\chrome-evosim-bundle';
const PORT = 9232;

if (!IS_HTTP && !existsSync(HTML)) {
  console.error(`找不到产物：${HTML}\n先跑 node tools/bundle.mjs`);
  process.exit(1);
}

rmSync(PROFILE, { recursive: true, force: true });
mkdirSync(PROFILE, { recursive: true });

let failures = 0;
let checks = 0;
function check(name, condition, extra = '') {
  checks++;
  const ok = !!condition;
  if (!ok) failures++;
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}${extra ? '  → ' + extra : ''}`);
}

const targetUrl = IS_HTTP ? ARG : pathToFileURL(HTML).href;
console.log(`打开：${targetUrl}\n`);

const chrome = spawn(CHROME, [
  '--headless=new', '--disable-gpu', '--no-sandbox', '--no-first-run',
  '--disable-dev-shm-usage',
  `--remote-debugging-port=${PORT}`,
  `--user-data-dir=${PROFILE}`,
  '--window-size=1440,900',
  targetUrl,
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
      const { resolve: res, reject } = pending.get(msg.id);
      pending.delete(msg.id);
      if (msg.error) reject(new Error(JSON.stringify(msg.error)));
      else res(msg.result);
    } else if (msg.method) events.push(msg);
  });
  const ready = new Promise((res, rej) => {
    ws.addEventListener('open', res);
    ws.addEventListener('error', () => rej(new Error('WebSocket 连接失败')));
  });
  const send = (method, params = {}) => new Promise((res, rej) => {
    const id = nextId++;
    pending.set(id, { resolve: res, reject: rej });
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

  const evaluate = async (expression) => {
    const r = await client.send('Runtime.evaluate', {
      expression, returnByValue: true, awaitPromise: true,
    });
    if (r.exceptionDetails) throw new Error('页面内异常: ' + JSON.stringify(r.exceptionDetails.exception));
    return r.result.value;
  };

  await sleep(3000);

  // ── 1. 单文件里模块真的加载了 ──
  console.log('=== 1. 单文件加载 ===');
  const boot = await evaluate(`(() => {
    if (!window.evosim) return { ok: false };
    const { world } = window.evosim;
    return {
      ok: true,
      tick: world.tick,
      organisms: world.organisms.length,
      badge: document.getElementById('stage-badge').textContent,
      moduleCount: document.querySelectorAll('script[src]').length,
      hasStyle: !!document.querySelector('style'),
      protocol: location.protocol,
    };
  })()`);
  check('打包产物正常启动', boot.ok === true);
  check('没有外部脚本请求（全内联）', boot.moduleCount === 0, `${boot.moduleCount} 个外链脚本`);
  check('样式已内联', boot.hasStyle === true);
  check(`运行在 ${IS_HTTP ? 'http' : 'file'} 协议下`,
    IS_HTTP ? boot.protocol === 'http:' : boot.protocol === 'file:', boot.protocol);
  check('世界里有生物', boot.organisms > 0, `${boot.organisms} 个`);
  check('徽章已填充阶段号', /^阶段 \d/.test(boot.badge), boot.badge);

  // ── 2. 演化 + 渲染 ──
  console.log('\n=== 2. 演化与渲染 ===');
  await evaluate(`window.evosim.engine.setSpeed(50)`);
  await sleep(4000);
  await evaluate(`window.evosim.engine.setSpeed(1)`);
  await sleep(500);

  const evolved = await evaluate(`(() => {
    const { world, renderer } = window.evosim;
    const c = document.getElementById('world-canvas');
    const img = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
    let ink = 0;
    for (let i = 0; i < img.length; i += 4) if (img[i + 3] > 8) ink++;
    return { tick: world.tick, organisms: world.organisms.length, ink, w: c.width, h: c.height };
  })()`);
  check('模拟推进了', evolved.tick > 400, `tick ${evolved.tick}`);
  check('画布画出了内容', evolved.ink > 1000, `${evolved.ink} px（${evolved.w}×${evolved.h}）`);
  check('种群没有崩', evolved.organisms > 0, `${evolved.organisms} 个`);

  // ── 3. 存档 / 读档（不走 localStorage）──
  console.log('\n=== 3. 存档 / 读档 ===');
  const saveLoad = await evaluate(`(() => {
    const { world } = window.evosim;
    const before = { tick: world.tick, organisms: world.organisms.length };
    const data = window.evosim.save();
    world.clearOrganisms();
    const cleared = world.organisms.length;
    const info = window.evosim.load(data);
    return {
      before, cleared,
      format: data.format,
      after: { tick: world.tick, organisms: world.organisms.length },
      infoOrganisms: info.organisms,
    };
  })()`);
  check('存档结构合法', saveLoad.format === 'evosim-world', saveLoad.format);
  check('清空生效', saveLoad.cleared === 0);
  check('读档还原了 tick 与种群',
    saveLoad.after.tick === saveLoad.before.tick && saveLoad.after.organisms === saveLoad.before.organisms,
    `tick ${saveLoad.after.tick} · ${saveLoad.after.organisms} 个体`);

  // ── 4. localStorage 不可用时的降级 ──
  console.log('\n=== 4. 本地快存的降级 ===');
  const storage = await evaluate(`(() => {
    let available, err = null;
    try { available = window.evosim.storageAvailable(); } catch (e) { err = String(e); }
    let saveErr = null;
    try { window.evosim.quickSave(); } catch (e) { saveErr = e.message; }
    let infoErr = null;
    let info = null;
    try { info = window.evosim.quickInfo(); } catch (e) { infoErr = String(e); }
    return { available, err, saveErr, infoErr, info };
  })()`);
  check('探测 localStorage 可用性不抛异常', storage.err === null, storage.err || 'ok');
  if (storage.available) {
    check('localStorage 可用时快存成功', storage.saveErr === null, storage.saveErr || 'ok');
  } else {
    check('不可用时快存给出友好提示而非崩溃',
      typeof storage.saveErr === 'string' && storage.saveErr.includes('保存到文件'), storage.saveErr || '（没抛错）');
  }
  check('快存信息查询不抛异常', storage.infoErr === null, storage.infoErr || 'ok');

  // ── 5. 参数面板与检视器仍在 ──
  console.log('\n=== 5. 交互仍然完整 ===');
  const ui = await evaluate(`(() => {
    const { controlPanel, editor, inspector, world, Config } = window.evosim;
    document.getElementById('btn-params').click();
    const panelOpen = controlPanel.isOpen;
    const rows = document.querySelectorAll('.param-row').length;
    const labels = [...document.querySelectorAll('.param-label')].map((l) => l.textContent);
    controlPanel.close();

    const src = world.organisms[0];
    editor.onInspect(src);
    const inspectorOpen = inspector.isOpen;
    const orgId = inspector.organism ? inspector.organism.id : null;
    inspector.close();

    // 橡皮擦笔刷
    const brushes = [...document.querySelectorAll('.swatch-label')].map((s) => s.textContent);
    const hasEraser = brushes.includes('橡皮擦');
    editor.setBrushType('eraser');
    const eraserType = editor.brushType;
    // 橡皮擦擦一堵墙：墙没了，生物不受影响
    const grid = world.grid;
    let wallIdx = -1;
    for (let i = 0; i < grid.length; i++) {
      if (grid.types[i] === 2 && grid.owners[i] < 0) { wallIdx = i; break; }
    }
    let erased = null;
    if (wallIdx >= 0) {
      const [c, r] = grid.coords(wallIdx);
      const orgBefore = world.organisms.length;
      editor.setBrushSize(1);
      editor._eraseCell(c, r);
      erased = { gone: grid.types[wallIdx] === 0, orgSame: world.organisms.length === orgBefore };
    }
    editor.setBrushType(2);

    return { panelOpen, rows, inspectorOpen, orgId, srcId: src.id,
             labels, hasEraser, eraserType, erased,
             burnLabel: labels.includes('全局消耗量倍率'),
             foodLabel: labels.includes('食物量倍率'),
             foodScale: Config.life.foodEnergyScale };
  })()`);
  check('参数抽屉能打开且有控件', ui.panelOpen === true && ui.rows > 30, `${ui.rows} 行`);
  check('检视器能打开并选中正确个体', ui.inspectorOpen === true && ui.orgId === ui.srcId,
    `id=${ui.orgId}`);
  check('工具栏里有「橡皮擦」笔刷且可选中', ui.hasEraser === true && ui.eraserType === 'eraser');
  check('橡皮擦擦掉地形但不影响生物',
    ui.erased && ui.erased.gone === true && ui.erased.orgSame === true,
    ui.erased ? `墙被清除 ${ui.erased.gone} · 生物数不变 ${ui.erased.orgSame}` : '没找到可擦的墙');
  check('参数面板有「全局消耗量倍率」与「食物量倍率」',
    ui.burnLabel === true && ui.foodLabel === true && ui.foodScale === 1,
    `食物量倍率默认 ${ui.foodScale}`);

  // ── 5b. 阶段 6：物种树 / 导出接口 / 着色模式 ──
  console.log('\n=== 5b. 物种树与导出 ===');
  const p6 = await evaluate(`(() => {
    const { speciesTree, renderer } = window.evosim;
    speciesTree.open();
    const c = speciesTree.canvas;
    const img = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
    let ink = 0;
    for (let i = 0; i < img.length; i += 4) {
      if (Math.abs(img[i] - 13) > 12 || Math.abs(img[i + 1] - 17) > 12 || Math.abs(img[i + 2] - 23) > 12) ink++;
    }
    const nodes = speciesTree._layout ? speciesTree._layout.order.length : 0;
    // 非模态浮窗：外层不拦事件、面板可交互、没有全屏遮罩、默认锁定
    const root = speciesTree.root;
    const panel = root.querySelector('.species-tree-panel');
    const rootPE = getComputedStyle(root).pointerEvents;
    const panelPE = getComputedStyle(panel).pointerEvents;
    const hasBackdrop = !!root.querySelector('.species-tree-backdrop');
    const pinned = speciesTree.pinned;
    speciesTree.close();
    const hasExport = typeof window.evosim.exportFossil === 'function'
      && typeof window.evosim.exportSpecies === 'function';
    renderer.setColorMode('species');
    const speciesMode = renderer.colorMode;
    renderer.setColorMode('type');
    return { ink, nodes, hasExport, speciesMode, backTo: renderer.colorMode,
             rootPE, panelPE, hasBackdrop, pinned,
             closed: speciesTree.isOpen };
  })()`);
  check('物种树能打开并画出内容', p6.ink > 500 && p6.nodes > 0,
    `${p6.nodes} 个节点 · ${p6.ink} px`);
  check('物种树是不挡画布的非模态浮窗',
    p6.rootPE === 'none' && p6.panelPE === 'auto' && p6.hasBackdrop === false,
    `外层 ${p6.rootPE} / 面板 ${p6.panelPE} / 遮罩 ${p6.hasBackdrop}`);
  check('物种树默认锁定（点画布不关闭）', p6.pinned === true);
  check('物种树能关闭', p6.closed === false);
  check('导出接口可用', p6.hasExport === true);
  check('着色模式可切换且能切回', p6.speciesMode === 'species' && p6.backTo === 'type');

  // ── 5c. 阶段 7：物种工坊 / 细胞属性 / 生物库 ──
  console.log('\n=== 5c. 物种工坊与细胞属性 ===');
  const p7 = await evaluate(`(() => {
    const { speciesEditor, library, Config, world } = window.evosim;
    speciesEditor.open();
    const c = speciesEditor.canvas;
    const img = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
    let ink = 0;
    for (let i = 0; i < img.length; i += 4) {
      if (img[i] > 30 || img[i + 1] > 30 || img[i + 2] > 60) ink++;
    }
    const cells = speciesEditor.cells.size;

    // 投放一份蓝图（走面板自己的投放回调），确认世界真的多了一个个体
    const before = world.organisms.length;
    speciesEditor.onSpawn(speciesEditor._toBlueprint(), '打包测试');
    const placed = world.organisms.length - before;

    // 三个投放/保存按钮都在
    const btns = [...speciesEditor.root.querySelectorAll('.se-btn')].map((b) => b.textContent.trim());
    const hasManual = btns.includes('手动放置');
    const hasCenter = btns.includes('放到视野中心');
    const hasLibrary = btns.includes('存入生物库');

    // 手动放置：装入放置笔刷并收起工坊
    speciesEditor._placeManual();
    const { editor } = window.evosim;
    const manualArmed = editor.isPlacing === true && speciesEditor.isOpen === false;
    editor.cancelClone();

    // 存入生物库
    speciesEditor.open();
    const before2 = library.count;
    speciesEditor._saveToLibrary();
    const after2 = library.count;

    // 细胞属性分组
    const groups = [...document.querySelectorAll('.param-group')];
    const cellsGroup = groups.find((g) => g.querySelector('.param-group-head span').textContent === '细胞属性');
    const attrRows = cellsGroup ? cellsGroup.querySelectorAll('.param-row').length : 0;

    // 导出落点：浏览器里没有宿主标记，应回退成下载
    const hostDir = window.evosim.hostSaveDir();
    const marker = window.__EVOSIM_HOST__ || null;

    speciesEditor.close();
    return { ink, cells, placed, libBefore: before2,
             libAfter: after2, attrRows, attrKinds: Object.keys(Config.cells).length,
             hasManual, hasCenter, hasLibrary, manualArmed,
             hostDir, markerApp: marker ? marker.app : null,
             closed: speciesEditor.isOpen };
  })()`);
  check('物种工坊能打开并画出蓝图', p7.ink > 500 && p7.cells >= 1,
    `${p7.cells} 细胞 · ${p7.ink} px`);
  check('投放把个体放进了世界', p7.placed === 1, `+${p7.placed}`);
  check('工坊有「手动放置 / 放到视野中心 / 存入生物库」三个按钮',
    p7.hasManual && p7.hasCenter && p7.hasLibrary);
  check('「手动放置」装入放置笔刷并收起工坊', p7.manualArmed === true);
  check('存入生物库生效', p7.libAfter === p7.libBefore + 1,
    `${p7.libBefore} → ${p7.libAfter}`);
  check('参数面板有 30 行细胞属性', p7.attrRows === 30 && p7.attrKinds === 6,
    `${p7.attrKinds} 种 · ${p7.attrRows} 行`);
  if (p7.markerApp === 'evosim') {
    check('桌面版导出去向是程序目录', typeof p7.hostDir === 'string' && p7.hostDir.length > 0, p7.hostDir);
  } else {
    check('浏览器版没有宿主标记，导出回退成下载', p7.hostDir === null && p7.markerApp === null);
  }
  check('物种工坊能关闭', p7.closed === false);

  // ── 6. 控制台 ──
  console.log('\n=== 6. 控制台 ===');
  const errors = client.events.filter((e) =>
    (e.method === 'Log.entryAdded' && e.params.entry.level === 'error')
    || e.method === 'Runtime.exceptionThrown');
  check('无控制台错误', errors.length === 0,
    errors.length ? errors.map((e) => JSON.stringify(e.params).slice(0, 240)).join(' | ') : '干净');

  console.log(`\n${'='.repeat(52)}`);
  if (failures === 0) console.log(`全部通过 ✓  (${checks} 项)`);
  else console.log(`通过 ${checks - failures} 项，失败 ${failures} 项 ✗`);

  client.ws.close();
  chrome.kill();
  process.exit(failures === 0 ? 0 : 1);
}

main().catch((err) => {
  console.error('验证崩溃:', err.message);
  chrome.kill();
  process.exit(1);
});
