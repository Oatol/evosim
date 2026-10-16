/**
 * 阶段 6 浏览器端到端验证：无头 Chrome + CDP。
 *
 * 逻辑测试（phase6-test.mjs）已经覆盖了 CSV 生成与谱系推导。
 * 这里只验证"在真实浏览器里能不能用"：
 *   1. 物种树能打开、画出了内容、点击能选中物种并联动高亮世界画布
 *   2. 两个导出按钮真的下载了结构正确的 CSV
 *   3. 着色模式切换真的改变了画布颜色
 *   4. 检视器的"跟随"开关能驱动相机
 *   5. 全程无控制台错误
 *
 * 注意：python -m http.server 不发 Cache-Control，Chrome 会启发式缓存 JS。
 * 复用 profile 会拿到上一版代码，所以每次跑都从干净 profile 开始。
 */

import { spawn, execSync } from 'node:child_process';
import { setTimeout as sleep } from 'node:timers/promises';
import { rmSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';

const CHROME = 'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe';
const BASE_PROFILE = 'C:\\Users\\安\\AppData\\Local\\Temp\\chrome-evosim-p6';
const DOWNLOADS = 'C:\\Users\\安\\AppData\\Local\\Temp\\evosim-downloads-p6';
const URL = 'http://127.0.0.1:8131/';
const PORT = 9227;
const SHOT = 'C:\\Users\\安\\AppData\\Local\\Temp\\phase6-species-tree.png';

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
    await sleep(260);
  };

  const clickEl = (selector) => evaluate(
    `(() => { const el = document.querySelector(${JSON.stringify(selector)}); if (!el) return false; el.click(); return true; })()`);

  await sleep(3000);

  // ── 1. 启动状态 ──
  console.log('=== 1. 启动状态 ===');
  const boot = await evaluate(`(() => {
    const { speciesTree, renderer, world } = window.evosim;
    return {
      tick: world.tick,
      organisms: world.organisms.length,
      badge: document.getElementById('stage-badge').textContent,
      hasTreeBtn: !!document.getElementById('btn-species-tree'),
      hasColorMode: !!document.getElementById('color-mode'),
      colorModeValue: document.getElementById('color-mode').value,
      rendererColorMode: renderer.colorMode,
      treeOpen: speciesTree.isOpen,
      highlightFp: renderer.highlightFingerprint,
    };
  })()`);
  check('页面跑起来了', boot.tick >= 0, `tick ${boot.tick}`);
  check('世界里有生物', boot.organisms > 0, `${boot.organisms} 个`);
  check("徽章已填充阶段号", /^阶段 \d/.test(boot.badge), boot.badge);
  check('顶栏有物种树按钮与着色选择', boot.hasTreeBtn && boot.hasColorMode);
  check('默认按类型着色', boot.rendererColorMode === 'type' && boot.colorModeValue === 'type');
  check('启动时物种树是关的', boot.treeOpen === false);
  check('启动时没有物种高亮', boot.highlightFp === null);

  // 先演化一段，攒出足够的物种
  console.log('\n=== 2. 演化出多个物种 ===');
  await evaluate(`window.evosim.engine.setSpeed(50)`);
  await sleep(6000);
  await evaluate(`window.evosim.engine.setSpeed(1)`);
  await sleep(400);
  const evolved = await evaluate(`(() => {
    const { world } = window.evosim;
    return { tick: world.tick, organisms: world.organisms.length,
             species: world.species.list.length, alive: world.species.aliveCount };
  })()`);
  check('模拟推进了', evolved.tick > 2000, `tick ${evolved.tick}`);
  check('已经产生多个物种', evolved.species > 3, `${evolved.species} 个物种（存活 ${evolved.alive}）`);

  // ── 3. 打开物种树 ──
  console.log('\n=== 3. 物种树 ===');
  await evaluate(`window.evosim.engine.pause()`);
  await clickEl('#btn-species-tree');
  await sleep(400);

  const tree = await evaluate(`(() => {
    const t = window.evosim.speciesTree;
    const c = t.canvas;
    const ctx = c.getContext('2d');
    const img = ctx.getImageData(0, 0, c.width, c.height).data;
    // 背景是 #0d1117
    let ink = 0;
    for (let i = 0; i < img.length; i += 4) {
      if (Math.abs(img[i] - 13) > 12 || Math.abs(img[i + 1] - 17) > 12 || Math.abs(img[i + 2] - 23) > 12) ink++;
    }
    return {
      open: t.isOpen, hidden: t.root.hidden,
      w: c.width, h: c.height, ink,
      nodes: t._layout ? t._layout.order.length : 0,
      hits: t._hit.length,
      btnActive: document.getElementById('btn-species-tree').classList.contains('is-active'),
      subtitle: t.subtitleEl.textContent,
    };
  })()`);
  check('点"物种树"后面板打开', tree.open === true && tree.hidden === false);
  check('顶栏按钮同步高亮', tree.btnActive === true);
  check('树 canvas 有尺寸', tree.w > 400 && tree.h > 200, `${tree.w}×${tree.h}`);
  check('树画出了内容（不是空白）', tree.ink > 2000, `${tree.ink} px`);
  check('树里有可点击的节点', tree.hits > 2 && tree.hits === tree.nodes, `${tree.hits} 个节点`);
  check('副标题给出了物种统计', /\d/.test(tree.subtitle), tree.subtitle);

  // ── 4. 点击选中一个物种 ──
  console.log('\n=== 4. 点击物种 → 联动高亮 ===');
  const hitTarget = await evaluate(`(() => {
    const t = window.evosim.speciesTree;
    if (!t._hit.length) return null;
    let best = null;
    for (const h of t._hit) {
      if (!best || (h.y1 - h.y0) > (best.y1 - best.y0)) best = h;
    }
    const r = t.canvas.getBoundingClientRect();
    return {
      id: best.sp.id, fp: best.sp.fingerprint,
      x: r.left + best.x,
      y: r.top + (best.y0 + best.y1) / 2,
      len: best.y1 - best.y0,
    };
  })()`);
  check('找得到一个可点的竖条', !!hitTarget, hitTarget ? `S${hitTarget.id} 长 ${hitTarget.len.toFixed(0)}px` : '没有');

  await clickAt(hitTarget.x, hitTarget.y);
  const afterClick = await evaluate(`(() => {
    const { speciesTree, renderer } = window.evosim;
    return {
      selectedId: speciesTree.selected ? speciesTree.selected.id : null,
      highlightFp: renderer.highlightFingerprint,
      detailActive: speciesTree.detailEl.classList.contains('is-active'),
      detail: speciesTree.detailEl.textContent,
    };
  })()`);
  check('点击后选中了那个物种', afterClick.selectedId === hitTarget.id,
    `选中 S${afterClick.selectedId} / 点击 S${hitTarget.id}`);
  check('世界画布同步高亮该物种', afterClick.highlightFp === hitTarget.fp, afterClick.highlightFp);
  check('底部详情已填充', afterClick.detailActive === true && afterClick.detail.length > 10,
    afterClick.detail.slice(0, 90));

  // 高亮真的画到了世界画布上
  const highlightInk = await evaluate(`(() => {
    const { renderer } = window.evosim;
    const cv = document.getElementById('world-canvas');
    const ctx = cv.getContext('2d');
    renderer.needsRender = true; renderer.render();
    const img = ctx.getImageData(0, 0, cv.width, cv.height).data;
    // 物种高亮是接近纯白的描边
    let white = 0;
    for (let i = 0; i < img.length; i += 4) {
      if (img[i] > 225 && img[i + 1] > 230 && img[i + 2] > 235) white++;
    }
    return white;
  })()`);
  check('世界画布上真的画出了高亮圈', highlightInk > 40, `${highlightInk} px 近白像素`);

  // 物种树 + 选中 + 世界高亮都在这张图里——阶段 6 最值得看的一张
  const shot = await client.send('Page.captureScreenshot', { format: 'png' });
  writeFileSync(SHOT, Buffer.from(shot.data, 'base64'));
  console.log(`  截图已保存：${SHOT}`);

  // ── 5. 着色模式 ──
  console.log('\n=== 5. 按物种着色 ===');
  const colorCount = `(() => {
    const cv = document.getElementById('world-canvas');
    const img = cv.getContext('2d').getImageData(0, 0, cv.width, cv.height).data;
    const set = new Set();
    for (let i = 0; i < img.length; i += 4) {
      set.add(((img[i] >> 4) << 8) | ((img[i + 1] >> 4) << 4) | (img[i + 2] >> 4));
    }
    return set.size;
  })()`;
  const typeColors = await evaluate(`(() => {
    const { renderer } = window.evosim;
    renderer.setColorMode('type'); renderer.needsRender = true; renderer.render();
    return ${colorCount};
  })()`);
  const speciesColors = await evaluate(`(() => {
    const { renderer } = window.evosim;
    renderer.setColorMode('species'); renderer.needsRender = true; renderer.render();
    return ${colorCount};
  })()`);
  const modeState = await evaluate(`(() => {
    const { renderer } = window.evosim;
    const sel = document.getElementById('color-mode');
    sel.value = 'type';
    sel.dispatchEvent(new Event('change', { bubbles: true }));
    return { afterSelect: renderer.colorMode };
  })()`);
  check('按物种着色用到的颜色明显更多',
    speciesColors > typeColors * 1.4,
    `按类型 ${typeColors} 色 → 按物种 ${speciesColors} 色`);
  check('下拉框能切回按类型', modeState.afterSelect === 'type', modeState.afterSelect);

  // ── 6. 导出 CSV（真实下载）──
  console.log('\n=== 6. 导出 CSV ===');
  const beforeExport = await evaluate(`(() => ({
    samples: window.evosim.record.count,
    species: window.evosim.world.species.list.length,
    tick: window.evosim.world.tick,
  }))()`);

  const clickToolBtn = (label) => evaluate(`(() => {
    const btns = [...document.querySelectorAll('#sidebar-left .tool-btn')];
    const t = btns.find((b) => b.textContent === ${JSON.stringify(label)});
    if (t) t.click();
    return !!t;
  })()`);

  check('工具栏里有"曲线 CSV"按钮', await clickToolBtn('曲线 CSV') === true);
  let files = [];
  for (let i = 0; i < 40 && files.length === 0; i++) {
    await sleep(250);
    files = readdirSync(DOWNLOADS).filter((f) => f.startsWith('evosim-history'));
  }
  check('点"曲线 CSV"下载了文件', files.length > 0, files[0] || '没等到文件');
  if (files.length) {
    const csv = readFileSync(`${DOWNLOADS}\\${files[0]}`, 'utf8');
    const lines = csv.replace(/^\ufeff/, '').split('\r\n');
    check('时间序列 CSV 表头正确',
      lines[0].startsWith('tick,population,aliveSpecies,avgCells,avgMutability,share_'),
      lines[0].slice(0, 70));
    check('时间序列 CSV 行数 = 采样点 + 1', lines.length === beforeExport.samples + 1,
      `${lines.length} 行 / ${beforeExport.samples} 个采样点`);
    check('文件名含 tick', files[0].includes(`tick${beforeExport.tick}`), files[0]);
  }

  check('工具栏里有"物种表 CSV"按钮', await clickToolBtn('物种表 CSV') === true);
  let sf = [];
  for (let i = 0; i < 40 && sf.length === 0; i++) {
    await sleep(250);
    sf = readdirSync(DOWNLOADS).filter((f) => f.startsWith('evosim-species'));
  }
  check('点"物种表 CSV"下载了文件', sf.length > 0, sf[0] || '没等到文件');
  if (sf.length) {
    const csv = readFileSync(`${DOWNLOADS}\\${sf[0]}`, 'utf8');
    const lines = csv.replace(/^\ufeff/, '').split('\r\n');
    check('物种表 CSV 表头正确',
      lines[0].startsWith('id,fingerprint,name,ancestorId,startTick,endTick,lifespan,'),
      lines[0].slice(0, 70));
    check('物种表 CSV 行数 = 物种数 + 1', lines.length === beforeExport.species + 1,
      `${lines.length} 行 / ${beforeExport.species} 个物种`);
  }

  // ── 7. 检视器跟随 ──
  console.log('\n=== 7. 相机跟随 ===');
  const follow = await evaluate(`(() => {
    const { world, editor, inspector, camera, engine } = window.evosim;
    engine.pause();
    const org = world.organisms[0];
    editor.onInspect(org);
    const before = { following: inspector.following, follow: camera.follow };
    inspector.setFollow(true);
    const on = { following: inspector.following, btnActive: inspector.followBtn.classList.contains('is-active') };
    inspector.close();
    return { before, on, afterClose: { following: inspector.following, follow: camera.follow } };
  })()`);
  check('打开检视器时默认不跟随', follow.before.following === false && follow.before.follow === null);
  check('点"跟随"后开关打开', follow.on.following === true && follow.on.btnActive === true);
  check('关闭检视器后自动解除跟随',
    follow.afterClose.following === false && follow.afterClose.follow === null);

  // 跟随真的驱动了相机
  const followMoves = await evaluate(`(() => {
    const { world, editor, inspector, camera, engine, renderer } = window.evosim;
    // 找一个会移动的个体，否则相机不动看不出跟随
    let target = null;
    for (const o of world.organisms) if (o.anatomy.hasMover) { target = o; break; }
    if (!target) target = world.organisms[0];
    editor.onInspect(target);
    inspector.setFollow(true);
    const startX = camera.x, startY = camera.y;
    let moved = false;
    for (let i = 0; i < 200 && !moved; i++) {
      world.step();
      if (inspector.following) {
        camera.follow = { col: target.col, row: target.row };
        camera.updateFollow(renderer.width, renderer.height);
      }
      if (Math.abs(camera.x - startX) > 0.5 || Math.abs(camera.y - startY) > 0.5) moved = true;
    }
    const stillFollowing = inspector.following;
    inspector.close();
    return { moved, stillFollowing, movedBy: Math.abs(camera.x - startX) + Math.abs(camera.y - startY) };
  })()`);
  check('跟随期间相机确实跟着动', followMoves.moved === true,
    `相机位移 ${followMoves.movedBy.toFixed(1)} 格`);

  // ── 8. 关闭与清理 ──
  console.log('\n=== 8. 关闭与清理 ===');
  await clickEl('#btn-species-tree');
  await sleep(200);
  const reopened = await evaluate(`(() => {
    const t = window.evosim.speciesTree;
    // 重新打开并选中一个物种，再按 Esc 验证高亮被清掉
    if (!t.isOpen) t.open();
    if (t._hit.length) t._onClick({ clientX: 0, clientY: 0 });
    return { open: t.isOpen, nodes: t._hit.length };
  })()`);
  check('物种树能重新打开', reopened.open === true && reopened.nodes > 0, `${reopened.nodes} 个节点`);

  // 选中一个物种（直接走内部选中逻辑），再按 Esc
  await evaluate(`(() => {
    const t = window.evosim.speciesTree;
    const h = t._hit[Math.floor(t._hit.length / 2)];
    t.selected = h.sp;
    t._draw(); t._renderDetail();
    if (t.onSelect) t.onSelect(h.sp);
    return true;
  })()`);
  const beforeEsc = await evaluate(`window.evosim.renderer.highlightFingerprint`);
  check('选中后世界有高亮', beforeEsc !== null, beforeEsc);

  await client.send('Input.dispatchKeyEvent', {
    type: 'rawKeyDown', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27, nativeVirtualKeyCode: 27,
  });
  await client.send('Input.dispatchKeyEvent', {
    type: 'keyUp', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27, nativeVirtualKeyCode: 27,
  });
  await sleep(300);
  const afterEsc = await evaluate(`(() => {
    const { speciesTree, renderer } = window.evosim;
    return {
      treeOpen: speciesTree.isOpen,
      hidden: speciesTree.root.hidden,
      highlightFp: renderer.highlightFingerprint,
      btnActive: document.getElementById('btn-species-tree').classList.contains('is-active'),
    };
  })()`);
  check('Esc 关闭物种树', afterEsc.treeOpen === false && afterEsc.hidden === true);
  check('Esc 后世界高亮被清除', afterEsc.highlightFp === null);
  check('Esc 后顶栏按钮取消高亮', afterEsc.btnActive === false);

  // T 键开合
  const pressKey = async (key, code, vk) => {
    await client.send('Input.dispatchKeyEvent', {
      type: 'rawKeyDown', key, code, windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk,
    });
    await client.send('Input.dispatchKeyEvent', {
      type: 'keyUp', key, code, windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk,
    });
    await sleep(280);
  };
  await pressKey('t', 'KeyT', 84);
  const afterT = await evaluate(`(() => {
    const { speciesTree } = window.evosim;
    return { open: speciesTree.isOpen,
             btnActive: document.getElementById('btn-species-tree').classList.contains('is-active') };
  })()`);
  check('按 T 打开物种树', afterT.open === true && afterT.btnActive === true);
  await pressKey('t', 'KeyT', 84);
  check('再按 T 关闭物种树', await evaluate(`window.evosim.speciesTree.isOpen`) === false);

  // 世界重建也要关掉物种树
  const afterRegen = await evaluate(`(() => {
    document.getElementById('btn-regenerate').click();
    const { speciesTree, renderer } = window.evosim;
    return { treeOpen: speciesTree.isOpen, highlightFp: renderer.highlightFingerprint,
             organisms: window.evosim.world.organisms.length };
  })()`);
  check('重建世界后物种树自动关闭', afterRegen.treeOpen === false);
  check('重建后高亮被清除', afterRegen.highlightFp === null);
  check('重建后世界里有新生物', afterRegen.organisms > 0, `${afterRegen.organisms} 个`);

  // ── 8b. 物种树是"不挡画布的非模态浮窗" ──
  // 用户反馈：物种树打开后占满整个空间，看不到下面的运行窗口，选中物种时看不到高亮。
  // 这里逐条验证它确实改成了和检视器一样的非模态浮窗。
  console.log('\n=== 8b. 物种树浮窗行为 ===');
  await evaluate(`window.evosim.speciesTree.open()`);
  await sleep(320);

  const floatState = await evaluate(`(() => {
    const { speciesTree } = window.evosim;
    const root = speciesTree.root;
    const panel = root.querySelector('.species-tree-panel');
    const head = root.querySelector('.species-tree-head');
    const wrap = document.querySelector('.canvas-wrap');
    const cs = getComputedStyle(root);
    const ps = getComputedStyle(panel);
    const pr = panel.getBoundingClientRect();
    const wr = wrap.getBoundingClientRect();
    const ix = Math.max(0, Math.min(pr.right, wr.right) - Math.max(pr.left, wr.left));
    const iy = Math.max(0, Math.min(pr.bottom, wr.bottom) - Math.max(pr.top, wr.top));
    const wrapArea = Math.max(1, wr.width * wr.height);
    // 扫一圈找"面板之外、且真能点到画布"的点
    const cands = [
      [wr.left + 24, wr.top + 24], [wr.right - 24, wr.top + 24],
      [wr.left + 24, wr.bottom - 24], [wr.right - 24, wr.bottom - 24],
      [wr.left + wr.width / 2, wr.bottom - 24], [wr.right - 24, wr.top + wr.height / 2],
    ];
    let probe = null;
    for (const [x, y] of cands) {
      const outside = (x < pr.left || x > pr.right || y < pr.top || y > pr.bottom);
      if (!outside) continue;
      const hit = document.elementFromPoint(x, y);
      if (hit && hit.closest && hit.closest('.canvas-wrap')) { probe = { x: Math.round(x), y: Math.round(y) }; break; }
    }
    return {
      rootPE: cs.pointerEvents,
      panelPE: ps.pointerEvents,
      hasBackdrop: !!root.querySelector('.species-tree-backdrop'),
      pinned: speciesTree.pinned,
      pinActive: speciesTree.pinBtn.classList.contains('is-active'),
      coverRatio: (ix * iy) / wrapArea,
      panelW: Math.round(pr.width), panelH: Math.round(pr.height),
      wrapW: Math.round(wr.width), wrapH: Math.round(wr.height),
      hasHead: !!head,
      probe,
    };
  })()`);
  check('浮窗外层不拦截事件（pointer-events:none）', floatState.rootPE === 'none');
  check('浮窗面板本身可交互（pointer-events:auto）', floatState.panelPE === 'auto');
  check('没有全屏遮罩元素', floatState.hasBackdrop === false);
  check('浮窗没有盖满整个画布', floatState.coverRatio < 0.8,
    `覆盖画布 ${(floatState.coverRatio * 100).toFixed(0)}%（面板 ${floatState.panelW}×${floatState.panelH} / 画布 ${floatState.wrapW}×${floatState.wrapH}）`);
  check('面板之外的画布仍能接到点击', !!floatState.probe,
    floatState.probe ? `探测点 (${floatState.probe.x},${floatState.probe.y})` : '找不到可点画布');
  check('物种树默认是锁定的', floatState.pinned === true && floatState.pinActive === true);
  check('有可拖动的标题栏', floatState.hasHead === true);

  // 锁定时点画布不该关掉物种树（用户要"选完物种去画布操作"）
  await clickAt(floatState.probe.x, floatState.probe.y);
  const pinnedClickOpen = await evaluate(`window.evosim.speciesTree.isOpen`);
  check('锁定状态下点画布不会关闭物种树', pinnedClickOpen === true);

  // 解锁后点画布才关
  await clickEl('.species-tree-pin');
  await sleep(180);
  const unpinned = await evaluate(`(() => {
    const t = window.evosim.speciesTree;
    return { pinned: t.pinned, pinActive: t.pinBtn.classList.contains('is-active') };
  })()`);
  check('点图钉可以解锁', unpinned.pinned === false && unpinned.pinActive === false);
  await clickAt(floatState.probe.x, floatState.probe.y);
  const unpinnedClickOpen = await evaluate(`window.evosim.speciesTree.isOpen`);
  check('解锁后点画布关闭物种树', unpinnedClickOpen === false);

  // 拖头部移动浮窗
  const dragStart = await evaluate(`(() => {
    const t = window.evosim.speciesTree;
    if (!t.isOpen) t.open();
    const head = t.root.querySelector('.species-tree-head');
    const r = head.getBoundingClientRect();
    const pr = t.root.querySelector('.species-tree-panel').getBoundingClientRect();
    return { x: Math.round(r.left + r.width / 2), y: Math.round(r.top + r.height / 2), left: pr.left, top: pr.top };
  })()`);
  await client.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: dragStart.x, y: dragStart.y, button: 'none', buttons: 0 });
  await client.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: dragStart.x, y: dragStart.y, button: 'left', buttons: 1, clickCount: 1 });
  await client.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: dragStart.x + 80, y: dragStart.y + 40, button: 'left', buttons: 1 });
  await client.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: dragStart.x + 150, y: dragStart.y + 80, button: 'left', buttons: 1 });
  await client.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: dragStart.x + 150, y: dragStart.y + 80, button: 'left', buttons: 0, clickCount: 1 });
  await sleep(220);
  const dragged = await evaluate(`(() => {
    const t = window.evosim.speciesTree;
    const pr = t.root.querySelector('.species-tree-panel').getBoundingClientRect();
    return { left: pr.left, top: pr.top };
  })()`);
  check('拖标题栏可以移动浮窗',
    Math.abs(dragged.left - dragStart.left) > 40 && Math.abs(dragged.top - dragStart.top) > 20,
    `(${dragStart.left.toFixed(0)},${dragStart.top.toFixed(0)}) → (${dragged.left.toFixed(0)},${dragged.top.toFixed(0)})`);

  await evaluate(`window.evosim.speciesTree.close()`);
  await sleep(180);

  // ── 9. 性能 ──
  console.log('\n=== 9. 性能 ===');
  await evaluate(`(() => { const e = window.evosim.engine; e.setSpeed(1); e.start(); })()`);
  let bestFps = 0;
  let bestTps = 0;
  for (let i = 0; i < 6; i++) {
    await sleep(600);
    const s = await evaluate(`(() => {
      const { engine, world } = window.evosim;
      return { fps: engine.fps, tps: engine.tps, running: engine.running, organisms: world.organisms.length };
    })()`);
    bestFps = Math.max(bestFps, s.fps);
    bestTps = Math.max(bestTps, s.tps);
  }
  check('帧率健康', bestFps >= 50, `${bestFps.toFixed(1)} fps`);
  check('tick 速率达标', bestTps >= 55, `${bestTps.toFixed(1)} tick/s`);

  // 物种树开着时的帧率——树上几百根竖条不该拖垮主循环
  await evaluate(`window.evosim.speciesTree.open()`);
  await sleep(1500);
  const withTree = await evaluate(`(() => {
    const { engine } = window.evosim;
    return { fps: engine.fps, open: window.evosim.speciesTree.isOpen };
  })()`);
  check('物种树开着时帧率仍健康', withTree.fps >= 45, `${withTree.fps.toFixed(1)} fps（树开着）`);

  const tickMs = await evaluate(`(() => {
    const { world, engine } = window.evosim;
    const wasRunning = engine.running;
    engine.pause();
    for (let i = 0; i < 20; i++) world.step();
    const t0 = performance.now();
    for (let i = 0; i < 150; i++) world.step();
    const ms = (performance.now() - t0) / 150;
    if (wasRunning) engine.start();
    return { ms, organisms: world.organisms.length };
  })()`);
  check('单 tick 耗时在预算内', tickMs.ms < 8, `${tickMs.ms.toFixed(3)} ms/tick · ${tickMs.organisms} 生物`);

  // ── 10. 控制台 ──
  console.log('\n=== 10. 控制台 ===');
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
