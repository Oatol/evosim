/**
 * 阶段 9 浏览器端到端验证：无头 Chrome + CDP。
 *
 * 逻辑测试（phase9-test.mjs）已经覆盖了三种细胞的机制。这里只验证"在真实浏览器里能用"：
 *   1. 物种工坊调色板里出现 储食 / 再生 / 刺，参数面板「生命」分组多出两个旋钮
 *   2. 在工坊里画出带新细胞的蓝图 → 投放 → 世界里的个体真的带上了对应标志位
 *   3. 储食：繁殖阈值被抬高，繁殖后能量留着（储备）
 *   4. 再生：累积伤害每 tick 被抹掉一点
 *   5. 刺：杀手贴上带刺的个体 → 双方都掉血
 *   6. 全程无控制台错误
 *
 * 注意：python -m http.server 不发 Cache-Control，Chrome 会启发式缓存 JS。
 * 复用 profile 会拿到上一版代码，所以每次跑都从干净 profile 开始。
 */

import { spawn, execSync } from 'node:child_process';
import { setTimeout as sleep } from 'node:timers/promises';
import { rmSync, mkdirSync } from 'node:fs';

const CHROME = 'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe';
const BASE_PROFILE = 'C:\\Users\\安\\AppData\\Local\\Temp\\chrome-evosim-p9';
const URL = 'http://127.0.0.1:8131/';
const PORT = 9230;

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

  // ── 1. 启动状态：调色板与细胞属性表 ──
  console.log('=== 1. 启动状态 ===');
  const boot = await evaluate(`(() => {
    const { world, Config, CellType } = window.evosim;
    return {
      tick: world.tick,
      organisms: world.organisms.length,
      badge: document.getElementById('stage-badge').textContent,
      cellAttrs: Object.keys(Config.cells).length,
      ids: { STORAGE: CellType.STORAGE, REGEN: CellType.REGEN, SPIKE: CellType.SPIKE },
    };
  })()`);
  check('页面跑起来了', boot.tick >= 0, `tick ${boot.tick}`);
  check('世界里有生物', boot.organisms > 0, `${boot.organisms} 个`);
  check('徽章是阶段 9', /^阶段 9/.test(boot.badge), boot.badge);
  check('三个新细胞 id 是 13 / 14 / 15',
    boot.ids.STORAGE === 13 && boot.ids.REGEN === 14 && boot.ids.SPIKE === 15,
    JSON.stringify(boot.ids));
  check('Config 里有 9 种身体细胞的属性', boot.cellAttrs === 9, `${boot.cellAttrs} 种`);

  // 工坊调色板
  await evaluate(`window.evosim.speciesEditor.open()`);
  await sleep(250);
  const palette = await evaluate(`(() => {
    const labels = [...document.querySelectorAll('.se-swatch-label')].map((s) => s.textContent);
    window.evosim.speciesEditor.close();
    return labels;
  })()`);
  check('工坊调色板有 10 项', palette.length === 10, `${palette.length} 项：${palette.join(' / ')}`);
  check('调色板里有 储食 / 再生 / 刺',
    ['储食', '再生', '刺'].every((l) => palette.includes(l)), palette.join(' / '));

  // ── 2. 参数面板「生命」分组的两个新旋钮 ──
  console.log('\n=== 2. 参数面板·新旋钮 ===');
  const params = await evaluate(`(() => {
    const { controlPanel, Config } = window.evosim;
    controlPanel.open();
    const groups = [...document.querySelectorAll('.param-group')];
    const g = groups.find((x) => x.querySelector('.param-group-head span').textContent === '生命');
    const labels = g ? [...g.querySelectorAll('.param-label')].map((l) => l.textContent) : [];
    return {
      hasGroup: !!g,
      labels,
      storageBonus: Config.life.storageFoodBonus,
      regenRate: Config.life.regenRate,
    };
  })()`);
  check('参数面板有「生命」分组', params.hasGroup === true);
  check('有「储食·储备倍率」', params.labels.includes('储食·储备倍率'), params.labels.join(' / '));
  check('有「再生·每tick回血」', params.labels.includes('再生·每tick回血'));
  check('默认储备倍率 1 / 回血 0.1',
    params.storageBonus === 1 && params.regenRate === 0.1,
    `${params.storageBonus} / ${params.regenRate}`);

  // 拖「再生·每tick回血」到 0.5，确认实时写进 Config
  const slide = await evaluate(`(() => {
    const { Config } = window.evosim;
    const rows = [...document.querySelectorAll('.param-row')];
    const row = rows.find((r) => {
      const l = r.querySelector('.param-label');
      return l && l.textContent === '再生·每tick回血';
    });
    if (!row) return { found: false };
    const s = row.querySelector('.param-slider');
    s.value = '0.5';
    s.dispatchEvent(new Event('input', { bubbles: true }));
    return { found: true, after: Config.life.regenRate };
  })()`);
  check('拖动回血参数实时写进 Config', slide.found === true && slide.after === 0.5,
    slide.found ? `regenRate = ${slide.after}` : '找不到滑块');
  await evaluate(`window.evosim.controlPanel.close()`);

  // ── 3. 在工坊里造出带新细胞的个体并投放 ──
  console.log('\n=== 3. 工坊造个体 ===');
  await evaluate(`(() => {
    const { world, engine } = window.evosim;
    engine.pause();
    world.clearOrganisms();
    world.fillGrid(0);
    window.__t = {
      spawn(cells, col, row) {
        const { speciesEditor, world: w } = window.evosim;
        speciesEditor.open();
        speciesEditor.cells.clear();
        for (const [dx, dy, type] of cells) speciesEditor.cells.set(dx + ',' + dy, { type, dir: 0 });
        const bp = speciesEditor._toBlueprint();
        speciesEditor.onSpawn(bp, 'cdp');
        speciesEditor.close();
        const o = w.organisms[w.organisms.length - 1];
        o.clearGrid();
        o.col = col; o.row = row;
        o.normalizePosition();
        o.writeGrid();
        return o;
      },
    };
    return true;
  })()`);
  await sleep(200);

  const built = await evaluate(`(() => {
    const C = window.evosim.CellType;
    const o = window.__t.spawn([[0, 0, C.MOUTH], [1, 0, C.STORAGE]], 60, 60);
    return {
      cells: o.anatomy.cells.length,
      hasStorage: o.anatomy.hasStorage,
      hasRegen: o.anatomy.hasRegen,
      hasSpike: o.anatomy.hasSpike,
      storageCount: o.storageCount(),
      foodNeeded: o.foodNeeded(),
      reproduceAt: o.reproduceAt(),
      desc: o.anatomy.describe(),
    };
  })()`);
  check('工坊造出的个体是 2 格', built.cells === 2, `${built.cells} 格：${built.desc}`);
  check('它带上了 hasStorage 标志位', built.hasStorage === true && built.storageCount === 1);
  check('它没有再生/刺标志位', built.hasRegen === false && built.hasSpike === false);
  check('繁殖阈值被储食抬高（2 → 4）',
    built.foodNeeded === 2 && built.reproduceAt === 4,
    `成本 ${built.foodNeeded} / 阈值 ${built.reproduceAt}`);

  // ── 4. 储食：繁殖后留下储备 ──
  console.log('\n=== 4. 储食：繁殖后留下储备 ===');
  const fat = await evaluate(`(() => {
    const { world } = window.evosim;
    const o = world.organisms[world.organisms.length - 1];
    o.energy = o.reproduceAt();          // 刚好到阈值
    const before = world.organisms.length;
    o.act();                             // 繁殖
    return {
      before, after: world.organisms.length,
      energy: o.energy,
      base: o.foodNeeded(),
    };
  })()`);
  check('到阈值后真的繁殖了', fat.after === fat.before + 1,
    `${fat.before} → ${fat.after} 个`);
  check('繁殖只扣基础成本，差额留作储备', Math.abs(fat.energy - fat.base) < 1e-6,
    `能量 ${fat.energy.toFixed(3)}（基础成本 ${fat.base}）`);

  // ── 5. 再生：累积伤害被抹掉 ──
  console.log('\n=== 5. 再生：每 tick 回血 ===');
  const regen = await evaluate(`(() => {
    const { world, Config } = window.evosim;
    const C = window.evosim.CellType;
    world.clearOrganisms();
    world.fillGrid(0);
    Config.life.regenRate = 0.5;         // 每个再生细胞 0.5
    const o = window.__t.spawn([[0, 0, C.MOUTH], [1, 0, C.REGEN], [2, 0, C.REGEN]], 60, 60);
    o.damage = 2;
    const hasRegen = o.anatomy.hasRegen;
    o.act();
    const after1 = o.damage;
    o.act();
    const after2 = o.damage;
    // 对照：没有再生细胞的生物不会自己回血
    const p = window.__t.spawn([[0, 0, C.MOUTH]], 100, 60);
    p.damage = 2;
    p.act();
    return { hasRegen, after1, after2, plainAfter: p.damage };
  })()`);
  check('带上了 hasRegen 标志位', regen.hasRegen === true);
  check('2 个再生细胞 × 0.5 = 每 tick 回 1 点', Math.abs(regen.after1 - 1) < 1e-6,
    `2 → ${regen.after1}`);
  check('再走一 tick 伤害归 0', Math.abs(regen.after2) < 1e-6, `→ ${regen.after2}`);
  check('没有再生细胞的不回血', Math.abs(regen.plainAfter - 2) < 1e-6, `${regen.plainAfter}`);

  // ── 6. 刺：被杀手攻击时反伤 ──
  console.log('\n=== 6. 刺：反伤 ===');
  const spike = await evaluate(`(() => {
    const { world } = window.evosim;
    const C = window.evosim.CellType;
    world.clearOrganisms();
    world.fillGrid(0);
    const k = window.__t.spawn([[0, 0, C.KILLER]], 60, 60);   // 杀手在左
    const s = window.__t.spawn([[0, 0, C.SPIKE]], 61, 60);    // 刺在右，紧挨着
    const sHasSpike = s.anatomy.hasSpike;
    const back = s.spikeDamage();
    k.act();                                                  // 杀手出手
    return {
      sHasSpike, back,
      killerDamage: k.damage,
      spikeDamage: s.damage,
      kAlive: k.alive, sAlive: s.alive,
    };
  })()`);
  check('带上了 hasSpike 标志位', spike.sHasSpike === true);
  check('刺的反伤值是 1', spike.back === 1);
  check('杀手打中了刺生物', spike.spikeDamage === 1, `刺生物受伤 ${spike.spikeDamage}`);
  check('杀手被反伤了 1 点', spike.killerDamage === 1, `杀手受伤 ${spike.killerDamage}`);

  // ── 7. 性能与复位 ──
  console.log('\n=== 7. 性能 ===');
  // 前面几节把世界清空成了实验台，这里点"重新生成世界"恢复一份正常生态再测帧率
  await evaluate(`document.getElementById('btn-regenerate').click()`);
  await sleep(500);
  await evaluate(`(() => { const e = window.evosim.engine; e.setSpeed(1); e.start(); })()`);
  let bestFps = 0;
  for (let i = 0; i < 8; i++) {
    await sleep(600);
    const fps = await evaluate(`window.evosim.engine.fps`);
    if (fps > bestFps) bestFps = fps;
  }
  check('重新生成后世界里有生物', await evaluate(`window.evosim.world.organisms.length`) > 0);
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
