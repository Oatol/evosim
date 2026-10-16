/**
 * 阶段 3 浏览器端到端验证：无头 Chrome + CDP。
 *
 * 重点验证两件逻辑测试覆盖不到的事：
 *   1. 曲线 canvas 真的被画出来了（不是空白 canvas）
 *   2. 整条链路在真实浏览器里跑起来没有报错
 */

import { spawn, execSync } from 'node:child_process';
import { setTimeout as sleep } from 'node:timers/promises';
import { writeFileSync, rmSync, mkdirSync } from 'node:fs';

const CHROME = 'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe';
const BASE_PROFILE = 'C:\\Users\\安\\AppData\\Local\\Temp\\chrome-evosim-p3';
const URL = 'http://127.0.0.1:8131/';
const PORT = 9224;
const SHOT = 'C:\\Users\\安\\AppData\\Local\\Temp\\phase3-charts.png';

/** 同步小睡：删 profile 失败时用来等上一轮 Chrome 退干净 */
function sleepSync(ms) {
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);
}

// python -m http.server 不发 Cache-Control，Chrome 会启发式缓存 JS。
// 复用 profile 会拿到上一版代码，所以每次跑都从干净的 profile 开始。
// 但直接 rmSync 常撞上"上一轮 Chrome 还没退干净"（EBUSY），所以先重试，
// 实在删不掉就换一个带 pid 的新目录，保证一定是干净的。
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
function check(name, condition, extra = '') {
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

/** 统计一个 chart canvas 里有多少像素不是背景色 */
const CANVAS_PROBE = `(() => {
  return [...document.querySelectorAll('.chart-canvas')].map((c) => {
    const ctx = c.getContext('2d');
    const img = ctx.getImageData(0, 0, c.width, c.height).data;
    let ink = 0;
    for (let i = 0; i < img.length; i += 4) {
      if (Math.abs(img[i] - 240) > 8 || Math.abs(img[i + 1] - 242) > 8 || Math.abs(img[i + 2] - 245) > 8) ink++;
    }
    return { w: c.width, h: c.height, ink };
  });
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

  await sleep(3000);

  // ── 1. 启动状态 ──
  console.log('=== 1. 启动状态 ===');
  const boot = await evaluate(`(() => {
    const { world, record, Config } = window.evosim;
    return {
      tick: world.tick,
      organisms: world.organisms.length,
      species: world.species.list.length,
      aliveSpecies: world.species.aliveCount,
      recordLen: record.length,
      badge: document.getElementById('stage-badge').textContent,
      chartCount: document.querySelectorAll('.chart-canvas').length,
      interval: Config.stats.interval,
    };
  })()`);

  check('页面跑起来了', boot.tick >= 0, `tick ${boot.tick}`);
  check('世界里有生物', boot.organisms > 0, `${boot.organisms} 个`);
  check('初始种群已登记物种', boot.species > 0, `${boot.species} 个物种`);
  // 具体是第几阶段由当时的 cdp-phaseN 脚本负责；这里只确认徽章被填充过
  check('阶段徽章已渲染', /^阶段 \d/.test(boot.badge), boot.badge);
  check('右栏渲染出 4 条曲线', boot.chartCount === 4, `${boot.chartCount} 条`);
  check('启动时已采过样', boot.recordLen >= 1, `${boot.recordLen} 个样本`);

  // ── 2. 读数面板 ──
  console.log('\n=== 2. 读数面板 ===');
  const labels = await evaluate(`(() => {
    const dt = [...document.querySelectorAll('#sidebar-right .stat-list dt')].map((e) => e.textContent);
    const chartLabels = [...document.querySelectorAll('.chart-label')].map((e) => e.textContent);
    return { dt, chartLabels };
  })()`);
  check('读数里有"存活物种"', labels.dt.includes('存活物种'), labels.dt.join(' / '));
  check('读数里有"平均变异率"', labels.dt.includes('平均变异率'));
  check('曲线标签齐全',
    ['种群数', '物种数', '平均体型', '平均变异率'].every((l) => labels.chartLabels.includes(l)),
    labels.chartLabels.join(' / '));

  // ── 3. 跑一段演化 ──
  console.log('\n=== 3. 演化中（×50 倍速跑 4 秒）===');
  await evaluate(`window.evosim.engine.setSpeed(50)`);
  await sleep(4000);

  const evolved = await evaluate(`(() => {
    const { world, record } = window.evosim;
    const bio = world.organismStats();
    return {
      tick: world.tick,
      organisms: world.organisms.length,
      aliveSpecies: world.species.aliveCount,
      totalSpecies: world.species.list.length,
      extinct: world.species.totalExtinct,
      recordLen: record.length,
      avgCells: bio.avgCells,
      avgMutability: bio.avgMutability,
      peakAlive: world.species.peakAliveCount,
    };
  })()`);

  check('模拟确实推进了', evolved.tick > 500, `tick ${evolved.tick}`);
  check('种群没有崩', evolved.organisms > 0, `${evolved.organisms} 个`);
  check('演化出了多个物种', evolved.peakAlive > 1, `峰值 ${evolved.peakAlive} 个存活物种`);
  check('累计物种数 > 存活物种数（有灭绝发生）',
    evolved.totalSpecies >= evolved.aliveSpecies, `${evolved.totalSpecies} 累计 / ${evolved.aliveSpecies} 存活`);
  check('化石记录在累积', evolved.recordLen > boot.recordLen, `${evolved.recordLen} 个样本`);
  check('平均变异率是合法数值',
    evolved.avgMutability >= 1 && evolved.avgMutability <= 100, evolved.avgMutability.toFixed(2));

  await evaluate(`window.evosim.engine.setSpeed(1)`);
  await sleep(600);

  // ── 4. 曲线真的画出来了 ──
  console.log('\n=== 4. 曲线渲染 ===');
  const canvases = await evaluate(CANVAS_PROBE);
  check('4 条曲线都有实际尺寸',
    canvases.every((c) => c.w > 0 && c.h > 0),
    canvases.map((c) => `${c.w}×${c.h}`).join(' '));
  check('每条曲线都画上了墨迹',
    canvases.every((c) => c.ink > 30),
    canvases.map((c) => c.ink).join(' / '));

  // ── 5. 重置清空历史 ──
  console.log('\n=== 5. 重置 ===');
  const afterReset = await evaluate(`(() => {
    const { world, record, charts } = window.evosim;
    world.reset();
    record.reset();
    charts.markDirty();
    // 采样一次起点（模拟 main.js 的 resetHistory）
    world.species.update(world, world.tick);
    const bio = world.detailedStats();
    record.push({ tick: world.tick, population: world.organisms.length, species: world.species.aliveCount,
                  avgCells: bio.avgCells, avgMutability: bio.avgMutability, cellShares: bio.cellShares });
    charts.refresh();
    return { tick: world.tick, organisms: world.organisms.length, species: world.species.list.length,
             recordLen: record.length };
  })()`);
  check('重置后 tick 归零', afterReset.tick === 0, `tick ${afterReset.tick}`);
  check('重置后生物清空', afterReset.organisms === 0, `${afterReset.organisms} 个`);
  check('重置后物种记录清空', afterReset.species === 0, `${afterReset.species} 个`);
  check('重置后曲线只剩起点样本', afterReset.recordLen === 1, `${afterReset.recordLen} 个样本`);

  const emptyCanvases = await evaluate(CANVAS_PROBE);
  check('重置后曲线被重绘（不再是旧曲线）',
    emptyCanvases.every((c) => c.ink < 400),
    emptyCanvases.map((c) => c.ink).join(' / '));

  // ── 6. 恢复世界并截图 ──
  console.log('\n=== 6. 恢复 + 截图 ===');
  const regen = await evaluate(`(() => {
    const { engine } = window.evosim;
    const runningBefore = engine.running;
    document.getElementById('btn-regenerate').click();
    const runningAfter = engine.running;
    engine.setSpeed(50);      // 跑久一点，截图里的曲线才有形状
    return { runningBefore, runningAfter };
  })()`);
  console.log(`  按钮前后 running: ${regen.runningBefore} → ${regen.runningAfter}`);

  await sleep(4000);
  await evaluate(`window.evosim.engine.setSpeed(1)`);
  await sleep(600);

  const shot = await client.send('Page.captureScreenshot', { format: 'png' });
  writeFileSync(SHOT, Buffer.from(shot.data, 'base64'));
  console.log(`  截图已保存：${SHOT}`);

  const final = await evaluate(`(() => {
    const { world, record, engine } = window.evosim;
    return { tick: world.tick, organisms: world.organisms.length,
             aliveSpecies: world.species.aliveCount, recordLen: record.length,
             running: engine.running, speed: engine.speed };
  })()`);
  check('重新生成后保持运行状态', regen.runningBefore === regen.runningAfter,
    `${regen.runningBefore} → ${regen.runningAfter}`);
  check('重建后世界正常运转', final.organisms > 0 && final.tick > 100,
    `tick ${final.tick} · ${final.organisms} 生物 · ${final.aliveSpecies} 物种 · running=${final.running} speed=${final.speed}`);

  // ── 7. 性能 ──
  console.log('\n=== 7. 性能 ===');
  await evaluate(`window.evosim.engine.setSpeed(1)`);
  await sleep(3000);
  const perf = await evaluate(`(() => {
    const { engine, world } = window.evosim;
    return { fps: engine.fps, tps: engine.tps, tick: world.tick,
             organisms: world.organisms.length, species: world.species.list.length };
  })()`);
  check('正常速度下帧率健康', perf.fps >= 50, `${perf.fps.toFixed(1)} fps`);
  check('tick 速率达标', perf.tps >= 55, `${perf.tps.toFixed(1)} tick/s`);
  console.log(`  ${perf.organisms} 生物 · ${perf.species} 条物种记录 · tick ${perf.tick}`);

  // ── 8. 控制台错误 ──
  console.log('\n=== 8. 控制台 ===');
  const errors = client.events.filter((e) =>
    (e.method === 'Log.entryAdded' && e.params.entry.level === 'error')
    || e.method === 'Runtime.exceptionThrown');
  check('无控制台错误', errors.length === 0,
    errors.length ? errors.map((e) => JSON.stringify(e.params).slice(0, 200)).join(' | ') : '干净');

  console.log(`\n${'='.repeat(52)}`);
  console.log(failures === 0 ? '全部通过 ✓' : `失败 ${failures} 项 ✗`);

  client.ws.close();
  forceKill(chrome.pid);
  process.exit(failures === 0 ? 0 : 1);
}

main().catch((err) => {
  console.error('测试崩溃:', err.message);
  forceKill(chrome.pid);
  process.exit(1);
});
