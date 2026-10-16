/**
 * 阶段 8 浏览器端到端验证：无头 Chrome + CDP。
 *
 * 逻辑测试（phase8-test.mjs）已经覆盖了注册表、通行性、floor 层、三个钩子、
 * 腐殖质造食、橡皮擦语义与存档往返。这里只验证"在真实浏览器里能不能用"：
 *   1. 工具栏里 7 个地形笔刷 + 橡皮擦都在，且能选中
 *   2. 参数面板多出「地形」分组，改动能实时写进 Config
 *   3. 用橡皮擦**真实点击**：墙没了，食物与生物毫发无损
 *   4. 把生物脚下的地板设成岩浆 → 每 tick 掉血，而且地形不会被啃掉
 *   5. 腐殖质真的在往周围冒食物
 *   6. 存档带上了 terrain 段，读档后地形原样回来
 *   7. 全程无控制台错误
 *
 * 注意：python -m http.server 不发 Cache-Control，Chrome 会启发式缓存 JS。
 * 复用 profile 会拿到上一版代码，所以每次跑都从干净 profile 开始。
 */

import { spawn, execSync } from 'node:child_process';
import { setTimeout as sleep } from 'node:timers/promises';
import { rmSync, mkdirSync } from 'node:fs';

const CHROME = 'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe';
const BASE_PROFILE = 'C:\\Users\\安\\AppData\\Local\\Temp\\chrome-evosim-p8';
const URL = 'http://127.0.0.1:8131/';
const PORT = 9229;

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

  await sleep(3000);

  // ── 1. 启动状态：笔刷列表 ──
  console.log('=== 1. 启动状态 ===');
  const boot = await evaluate(`(() => {
    const { world } = window.evosim;
    return {
      tick: world.tick,
      organisms: world.organisms.length,
      badge: document.getElementById('stage-badge').textContent,
      brushes: [...document.querySelectorAll('.swatch-label')].map((s) => s.textContent),
    };
  })()`);
  check('页面跑起来了', boot.tick >= 0, `tick ${boot.tick}`);
  check('世界里有生物', boot.organisms > 0, `${boot.organisms} 个`);
  check('徽章已填充阶段号', /^阶段 \d/.test(boot.badge), boot.badge);

  const wantBrushes = ['空', '食物', '墙', '岩浆', '沼泽', '辐射带', '腐殖质', '橡皮擦', '随机生物', '克隆'];
  const missing = wantBrushes.filter((b) => !boot.brushes.includes(b));
  check('工具栏里有全部地形笔刷与橡皮擦', missing.length === 0,
    missing.length ? `缺少 ${missing.join('、')}` : boot.brushes.join(' / '));

  // ── 2. 参数面板「地形」分组 ──
  console.log('\n=== 2. 参数面板·地形分组 ===');
  const params = await evaluate(`(() => {
    const { controlPanel, Config } = window.evosim;
    controlPanel.open();
    const groups = [...document.querySelectorAll('.param-group')];
    const g = groups.find((x) => x.querySelector('.param-group-head span').textContent === '地形');
    const labels = g ? [...g.querySelectorAll('.param-label')].map((l) => l.textContent) : [];
    const rows = g ? g.querySelectorAll('.param-row').length : 0;
    controlPanel.close();
    return { hasGroup: !!g, rows, labels, cfg: JSON.parse(JSON.stringify(Config.terrain)) };
  })()`);
  check('参数面板有「地形」分组', params.hasGroup === true);
  check('地形分组有 6 项', params.rows === 6, `${params.rows} 项`);
  check('地形分组的标签齐全',
    ['岩浆·踏入伤害', '岩浆·驻留伤害', '沼泽·移动耗能', '辐射带·每tick伤害', '辐射带·变异率倍率', '腐殖质·造食率(%)']
      .every((l) => params.labels.includes(l)), params.labels.join(' / '));
  check('地形默认值合理',
    params.cfg.lavaEnterDamage === 2 && params.cfg.radMutationBoost === 3 && params.cfg.humusFoodProb === 0.5,
    JSON.stringify(params.cfg));

  const slide = await evaluate(`(() => {
    const { Config } = window.evosim;
    const rows = [...document.querySelectorAll('.param-row')];
    const row = rows.find((r) => {
      const l = r.querySelector('.param-label');
      return l && l.textContent === '腐殖质·造食率(%)';
    });
    if (!row) return { found: false };
    const s = row.querySelector('.param-slider');
    s.value = '12';
    s.dispatchEvent(new Event('input', { bubbles: true }));
    return { found: true, after: Config.terrain.humusFoodProb };
  })()`);
  check('拖动地形参数实时写进 Config', slide.found === true && slide.after === 12,
    slide.found ? `humusFoodProb = ${slide.after}` : '找不到滑块');
  await evaluate(`window.evosim.Config.terrain.humusFoodProb = 0.5`);

  // ── 3. 橡皮擦：真实点击 ──
  console.log('\n=== 3. 橡皮擦（真实点击）===');
  await evaluate(`window.evosim.engine.pause()`);
  await sleep(250);

  const setup = await evaluate(`(() => {
    const { world, camera } = window.evosim;
    const canvas = document.getElementById('world-canvas');
    const rect = canvas.getBoundingClientRect();
    const grid = world.grid;
    for (let row = 6; row < grid.rows - 6; row += 3) {
      for (let col = 6; col < grid.cols - 6; col += 3) {
        let clear = true;
        for (let dr = -2; dr <= 2 && clear; dr++) {
          for (let dc = -2; dc <= 2; dc++) {
            const i = grid.index(col + dc, row + dr);
            if (i < 0 || grid.types[i] !== 0 || grid.owners[i] >= 0 || grid.floor[i] !== 0) { clear = false; break; }
          }
        }
        if (!clear) continue;
        // 一堵 3×3 墙 + 紧挨着的一份食物
        for (let dr = -1; dr <= 1; dr++) for (let dc = -1; dc <= 1; dc++) {
          world.setCell(col + dc, row + dr, 2, -1);
        }
        world.setCell(col + 2, row, 1, -1);
        const z = camera.zoom;
        return {
          col, row,
          wall: { x: rect.left + camera.worldToScreenX(col) + z / 2,
                  y: rect.top + camera.worldToScreenY(row) + z / 2 },
          foodIdx: grid.index(col + 2, row),
          orgs: world.organisms.length,
        };
      }
    }
    return null;
  })()`);
  check('找到空地并布好「墙 + 食物」', !!setup,
    setup ? `墙心 (${setup.col}, ${setup.row})` : '没找到');

  await evaluate(`(() => {
    const { editor } = window.evosim;
    editor.cancelClone();
    editor.setTool('paint');
    editor.setBrushType('eraser');
    editor.setBrushSize(3);
  })()`);
  await sleep(150);
  check('切到了橡皮擦', await evaluate(`window.evosim.editor.brushType`) === 'eraser');

  await clickAt(setup.wall.x, setup.wall.y);
  const afterErase = await evaluate(`(() => {
    const { world } = window.evosim;
    const grid = world.grid;
    let walls = 0;
    for (let dr = -1; dr <= 1; dr++) for (let dc = -1; dc <= 1; dc++) {
      const i = grid.index(${setup.col} + dc, ${setup.row} + dr);
      if (i >= 0 && grid.types[i] === 2) walls++;
    }
    return {
      walls,
      food: grid.types[${setup.foodIdx}],
      orgs: world.organisms.length,
    };
  })()`);
  check('橡皮擦把墙擦干净了', afterErase.walls === 0, `剩余墙格 ${afterErase.walls}`);
  check('橡皮擦留下了食物', afterErase.food === 1, `type=${afterErase.food}`);
  check('橡皮擦没影响生物', afterErase.orgs === setup.orgs,
    `${setup.orgs} → ${afterErase.orgs}`);

  // ── 4. 岩浆：驻留掉血 + 地形不被啃掉 ──
  console.log('\n=== 4. 岩浆灼烧与 floor 层 ===');
  const lava = await evaluate(`(() => {
    const { world } = window.evosim;
    const grid = world.grid;
    const org = world.organisms[0];
    if (!org) return { ok: false };
    // 直接把生物脚下 5×5 的地板设成岩浆（不走 setCellAt，免得覆盖掉生物身体）
    for (let dr = -2; dr <= 2; dr++) for (let dc = -2; dc <= 2; dc++) {
      const i = grid.index(org.col + dc, org.row + dr);
      if (i >= 0) grid.floor[i] = 9;
    }
    world._initTerrainIndex();
    return { ok: true, id: org.id, col: org.col, row: org.row, damage: org.damage };
  })()`);
  check('把一只生物脚下的地板换成了岩浆', lava.ok === true,
    lava.ok ? `#${lava.id} @(${lava.col}, ${lava.row})` : '没有生物');

  await evaluate(`window.evosim.engine.start()`);
  await sleep(900);
  await evaluate(`window.evosim.engine.pause()`);
  await sleep(200);

  const burned = await evaluate(`(() => {
    const { world } = window.evosim;
    const org = world.organismById(${lava.id});
    const grid = world.grid;
    const i = grid.index(${lava.col}, ${lava.row});
    return {
      alive: !!(org && org.alive),
      damage: org ? org.damage : -1,
      floor: grid.floor[i],
      moved: org ? (org.col !== ${lava.col} || org.row !== ${lava.row}) : null,
    };
  })()`);
  check('岩浆把生物烤伤了（或烤死了）',
    burned.alive === false || burned.damage > lava.damage,
    `伤害 ${lava.damage} → ${burned.damage.toFixed(2)}，存活 ${burned.alive}`);
  check('岩浆没被生物啃掉（floor 层保住了）', burned.floor === 9, `floor=${burned.floor}`);

  // ── 5. 腐殖质造食 ──
  console.log('\n=== 5. 腐殖质造食 ===');
  const humus = await evaluate(`(() => {
    const { world, Config } = window.evosim;
    const grid = world.grid;
    for (let row = 6; row < grid.rows - 6; row += 3) {
      for (let col = 6; col < grid.cols - 6; col += 3) {
        let clear = true;
        for (let dr = -1; dr <= 1 && clear; dr++) {
          for (let dc = -1; dc <= 1; dc++) {
            const i = grid.index(col + dc, row + dr);
            if (i < 0 || grid.types[i] !== 0 || grid.owners[i] >= 0 || grid.floor[i] !== 0) { clear = false; break; }
          }
        }
        if (!clear) continue;
        world.setCell(col, row, 12, -1);            // HUMUS
        Config.terrain.humusFoodProb = 100;         // 必定成功，方便断言
        return { col, row, foodBefore: world.foodCells.length };
      }
    }
    return null;
  })()`);
  check('找到空地放了一块腐殖质', !!humus, humus ? `(${humus.col}, ${humus.row})` : '没找到');

  await evaluate(`window.evosim.engine.start()`);
  await sleep(700);
  await evaluate(`window.evosim.engine.pause()`);
  await sleep(200);

  const humusAfter = await evaluate(`(() => {
    const { world, Config } = window.evosim;
    Config.terrain.humusFoodProb = 0.5;
    const grid = world.grid;
    // 腐殖质四邻域里的食物
    let near = 0;
    const n = [[0,-1],[1,0],[0,1],[-1,0]];
    for (const [dc, dr] of n) {
      const i = grid.index(${humus.col} + dc, ${humus.row} + dr);
      if (i >= 0 && grid.types[i] === 1) near++;
    }
    return { near, foodNow: world.foodCells.length, humusStill: grid.types[grid.index(${humus.col}, ${humus.row})] };
  })()`);
  check('腐殖质在四邻域冒出了食物', humusAfter.near > 0, `邻域食物 ${humusAfter.near} 份`);
  check('腐殖质本身还在', humusAfter.humusStill === 12, `type=${humusAfter.humusStill}`);

  // ── 6. 存档往返带上地形 ──
  console.log('\n=== 6. 存档往返（含地形）===');
  const roundTrip = await evaluate(`(() => {
    const { world } = window.evosim;
    const data = window.evosim.save();
    const hasTerrain = !!data.grid.terrain;
    const hasWallsKey = data.grid.walls !== undefined;
    const lavaCount = data.grid.terrain && data.grid.terrain.LAVA ? data.grid.terrain.LAVA.length : 0;
    const before = [];
    for (let i = 0; i < world.grid.floor.length; i++) if (world.grid.floor[i]) before.push(i + ':' + world.grid.floor[i]);
    world.clearOrganisms();
    const info = window.evosim.load(data);
    const after = [];
    for (let i = 0; i < world.grid.floor.length; i++) if (world.grid.floor[i]) after.push(i + ':' + world.grid.floor[i]);
    let diff = 0;
    if (before.length !== after.length) diff = -1;
    else for (let i = 0; i < before.length; i++) if (before[i] !== after[i]) diff++;
    return { hasTerrain, hasWallsKey, lavaCount, diff, version: data.version,
             infoTerrain: info.terrain, terrainCells: before.length };
  })()`);
  check('存档里有 terrain 段', roundTrip.hasTerrain === true);
  check('存档版本升到 2', roundTrip.version === 2, `version=${roundTrip.version}`);
  check('存档里的岩浆格数 > 0', roundTrip.lavaCount > 0, `${roundTrip.lavaCount} 格`);
  check('读档后地板逐格一致', roundTrip.diff === 0,
    `差异 ${roundTrip.diff}（共 ${roundTrip.terrainCells} 格地形）`);

  // ── 7. 性能与复位 ──
  console.log('\n=== 7. 性能 ===');
  await evaluate(`(() => { const e = window.evosim.engine; e.setSpeed(1); e.start(); })()`);
  let bestFps = 0;
  for (let i = 0; i < 8; i++) {
    await sleep(600);
    const fps = await evaluate(`window.evosim.engine.fps`);
    if (fps > bestFps) bestFps = fps;
  }
  check('帧率健康', bestFps > 40, `${bestFps.toFixed(1)} fps`);

  // ── 8. 控制台 ──
  console.log('\n=== 8. 控制台 ===');
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
  forceKill(chrome.pid);
  process.exit(failures === 0 ? 0 : 1);
}

main().catch((err) => {
  console.error('验证崩溃:', err.message);
  chrome.kill();
  forceKill(chrome.pid);
  process.exit(1);
});
