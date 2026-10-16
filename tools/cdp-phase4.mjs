/**
 * 阶段 4 浏览器端到端验证：无头 Chrome + CDP。
 *
 * 逻辑测试（phase4-test.mjs）已经覆盖了神经网络本身的数学。
 * 这里只验证"在真实浏览器里，人能不能看见、能不能点开"：
 *   1. 点击画布上的生物能打开检视器
 *   2. 身体蓝图 canvas 与网络 canvas 都真的画出了内容（不是空白）
 *   3. 网络图里正权重（蓝）与负权重（红）都有，argmax 蓝框画在正确的位置
 *   4. Esc / 点背景 / 重置世界都能关掉，高亮同步清除
 *   5. 全程无控制台错误
 *
 * 注意：python -m http.server 不发 Cache-Control，Chrome 会启发式缓存 JS。
 * 复用 profile 会拿到上一版代码，所以每次跑都从干净 profile 开始。
 */

import { spawn, execSync } from 'node:child_process';
import { setTimeout as sleep } from 'node:timers/promises';
import { writeFileSync, rmSync, mkdirSync } from 'node:fs';

const CHROME = 'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe';
const BASE_PROFILE = 'C:\\Users\\安\\AppData\\Local\\Temp\\chrome-evosim-p4';
const URL = 'http://127.0.0.1:8131/';
const PORT = 9225;
const SHOT = 'C:\\Users\\安\\AppData\\Local\\Temp\\phase4-inspector.png';

/** 同步小睡：删 profile 失败时用来等上一轮 Chrome 退干净 */
function sleepSync(ms) {
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);
}

// 每次跑都从干净 profile 开始，否则 Chrome 的启发式 JS 缓存会让我们拿到旧代码。
// 直接 rmSync 常撞上"上一轮 Chrome 还没退干净"（EBUSY），所以先重试，
// 实在删不掉就换一个带 pid 的新目录。
function resetProfile(base) {
  for (let i = 0; i < 8; i++) {
    try { rmSync(base, { recursive: true, force: true }); return base; }
    catch { sleepSync(400); }
  }
  const alt = `${base}-${process.pid}`;
  try { rmSync(alt, { recursive: true, force: true }); } catch { /* 忽略 */ }
  return alt;
}

const PROFILE = resetProfile(BASE_PROFILE);
mkdirSync(PROFILE, { recursive: true });

/**
 * 强杀进程树。`chrome.kill()` 只发 SIGTERM，Windows 上 Chrome 常常忽略它，
 * 留下子进程继续占着调试端口。
 */
function forceKill(pid) {
  if (!pid) return;
  try { execSync(`taskkill /F /T /PID ${pid}`, { stdio: 'ignore' }); } catch { /* 已经退了 */ }
}

/**
 * 释放调试端口：杀掉还在监听它的残留进程。
 *
 * 这一步是**必需的**：如果上一轮 Chrome 没退干净，新 Chrome 绑不上这个端口，
 * 而 `fetch('/json/list')` 会连到**旧实例**的页面上——于是测试在别人的页面里跑，
 * 结果全是假的。
 */
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
  '--window-size=1440,900',
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

/**
 * 读检视器两个 canvas 的像素。
 *   身体蓝图：背景是深色 #0b1020，所以"不是背景色"的就是细胞
 *   网络图：背景透明，所以 alpha > 4 的就是墨迹；再按色相分出蓝/红
 */
const INSPECTOR_PROBE = `(() => {
  const ins = window.evosim.inspector;

  const readBody = () => {
    const c = ins.bodyCanvas;
    const img = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
    let ink = 0;
    for (let i = 0; i < img.length; i += 4) {
      if (Math.abs(img[i] - 11) > 10 || Math.abs(img[i + 1] - 16) > 10 || Math.abs(img[i + 2] - 32) > 10) ink++;
    }
    return { w: c.width, h: c.height, ink };
  };

  const readNet = () => {
    const c = ins.netCanvas;
    const img = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
    let ink = 0, blue = 0, red = 0;
    for (let i = 0; i < img.length; i += 4) {
      const r = img[i], g = img[i + 1], b = img[i + 2], a = img[i + 3];
      if (a <= 4) continue;
      ink++;
      if (b > r + 12 && b > g + 8) blue++;
      else if (r > b + 12 && r > g + 8) red++;
    }
    return { w: c.width, h: c.height, ink, blue, red };
  };

  // 采样 argmax 输出节点外围的蓝框
  let ring = null;
  const org = ins.organism;
  const L = ins._layout;
  if (org && L) {
    const out = org.brain.outputActivations;
    let best = 0;
    for (let k = 1; k < L.OUT; k++) if (out[k] > out[best]) best = k;
    const dpr = window.devicePixelRatio || 1;
    const cx = Math.round((L.outX + 12.5) * dpr);
    const cy = Math.round(L.outY[best] * dpr);
    const data = ins.netCtx.getImageData(cx - 3, cy - 3, 7, 7).data;
    let bestBlue = -999;
    for (let i = 0; i < data.length; i += 4) {
      if (data[i + 3] <= 4) continue;
      const v = data[i + 2] - data[i];
      if (v > bestBlue) bestBlue = v;
    }
    ring = { best, bestBlue, label: ['前', '右', '后', '左', '转身'][best] };
  }

  const attrs = [...ins.attrList.querySelectorAll('dd')].map((e) => e.textContent);

  return {
    open: ins.isOpen,
    hidden: ins.root.hidden,
    title: ins.titleEl.textContent,
    body: readBody(),
    net: readNet(),
    ring,
    attrs,
  };
})()`;

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
    if (r.exceptionDetails) {
      throw new Error('页面内异常: ' + JSON.stringify(r.exceptionDetails.exception));
    }
    return r.result.value;
  };

  const clickAt = async (x, y) => {
    await client.send('Input.dispatchMouseEvent', {
      type: 'mouseMoved', x, y, button: 'none', buttons: 0,
    });
    await client.send('Input.dispatchMouseEvent', {
      type: 'mousePressed', x, y, button: 'left', buttons: 1, clickCount: 1,
    });
    await client.send('Input.dispatchMouseEvent', {
      type: 'mouseReleased', x, y, button: 'left', buttons: 0, clickCount: 1,
    });
    await sleep(250);
  };

  await sleep(3000);

  // ── 1. 启动状态：大脑接线 ──
  console.log('=== 1. 启动状态 ===');
  const boot = await evaluate(`(() => {
    const { world, Config } = window.evosim;
    const orgs = world.organisms;
    const sample = orgs.slice(0, 40);
    const brains = new Set(sample.map((o) => o.brain));
    return {
      tick: world.tick,
      organisms: orgs.length,
      badge: document.getElementById('stage-badge').textContent,
      brainCount: brains.size,
      weightCount: orgs[0] ? orgs[0].brain.weightCount : 0,
      inSize: orgs[0] ? orgs[0].brain.inSize : 0,
      hiddenSize: orgs[0] ? orgs[0].brain.hiddenSize : 0,
      outSize: orgs[0] ? orgs[0].brain.outSize : 0,
      senseDim: orgs[0] ? orgs[0].sensors.size : 0,
      allHaveBrain: sample.every((o) => o.brain && o.sensors),
      cfgTurnCooldown: Config.life.turnCooldown,
      hasMoveRange: 'initialMoveRange' in Config.life,
    };
  })()`);

  check('页面跑起来了', boot.tick >= 0, `tick ${boot.tick}`);
  check('世界里有生物', boot.organisms > 0, `${boot.organisms} 个`);
  check('徽章已填充阶段号', /^阶段 \d/.test(boot.badge), boot.badge);
  check('每个个体都有大脑与感知器', boot.allHaveBrain);
  check('大脑是每个个体独立实例', boot.brainCount === Math.min(40, boot.organisms),
    `${boot.brainCount} 个独立大脑`);
  check('网络拓扑 40 → 12 → 5', boot.inSize === 40 && boot.hiddenSize === 12 && boot.outSize === 5,
    `${boot.inSize} → ${boot.hiddenSize} → ${boot.outSize}`);
  check('权重总数 557', boot.weightCount === 557, `${boot.weightCount}`);
  check('感知维度 40', boot.senseDim === 40, `${boot.senseDim}`);
  check('配置里转身冷却已生效', boot.cfgTurnCooldown === 4, `${boot.cfgTurnCooldown}`);
  check('旧的 moveRange 配置已移除', boot.hasMoveRange === false);

  // ── 2. 演化一段，确认神经系统真的在驱动行为 ──
  console.log('\n=== 2. 演化中（×50 倍速跑 5 秒）===');
  await evaluate(`window.evosim.engine.setSpeed(50)`);
  await sleep(5000);
  await evaluate(`window.evosim.engine.setSpeed(1)`);
  await sleep(600);

  const evolved = await evaluate(`(() => {
    const { world } = window.evosim;
    let movers = 0, eyes = 0, killers = 0, badInput = 0, badHidden = 0;
    let biasOk = 0, newborns = 0;
    for (const o of world.organisms) {
      if (o.anatomy.hasMover) movers++;
      if (o.anatomy.hasEye) eyes++;
      if (o.anatomy.hasKiller) killers++;
      const inp = o.brain.lastInput;
      // 刚在上一个 tick 的阶段 B 出生的个体还没轮到 think()，lastInput 仍是 null
      if (!inp) newborns++;
      else if (inp.length !== 40) badInput++;
      else if (Math.abs(inp[39] - 1) < 1e-6) biasOk++;
      const h = o.brain.hiddenActivations;
      for (let j = 0; j < h.length; j++) if (!(h[j] >= -1.001 && h[j] <= 1.001)) { badHidden++; break; }
    }
    return {
      tick: world.tick,
      organisms: world.organisms.length,
      movers, eyes, killers, badInput, badHidden, biasOk, newborns,
      avgCells: world.organismStats().avgCells,
    };
  })()`);

  check('模拟确实推进了', evolved.tick > 500, `tick ${evolved.tick}`);
  check('种群没有崩', evolved.organisms > 0, `${evolved.organisms} 个`);
  check('感知向量长度都是 40', evolved.badInput === 0, `${evolved.badInput} 个异常`);
  check('已感知过的个体偏置输入恒为 1', evolved.biasOk === evolved.organisms - evolved.newborns,
    `${evolved.biasOk} / ${evolved.organisms}（${evolved.newborns} 个新生个体尚未 think）`);
  check('隐藏层激活值都在 tanh 值域内', evolved.badHidden === 0, `${evolved.badHidden} 个越界`);
  check('演化出了会移动的个体', evolved.movers > 0, `${evolved.movers} 个会移动`);
  console.log(`  ${evolved.organisms} 生物 · 平均体型 ${evolved.avgCells.toFixed(2)} 格 · ` +
    `会移动 ${evolved.movers} · 有眼睛 ${evolved.eyes} · 有杀手 ${evolved.killers}`);

  // ── 3. 检视工具：真实鼠标点击 ──
  console.log('\n=== 3. 点击生物打开检视器 ===');
  await evaluate(`window.evosim.engine.pause()`);
  await sleep(300);

  const toolState = await evaluate(`(() => {
    const { editor } = window.evosim;
    const btns = [...document.querySelectorAll('#sidebar-left .tool-btn')];
    const target = btns.find((b) => b.textContent === '检视');
    if (target) target.click();
    return { labels: btns.map((b) => b.textContent), tool: editor.tool };
  })()`);
  check('工具栏里有"检视"按钮', toolState.labels.includes('检视'), toolState.labels.join(' / '));
  check('点击后编辑器切到检视模式', toolState.tool === 'inspect', toolState.tool);

  // 找一个被生物占据的格子，算出它在屏幕上的位置
  const targetCell = await evaluate(`(() => {
    const { world, camera } = window.evosim;
    const canvas = document.getElementById('world-canvas');
    const rect = canvas.getBoundingClientRect();
    const grid = world.grid;
    for (let i = 0; i < grid.owners.length; i++) {
      const id = grid.owners[i];
      if (id < 0) continue;
      const o = world.organismById(id);
      if (!o || !o.alive) continue;
      const col = i % grid.cols;
      const row = (i / grid.cols) | 0;
      return {
        x: rect.left + camera.worldToScreenX(col) + camera.zoom / 2,
        y: rect.top + camera.worldToScreenY(row) + camera.zoom / 2,
        col, row, id: o.id, zoom: camera.zoom,
      };
    }
    return null;
  })()`);
  check('能在网格里找到一个活着的个体', !!targetCell,
    targetCell ? `id=${targetCell.id} @ (${targetCell.col}, ${targetCell.row})` : '没找到');

  await clickAt(targetCell.x, targetCell.y);

  const opened = await evaluate(INSPECTOR_PROBE);
  const worldState = await evaluate(`(() => {
    const { inspector, renderer, editor } = window.evosim;
    return {
      isOpen: inspector.isOpen,
      orgId: inspector.organism ? inspector.organism.id : null,
      highlightId: renderer.highlight ? renderer.highlight.id : null,
      tool: editor.tool,
    };
  })()`);

  check('点击生物后检视器打开了', opened.open === true);
  check('检视器浮层可见', opened.hidden === false);
  check('打开的是被点中的那个个体', worldState.orgId === targetCell.id,
    `检视 id=${worldState.orgId} / 点击 id=${targetCell.id}`);
  check('世界画布同步高亮', worldState.highlightId === targetCell.id,
    `highlight id=${worldState.highlightId}`);

  // ── 4. 检视器内容真的画出来了 ──
  console.log('\n=== 4. 检视器渲染 ===');
  check('身体蓝图 canvas 有尺寸', opened.body.w > 0 && opened.body.h > 0,
    `${opened.body.w}×${opened.body.h}`);
  check('身体蓝图画出了细胞', opened.body.ink > 200, `${opened.body.ink} px`);
  check('网络图 canvas 有尺寸', opened.net.w > 0 && opened.net.h > 0,
    `${opened.net.w}×${opened.net.h}`);
  check('网络图不是空白（连线 + 节点）', opened.net.ink > 500, `${opened.net.ink} px`);
  check('网络图里有正权重（蓝）', opened.net.blue > 100, `${opened.net.blue} px`);
  check('网络图里有负权重（红）', opened.net.red > 100, `${opened.net.red} px`);
  check('输出层 argmax 画了蓝框', opened.ring && opened.ring.bestBlue > 40,
    opened.ring ? `argmax=${opened.ring.label}(${opened.ring.best}) 蓝度 ${opened.ring.bestBlue}` : '取不到布局');
  check('属性面板填满了 10 项', opened.attrs.length === 10 && opened.attrs.every((t) => t && t !== '—'),
    opened.attrs.join(' | ').slice(0, 160));

  // 检视器开着的时候截图——这是这个阶段最值得看的一张图
  const shot = await client.send('Page.captureScreenshot', { format: 'png' });
  writeFileSync(SHOT, Buffer.from(shot.data, 'base64'));
  console.log(`  截图已保存：${SHOT}`);

  // ── 5. 激活值随 tick 更新 ──
  console.log('\n=== 5. 实时激活值 ===');
  const before = await evaluate(`(() => {
    const o = window.evosim.inspector.organism;
    return { id: o.id, input: Array.from(o.brain.lastInput), out: Array.from(o.brain.outputActivations) };
  })()`);

  const stepped = await evaluate(`(() => {
    const { world, inspector } = window.evosim;
    for (let i = 0; i < 6; i++) world.step();
    inspector.refresh();
    const o = inspector.organism;
    return {
      alive: o.alive,
      input: Array.from(o.brain.lastInput),
      out: Array.from(o.brain.outputActivations),
      finite: o.brain.outputActivations.every((v) => Number.isFinite(v)),
    };
  })()`);

  const inputChanged = before.input.some((v, i) => v !== stepped.input[i]);
  check('走 6 tick 后感知输入发生变化', stepped.alive ? inputChanged : true,
    stepped.alive ? `年龄分量 ${before.input[37].toFixed(4)} → ${stepped.input[37].toFixed(4)}` : '个体已死亡，跳过');
  check('输出激活值都是有限数', stepped.finite);

  const afterStep = await evaluate(INSPECTOR_PROBE);
  check('刷新后网络图仍然正常绘制', afterStep.net.ink > 500, `${afterStep.net.ink} px`);

  // ── 6. 浮窗行为：关闭 / 图钉 / 拖动 ──
  console.log('\n=== 6. 浮窗行为（关闭 / 图钉 / 拖动）===');
  const panelBox = await evaluate(`(() => {
    const p = document.querySelector('.inspector-panel');
    const r = p.getBoundingClientRect();
    return { w: r.width, h: r.height, vw: window.innerWidth, vh: window.innerHeight,
             backdrop: !!document.querySelector('.inspector-backdrop') };
  })()`);
  check('检视器是不挡画布的浮窗（不再全屏）',
    panelBox.w < panelBox.vw * 0.9 && panelBox.h < panelBox.vh * 0.9,
    `${Math.round(panelBox.w)}×${Math.round(panelBox.h)} / 视口 ${panelBox.vw}×${panelBox.vh}`);
  check('已移除全屏遮罩', panelBox.backdrop === false);

  const emptyCell = await evaluate(`(() => {
    const { world, camera } = window.evosim;
    const canvas = document.getElementById('world-canvas');
    const rect = canvas.getBoundingClientRect();
    const grid = world.grid;
    const p = document.querySelector('.inspector-panel').getBoundingClientRect();
    const inPanel = (x, y) => x >= p.left - 4 && x <= p.right + 4 && y >= p.top - 4 && y <= p.bottom + 4;
    for (let i = 0; i < grid.owners.length; i++) {
      if (grid.owners[i] !== -1) continue;
      const col = i % grid.cols;
      const row = (i / grid.cols) | 0;
      const sx = camera.worldToScreenX(col) + camera.zoom / 2;
      const sy = camera.worldToScreenY(row) + camera.zoom / 2;
      if (sx < 4 || sy < 4 || sx > rect.width - 4 || sy > rect.height - 4) continue;
      const cx = rect.left + sx, cy = rect.top + sy;
      if (inPanel(cx, cy)) continue;
      return { x: cx, y: cy, col, row };
    }
    return null;
  })()`);
  check('能在浮窗外找到一个画布空白点', !!emptyCell,
    emptyCell ? `(${emptyCell.col}, ${emptyCell.row})` : '没找到');

  // 未锁定时点画布 → 关闭
  await clickAt(emptyCell.x, emptyCell.y);
  const afterEmpty = await evaluate(`(() => {
    const { inspector, renderer } = window.evosim;
    return { isOpen: inspector.isOpen, highlightId: renderer.highlight ? renderer.highlight.id : null };
  })()`);
  check('未锁定时点画布会关闭检视器', afterEmpty.isOpen === false);
  check('关闭后世界高亮被清除', afterEmpty.highlightId === null);

  // 重新打开 + 图钉锁定
  const reopenWin = await evaluate(`(() => {
    const { world, editor, inspector } = window.evosim;
    const o = world.organisms[0];
    editor.onInspect(o);
    return { isOpen: inspector.isOpen, id: o.id };
  })()`);
  check('可以重新打开检视器', reopenWin.isOpen === true, `id=${reopenWin.id}`);

  const pinned = await evaluate(`(() => {
    const { inspector } = window.evosim;
    inspector.setPinned(true);
    return { pinned: inspector.pinned, active: inspector.pinBtn.classList.contains('is-active') };
  })()`);
  check('图钉按钮能锁定', pinned.pinned === true && pinned.active === true);

  await clickAt(emptyCell.x, emptyCell.y);
  const afterPinned = await evaluate(`window.evosim.inspector.isOpen`);
  check('锁定时点画布不会关闭检视器', afterPinned === true);

  // 拖动头部移动浮窗
  const drag = await evaluate(`(() => {
    const p = document.querySelector('.inspector-panel');
    const h = document.querySelector('.inspector-head');
    const before = p.getBoundingClientRect();
    const opt = (x, y) => ({ bubbles: true, cancelable: true, clientX: x, clientY: y,
                             button: 0, pointerId: 77, isPrimary: true });
    h.dispatchEvent(new PointerEvent('pointerdown', opt(before.left + 60, before.top + 12)));
    h.dispatchEvent(new PointerEvent('pointermove', opt(before.left - 60, before.top + 102)));
    h.dispatchEvent(new PointerEvent('pointerup', opt(before.left - 60, before.top + 102)));
    const after = p.getBoundingClientRect();
    return { dx: Math.round(after.left - before.left), dy: Math.round(after.top - before.top) };
  })()`);
  check('拖动头部能移动浮窗', Math.abs(drag.dx) > 40 && Math.abs(drag.dy) > 40,
    `位移 ${drag.dx}, ${drag.dy}`);

  // 解锁后点画布又关闭
  await evaluate(`window.evosim.inspector.setPinned(false)`);
  await clickAt(emptyCell.x, emptyCell.y);
  const afterUnpin = await evaluate(`window.evosim.inspector.isOpen`);
  check('解锁后点画布会再次关闭', afterUnpin === false);

  // ── 7. Esc 关闭 ──
  console.log('\n=== 7. Esc 关闭 ===');
  await evaluate(`(() => {
    const { world, editor } = window.evosim;
    editor.onInspect(world.organisms[0]);
  })()`);
  await sleep(120);
  await client.send('Input.dispatchKeyEvent', {
    type: 'rawKeyDown', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27, nativeVirtualKeyCode: 27,
  });
  await client.send('Input.dispatchKeyEvent', {
    type: 'keyUp', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27, nativeVirtualKeyCode: 27,
  });
  await sleep(300);

  const afterEsc = await evaluate(`(() => {
    const { inspector, renderer } = window.evosim;
    // 不能直接把 organism 对象序列化回来——它带着 world/grid 的循环引用
    return { isOpen: inspector.isOpen, hidden: inspector.root.hidden,
             highlightId: renderer.highlight ? renderer.highlight.id : null };
  })()`);
  check('Esc 后检视器关闭', afterEsc.isOpen === false);
  check('Esc 后浮层隐藏', afterEsc.hidden === true);
  check('Esc 后世界高亮被清除', afterEsc.highlightId === null);

  // ── 8. 重建世界时自动关闭检视器 ──
  console.log('\n=== 8. 重置世界 ===');
  const reopen = await evaluate(`(() => {
    const { world, editor, engine } = window.evosim;
    engine.pause();
    const o = world.organisms[0];
    editor.onInspect(o);
    return { id: o.id, isOpen: window.evosim.inspector.isOpen };
  })()`);
  check('重新打开检视器（直接走回调）', reopen.isOpen === true, `id=${reopen.id}`);

  const afterRegen = await evaluate(`(() => {
    const { engine, inspector, renderer } = window.evosim;
    document.getElementById('btn-regenerate').click();
    return { isOpen: inspector.isOpen, hidden: inspector.root.hidden,
             highlightId: renderer.highlight ? renderer.highlight.id : null,
             running: engine.running,
             organisms: window.evosim.world.organisms.length };
  })()`);
  check('重建世界后检视器自动关闭', afterRegen.isOpen === false);
  check('重建后浮层隐藏', afterRegen.hidden === true);
  check('重建后高亮被清除', afterRegen.highlightId === null);
  check('重建后世界里有新生物', afterRegen.organisms > 0, `${afterRegen.organisms} 个`);

  // ── 9. 缩小时不重复平铺世界 ──
  console.log('\n=== 9. 缩小时不重复平铺世界 ===');
  // 判据：把画面横向平移"一个世界宽度"再比对——如果世界被平铺了，
  // 平移后应当几乎完全重合（重合率高）；只画一块时平移后落在背景上（重合率≈0）。
  // 这个判据不依赖世界内容怎么分布，只要世界里有东西就成立。
  const tileCheck = await evaluate(`(() => {
    const { camera, renderer, world } = window.evosim;
    const cv = document.getElementById('world-canvas');
    const ctx = cv.getContext('2d');
    const bg = renderer.colorScheme.background;
    const bgR = parseInt(bg.slice(1,3),16), bgG = parseInt(bg.slice(3,5),16), bgB = parseInt(bg.slice(5,7),16);

    const overlapRatio = () => {
      const W = cv.width, H = cv.height;
      const img = ctx.getImageData(0, 0, W, H).data;
      const isBg = (x, y) => {
        const i = (y*W+x)*4;
        return Math.abs(img[i]-bgR)<10 && Math.abs(img[i+1]-bgG)<10 && Math.abs(img[i+2]-bgB)<10;
      };
      const lag = Math.round(world.grid.cols * camera.zoom);
      if (lag <= 0 || lag >= W) return null;
      let content = 0, both = 0;
      for (let y = 0; y < H; y += 2) {
        for (let x = 0; x < W - lag; x += 2) {
          if (isBg(x, y)) continue;
          content++;
          if (!isBg(x + lag, y)) both++;
        }
      }
      return { lag, content, ratio: content ? both / content : 0 };
    };

    const saved = { z: camera.zoom, x: camera.x, y: camera.y };

    camera.setZoom(1.2, renderer.width, renderer.height);
    renderer.needsRender = true; renderer.render();
    const small = overlapRatio();

    camera.zoom = saved.z; camera.x = saved.x; camera.y = saved.y;
    renderer.requestFullRedraw(); renderer.render();

    return { small, W: cv.width, smallTileW: world.grid.cols * 1.2, wrap: world.grid.wrap };
  })()`);
  check('环绕世界在缩小后只画一块（不平铺）',
    tileCheck.wrap && tileCheck.small && tileCheck.small.content > 200 && tileCheck.small.ratio < 0.1,
    tileCheck.small ? `世界宽 ${tileCheck.smallTileW.toFixed(0)}px < 视口 ${tileCheck.W}px · 平移一个世界宽后重合率 ${(tileCheck.small.ratio * 100).toFixed(1)}%`
      : '取不到样本');

  // 结构契约：直接拿桩 ctx 调 _drawTiled 数"画了几块"。
  // （像素判据在这里用不上：EMPTY 与 background 同色，放大时无法靠背景色判断覆盖）
  const tiling = await evaluate(`(() => {
    const { renderer } = window.evosim;
    const probe = (tileW, tileH) => {
      const draws = [];
      renderer._drawTiled(
        { drawImage: (img, x, y, w, h) => draws.push([x, y, w, h]) },
        0, 0, tileW, tileH, 960, 645,
      );
      return draws;
    };
    return { small: probe(300, 225), big: probe(2400, 1800), justUnder: probe(959, 644) };
  })()`);
  check('世界小于视口 → 只画一块',
    tiling.small.length === 1, `${tiling.small.length} 块`);
  check('世界大于视口 → 平铺覆盖视口（1–4 块）',
    tiling.big.length >= 1 && tiling.big.length <= 4, `${tiling.big.length} 块`);
  check('世界略小于视口也仍只画一块',
    tiling.justUnder.length === 1, `${tiling.justUnder.length} 块`);

  // ── 10. 性能 ──
  console.log('\n=== 10. 性能 ===');
  // 注意：setSpeed 不会启动引擎，而 fps/tps 在暂停时是冻结的旧读数。
  // 第 8 段重建世界时引擎正处于暂停态，所以这里必须显式 start()。
  await evaluate(`(() => { const e = window.evosim.engine; e.setSpeed(1); e.start(); })()`);
  await sleep(3000);
  const perf = await evaluate(`(() => {
    const { engine, world } = window.evosim;
    return { fps: engine.fps, tps: engine.tps, tick: world.tick,
             organisms: world.organisms.length, running: engine.running };
  })()`);

  // 真实 RAF 频率：用来区分"引擎慢"和"无头浏览器合成器本来就慢"
  const rafRate = await evaluate(`new Promise((resolve) => {
    let n = 0;
    const t0 = performance.now();
    const tick = () => {
      n++;
      if (performance.now() - t0 < 2000) requestAnimationFrame(tick);
      else resolve(n / ((performance.now() - t0) / 1000));
    };
    requestAnimationFrame(tick);
  })`);

  // 一帧的真实成本 = 推进一个 tick + 重绘（模拟运行时的实际开销）
  const frameCost = await evaluate(`(() => {
    const { renderer, camera, world } = window.evosim;
    for (let i = 0; i < 5; i++) { world.step(); renderer.render(); }
    const t0 = performance.now();
    for (let i = 0; i < 60; i++) { world.step(); renderer.render(); }
    const ms = (performance.now() - t0) / 60;
    return { ms, w: renderer.width, h: renderer.height, zoom: camera.zoom,
             cols: world.grid.cols, rows: world.grid.rows, organisms: world.organisms.length };
  })()`);

  check('引擎确实在跑', perf.running === true, `running=${perf.running}`);
  check('正常速度下帧率健康', perf.fps >= 50,
    `${perf.fps.toFixed(1)} fps（真实 RAF ${rafRate.toFixed(1)} Hz）`);
  check('tick 速率达标', perf.tps >= 55, `${perf.tps.toFixed(1)} tick/s`);
  check('单帧成本（tick + 重绘）在 16.7 ms 预算内', frameCost.ms < 16.7,
    `${frameCost.ms.toFixed(3)} ms/帧 · ${frameCost.organisms} 生物 · ` +
    `${frameCost.w}×${frameCost.h} 视口 · zoom ${frameCost.zoom.toFixed(2)} · ` +
    `网格 ${frameCost.cols}×${frameCost.rows}`);

  const tickMs = await evaluate(`(() => {
    const { world, engine } = window.evosim;
    const wasRunning = engine.running;
    engine.pause();
    for (let i = 0; i < 20; i++) world.step();       // 预热，跳过 JIT
    const t0 = performance.now();
    for (let i = 0; i < 200; i++) world.step();
    const ms = (performance.now() - t0) / 200;
    if (wasRunning) engine.start();
    return { ms, organisms: world.organisms.length };
  })()`);
  check('单 tick 耗时在预算内（含神经网络前向）', tickMs.ms < 8,
    `${tickMs.ms.toFixed(3)} ms/tick · ${tickMs.organisms} 生物`);

  // ── 11. 控制台 ──
  console.log('\n=== 11. 控制台 ===');
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
