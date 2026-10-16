/**
 * 阶段 5 浏览器端到端验证：无头 Chrome + CDP。
 *
 * 逻辑测试（phase5-test.mjs）已经覆盖了存档往返与元数据。
 * 这里只验证"在真实浏览器里，人能不能用得起来"：
 *   1. 参数抽屉能打开，控件由元数据生成，改动能实时写进 Config
 *   2. 网格尺寸是延迟生效的：拖滑块不动世界，点"应用"才重建
 *   3. "恢复默认"真的把一组参数还原
 *   4. 点"保存"真的下载了一个结构合法的 JSON 存档
 *   5. 通过文件选择框读档，世界被真实还原（tick / 生物数一致）
 *   6. 快速存档 / 快速读档（localStorage）能走通
 *   7. 克隆笔刷：检视个体 → 克隆选中 → 画布投放，产出同物种个体
 *   8. 全程无控制台错误
 *
 * 注意：python -m http.server 不发 Cache-Control，Chrome 会启发式缓存 JS。
 * 复用 profile 会拿到上一版代码，所以每次跑都从干净 profile 开始。
 */

import { spawn, execSync } from 'node:child_process';
import { setTimeout as sleep } from 'node:timers/promises';
import { writeFileSync, rmSync, mkdirSync, readdirSync, readFileSync } from 'node:fs';

const CHROME = 'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe';
const BASE_PROFILE = 'C:\\Users\\安\\AppData\\Local\\Temp\\chrome-evosim-p5';
const DOWNLOADS = 'C:\\Users\\安\\AppData\\Local\\Temp\\evosim-downloads';
const URL = 'http://127.0.0.1:8131/';
const PORT = 9226;
const SHOT = 'C:\\Users\\安\\AppData\\Local\\Temp\\phase5-panel.png';

/** 同步小睡：删 profile 失败时用来等上一轮 Chrome 退干净 */
function sleepSync(ms) {
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);
}

/**
 * 准备一个干净的 profile 目录。
 *
 * 每次跑都必须从干净 profile 开始（否则 Chrome 的启发式 JS 缓存会让我们拿到旧代码）。
 * 但直接 rmSync 常常撞上"上一轮 Chrome 还没退干净"——报 EBUSY: resource busy or locked。
 * 所以先重试几次；实在删不掉就换一个带 pid 的新目录，保证一定是干净的。
 */
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
rmSync(DOWNLOADS, { recursive: true, force: true });
mkdirSync(DOWNLOADS, { recursive: true });

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
 * 结果全是假的（表现为"启动时就有克隆蓝图""tick 已经好几千"这类怪现象）。
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

/** 在页面里按标签文本找一行参数控件 */
const ROW_HELPER = `
  window.__findRow = (labelText) => {
    const rows = [...document.querySelectorAll('.param-row')];
    return rows.find((r) => {
      const l = r.querySelector('.param-label');
      return l && l.textContent === labelText;
    }) || null;
  };
`;

async function main() {
  const target = await getPageTarget();
  const client = makeClient(target.webSocketDebuggerUrl);
  await client.ready;
  await client.send('Runtime.enable');
  await client.send('Log.enable');
  await client.send('Page.enable');
  await client.send('DOM.enable');
  await client.send('Browser.setDownloadBehavior', {
    behavior: 'allow', downloadPath: DOWNLOADS, eventsEnabled: true,
  });

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
    await client.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y, button: 'none', buttons: 0 });
    await client.send('Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', buttons: 1, clickCount: 1 });
    await client.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x, y, button: 'left', buttons: 0, clickCount: 1 });
    await sleep(220);
  };

  const clickEl = async (selector) => evaluate(
    `(() => { const el = document.querySelector(${JSON.stringify(selector)}); if (!el) return false; el.click(); return true; })()`);

  await sleep(3000);
  await evaluate(ROW_HELPER);

  // ── 1. 启动状态 ──
  console.log('=== 1. 启动状态 ===');
  const boot = await evaluate(`(() => {
    const { controlPanel, Config, world, editor, inspector } = window.evosim;
    return {
      tick: world.tick,
      organisms: world.organisms.length,
      badge: document.getElementById('stage-badge').textContent,
      panelOpen: controlPanel.isOpen,
      hasSave: !!document.getElementById('btn-save'),
      hasLoad: !!document.getElementById('btn-load'),
      hasParams: !!document.getElementById('btn-params'),
      fileInput: !!document.getElementById('save-file'),
      groups: document.querySelectorAll('.param-group').length,
      rows: document.querySelectorAll('.param-row').length,
      sliders: document.querySelectorAll('.param-slider').length,
      switches: document.querySelectorAll('.param-switch').length,
      blueprintAtBoot: !!editor.cloneBlueprint,
      inspectorAtBoot: inspector.isOpen,
    };
  })()`);
  // 这两条同时是"连上的是不是一个干净的新页面"的哨兵：
  // 如果上一轮 Chrome 没退干净、我们连到了旧实例，这里会立刻报警。
  check('启动时没有预置克隆蓝图', boot.blueprintAtBoot === false,
    boot.blueprintAtBoot ? '启动即有蓝图（可能连到了旧实例）' : '干净');
  check('启动时检视器是关的', boot.inspectorAtBoot === false);

  check('页面跑起来了', boot.tick >= 0, `tick ${boot.tick}`);
  check('世界里有生物', boot.organisms > 0, `${boot.organisms} 个`);
  check('徽章已填充阶段号', /^阶段 \d/.test(boot.badge), boot.badge);
  check('参数抽屉默认关闭', boot.panelOpen === false);
  check('顶栏有参数/保存/读取按钮', boot.hasParams && boot.hasSave && boot.hasLoad);
  check('隐藏的文件选择框存在', boot.fileInput === true);
  check('参数面板按分组生成（含"细胞属性"）', boot.groups === 10, `${boot.groups} 组`);
  check('参数控件由元数据生成（行/滑块/开关）',
    boot.rows > 30 && boot.sliders > 20 && boot.switches === 8,
    `${boot.rows} 行 · ${boot.sliders} 滑块 · ${boot.switches} 开关`);

  // ── 2. 打开参数抽屉 ──
  console.log('\n=== 2. 打开参数抽屉 ===');
  await clickEl('#btn-params');
  await sleep(300);
  const drawer = await evaluate(`(() => {
    const { controlPanel } = window.evosim;
    const root = controlPanel.root;
    const rect = root.getBoundingClientRect();
    return {
      open: controlPanel.isOpen,
      hidden: root.hidden,
      w: Math.round(rect.width), h: Math.round(rect.height),
      btnActive: document.getElementById('btn-params').classList.contains('is-active'),
      firstGroupOpen: document.querySelector('.param-group').open,
    };
  })()`);
  check('点"参数"后抽屉打开', drawer.open === true);
  check('抽屉浮层可见且有尺寸', drawer.hidden === false && drawer.w > 200 && drawer.h > 300,
    `${drawer.w}×${drawer.h}`);
  check('顶栏按钮同步高亮', drawer.btnActive === true);
  check('第一组默认展开', drawer.firstGroupOpen === true);

  // ── 3. 改动实时写入 Config ──
  console.log('\n=== 3. 参数改动实时生效 ===');
  const sliderChange = await evaluate(`(() => {
    const { Config } = window.evosim;
    const row = window.__findRow('生产者造食率');
    if (!row) return { found: false };
    const before = Config.life.foodProdProb;
    const s = row.querySelector('.param-slider');
    s.value = '42';
    s.dispatchEvent(new Event('input', { bubbles: true }));
    return {
      found: true,
      before,
      after: Config.life.foodProdProb,
      label: row.querySelector('.param-value').textContent,
    };
  })()`);
  check('找到"生产者造食率"滑块', sliderChange.found === true);
  check('拖动滑块实时写入 Config', sliderChange.after === 42,
    `${sliderChange.before} → ${sliderChange.after}`);
  check('数值标签同步刷新', sliderChange.label === '42', sliderChange.label);

  const boolChange = await evaluate(`(() => {
    const { Config, world } = window.evosim;
    const row = window.__findRow('边界环绕');
    if (!row) return { found: false };
    const before = { cfg: Config.world.wrapEdges, grid: world.grid.wrap };
    row.querySelector('.param-switch').click();
    return {
      found: true, before,
      after: { cfg: Config.world.wrapEdges, grid: world.grid.wrap },
      on: row.querySelector('.param-switch').classList.contains('is-on'),
    };
  })()`);
  check('找到"边界环绕"开关', boolChange.found === true);
  check('切换开关同时改 Config 与网格', boolChange.after.cfg === !boolChange.before.cfg
    && boolChange.after.grid === boolChange.after.cfg,
    `Config ${boolChange.before.cfg}→${boolChange.after.cfg} · grid.wrap ${boolChange.before.grid}→${boolChange.after.grid}`);
  check('开关视觉状态同步', boolChange.on === boolChange.after.cfg);

  // ── 4. 网格尺寸延迟生效 ──
  console.log('\n=== 4. 网格尺寸延迟生效 ===');
  const deferred = await evaluate(`(() => {
    const { Config, world } = window.evosim;
    const before = { cfg: Config.world.cols, grid: world.grid.cols, input: document.getElementById('world-cols').value };
    const row = window.__findRow('网格列数');
    const s = row.querySelector('.param-slider');
    s.value = '260';
    s.dispatchEvent(new Event('input', { bubbles: true }));
    return {
      before,
      afterDrag: { cfg: Config.world.cols, grid: world.grid.cols, input: document.getElementById('world-cols').value },
      hasApply: !!row.querySelector('.param-apply'),
    };
  })()`);
  check('拖动网格尺寸不立即改 Config', deferred.afterDrag.cfg === deferred.before.cfg,
    `Config.world.cols 仍是 ${deferred.afterDrag.cfg}`);
  check('拖动网格尺寸不立即改网格', deferred.afterDrag.grid === deferred.before.grid,
    `grid.cols 仍是 ${deferred.afterDrag.grid}`);
  check('顶栏列输入框同步待应用值', deferred.afterDrag.input === '260', deferred.afterDrag.input);
  check('网格尺寸行有"应用"按钮', deferred.hasApply === true);

  const applied = await evaluate(`(() => {
    const { Config, world } = window.evosim;
    const row = window.__findRow('网格列数');
    row.querySelector('.param-apply').click();
    return { cfg: Config.world.cols, grid: world.grid.cols, rows: world.grid.rows,
             organisms: world.organisms.length };
  })()`);
  check('点"应用并重建世界"后网格真的重建', applied.grid === 260, `grid.cols = ${applied.grid}`);
  check('重建后世界里有新生物', applied.organisms > 0, `${applied.organisms} 个`);

  // ── 5. 恢复默认 ──
  console.log('\n=== 5. 恢复默认 ===');
  const reset = await evaluate(`(() => {
    const { Config } = window.evosim;
    // 先制造几个偏离默认的值
    Config.life.foodProdProb = 77;
    Config.life.turnCooldown = 30;
    const group = [...document.querySelectorAll('.param-group')]
      .find((g) => g.querySelector('.param-group-head span').textContent === '生命');
    group.querySelector('.param-reset').click();
    return { foodProdProb: Config.life.foodProdProb, turnCooldown: Config.life.turnCooldown };
  })()`);
  check('"恢复默认"把生产者造食率还原', reset.foodProdProb === 5, `${reset.foodProdProb}`);
  check('"恢复默认"把转身冷却还原', reset.turnCooldown === 4, `${reset.turnCooldown}`);

  // 面板开着的时候截一张图
  const shot = await client.send('Page.captureScreenshot', { format: 'png' });
  writeFileSync(SHOT, Buffer.from(shot.data, 'base64'));
  console.log(`  截图已保存：${SHOT}`);

  // ── 6. 保存到文件（真实下载）──
  console.log('\n=== 6. 保存到文件 ===');
  await clickEl('#btn-params');            // 先收起面板，避免遮挡
  await sleep(200);
  const beforeSave = await evaluate(`(() => {
    const { world } = window.evosim;
    return { tick: world.tick, organisms: world.organisms.length, cols: world.grid.cols };
  })()`);

  await clickEl('#btn-save');
  let savePath = null;
  for (let i = 0; i < 60; i++) {
    await sleep(250);
    const files = readdirSync(DOWNLOADS).filter((f) => f.endsWith('.json'));
    if (files.length > 0) { savePath = `${DOWNLOADS}\\${files[0]}`; break; }
  }
  check('点"保存"下载了一个 JSON 存档', !!savePath, savePath || '没等到文件');

  let saveData = null;
  if (savePath) {
    saveData = JSON.parse(readFileSync(savePath, 'utf8'));
    check('存档 format 正确', saveData.format === 'evosim-world', saveData.format);
    check('存档 tick 与当前一致', saveData.tick === beforeSave.tick,
      `存档 ${saveData.tick} / 世界 ${beforeSave.tick}`);
    check('存档含个体与物种表', saveData.organisms.length > 0 && saveData.species.list.length > 0,
      `${saveData.organisms.length} 个体 · ${saveData.species.list.length} 物种`);
    check('存档含配置快照与 RNG 状态', !!saveData.config && !!saveData.rng);
    check('存档里的权重是 base64', typeof saveData.organisms[0].brain.w1 === 'string');
    check('存档文件名符合规范', /evosim-tick\d+-[\d-]+\.json$/.test(savePath.split('\\').pop()),
      savePath.split('\\').pop());
  }

  // ── 7. 从文件读档（真实文件选择）──
  console.log('\n=== 7. 从文件读档 ===');
  // 先暂停引擎：否则读档后引擎恢复运行，tick 会继续往前走，没法比对
  await evaluate(`window.evosim.engine.pause()`);
  // 再把世界搞乱：清空生物
  const dirtied = await evaluate(`(() => {
    const { world } = window.evosim;
    world.clearOrganisms();
    window.evosim.editor.onPaint && window.evosim.editor.onPaint();
    return { organisms: world.organisms.length, tick: world.tick };
  })()`);
  check('先把世界清空（制造差异）', dirtied.organisms === 0, `${dirtied.organisms} 个`);

  const { root } = await client.send('DOM.getDocument');
  const { nodeId } = await client.send('DOM.querySelector', { nodeId: root.nodeId, selector: '#save-file' });
  check('拿到文件选择框的节点', !!nodeId, `nodeId=${nodeId}`);
  await client.send('DOM.setFileInputFiles', { files: [savePath], nodeId });
  await sleep(1200);

  const afterLoad = await evaluate(`(() => {
    const { world } = window.evosim;
    return {
      tick: world.tick,
      organisms: world.organisms.length,
      cols: world.grid.cols,
      species: world.species.list.length,
      toast: document.getElementById('toast').textContent,
    };
  })()`);
  check('读档后 tick 还原', afterLoad.tick === beforeSave.tick,
    `读档 ${afterLoad.tick} / 存档 ${beforeSave.tick}`);
  check('读档后生物数量还原', afterLoad.organisms === beforeSave.organisms,
    `读档 ${afterLoad.organisms} / 存档 ${beforeSave.organisms}`);
  check('读档后网格尺寸还原', afterLoad.cols === beforeSave.cols, `${afterLoad.cols}`);
  check('读档后物种表还原', afterLoad.species === saveData.species.list.length,
    `${afterLoad.species} 条`);
  check('读档给出了提示', afterLoad.toast.includes('读档') || afterLoad.toast.includes('已读取'),
    afterLoad.toast);

  // ── 8. 快速存档 / 快速读档 ──
  console.log('\n=== 8. 快速存档 / 快速读档 ===');
  const quick = await evaluate(`(() => {
    const { world } = window.evosim;
    const before = { tick: world.tick, organisms: world.organisms.length };
    const bytes = window.evosim.quickSave();
    world.clearOrganisms();
    const cleared = world.organisms.length;
    window.evosim.quickLoad();
    return {
      before, bytes, cleared,
      after: { tick: world.tick, organisms: world.organisms.length },
      info: window.evosim.quickInfo(),
    };
  })()`);
  check('快速存档写入了 localStorage', quick.bytes > 0 && quick.info && quick.info.bytes > 0,
    `${(quick.bytes / 1024).toFixed(0)} KB`);
  check('清空后确实变空', quick.cleared === 0);
  check('快速读档把世界还原', quick.after.organisms === quick.before.organisms
    && quick.after.tick === quick.before.tick,
    `${quick.after.organisms} 个体 · tick ${quick.after.tick}`);

  // ── 9. 克隆笔刷 ──
  console.log('\n=== 9. 克隆笔刷 ===');
  const clone = await evaluate(`(() => {
    const { world, editor, inspector, engine } = window.evosim;
    engine.pause();
    const src = world.organisms[0];
    // 模拟"检视选中这个个体"
    editor.onInspect(src);
    const hasBlueprintBefore = !!editor.cloneBlueprint;
    const inspectorOpenBefore = inspector.isOpen;

    // 点工具栏"克隆选中"
    const btns = [...document.querySelectorAll('#sidebar-left .tool-btn')];
    const target = btns.find((b) => b.textContent === '克隆选中');
    if (target) target.click();

    const bp = editor.cloneBlueprint;
    return {
      srcId: src.id,
      srcFp: src.speciesFingerprint,
      hasBlueprintBefore,
      beforeLabel: hasBlueprintBefore ? bp.label : null,
      inspectorOpenBefore,
      inspectorOpenAfter: inspector.isOpen,
      brushType: editor.brushType,
      blueprintFp: bp ? bp.fingerprint : null,
      cells: bp ? bp.cells.length : 0,
      label: bp ? bp.label : '',
    };
  })()`);
  check('克隆前没有蓝图', clone.hasBlueprintBefore === false,
    clone.hasBlueprintBefore ? `已被预置：${clone.beforeLabel}` : '干净');
  check('检视个体后检视器是打开的', clone.inspectorOpenBefore === true);
  check('点"克隆选中"装入了蓝图', clone.cells > 0 && clone.blueprintFp === clone.srcFp,
    `${clone.cells} 细胞 · ${clone.label}`);
  check('自动切到克隆笔刷', clone.brushType === 'clone', clone.brushType);
  check('克隆后检视器保持打开（浮窗不再挡画布）', clone.inspectorOpenAfter === true);

  // 在画布上真实点击投放
  const spot = await evaluate(`(() => {
    const { world, camera } = window.evosim;
    const canvas = document.getElementById('world-canvas');
    const rect = canvas.getBoundingClientRect();
    const grid = world.grid;
    // 检视器现在是浮窗、还开着，投放点要避开它的包围盒，否则点击会落在面板上
    const panel = document.querySelector('.inspector-panel');
    const pr = panel ? panel.getBoundingClientRect() : null;
    const inPanel = (x, y) => pr && x >= pr.left - 8 && x <= pr.right + 8
      && y >= pr.top - 8 && y <= pr.bottom + 8;
    // 找一块够大的空地，保证克隆体能放下（3 细胞蓝图只需 3×3）
    for (let row = 4; row < grid.rows - 4; row += 2) {
      for (let col = 4; col < grid.cols - 4; col += 2) {
        let clear = true;
        for (let dr = -2; dr <= 2 && clear; dr++) {
          for (let dc = -2; dc <= 2; dc++) {
            const idx = grid.index(col + dc, row + dr);
            if (idx < 0 || grid.types[idx] !== 0) { clear = false; break; }
          }
        }
        if (!clear) continue;
        const sx = camera.worldToScreenX(col) + camera.zoom / 2;
        const sy = camera.worldToScreenY(row) + camera.zoom / 2;
        if (sx < 6 || sy < 6 || sx > rect.width - 6 || sy > rect.height - 6) continue;
        const cx = rect.left + sx;
        const cy = rect.top + sy;
        if (inPanel(cx, cy)) continue;
        return { x: cx, y: cy, col, row, count: world.organisms.length };
      }
    }
    return null;
  })()`);
  check('找到一块可以投放克隆体的空地', !!spot, spot ? `(${spot.col}, ${spot.row})` : '没找到');

  await clickAt(spot.x, spot.y);
  const afterDrop = await evaluate(`(() => {
    const { world } = window.evosim;
    const last = world.organisms[world.organisms.length - 1];
    return {
      count: world.organisms.length,
      lastFp: last ? last.speciesFingerprint : null,
    };
  })()`);
  check('画布点击投放了一个个体', afterDrop.count === spot.count + 1,
    `${spot.count} → ${afterDrop.count}`);
  check('投放出的个体与源同物种', afterDrop.lastFp === clone.srcFp, `${afterDrop.lastFp}`);

  // ── 9b. 暂停时仍然渲染（准星跟着鼠标走）──
  // 用户反馈：暂停模拟后周围操作栏都能交互，但画布上的准星也冻住了。
  // 根因是 pause() 里 cancelAnimationFrame，导致 onFrame 不再触发。
  console.log('\n=== 9b. 暂停时仍然渲染 ===');
  await evaluate(`(() => {
    const { editor } = window.evosim;
    editor.cancelClone();
    editor.setTool('paint');
    editor.setBrushSize(10);
  })()`);
  await evaluate(`window.evosim.engine.pause()`);
  await sleep(260);

  const paused = await evaluate(`(() => {
    const e = window.evosim.engine;
    return { running: e.running, rafAlive: e._rafId !== null };
  })()`);
  check('暂停后世界不再推进', paused.running === false);
  check('暂停后渲染循环仍在挂帧（_rafId 不为空）', paused.rafAlive === true);

  // 帧计数：暂停后每帧回调必须继续触发（直接盯住 onFrame）
  const frames = await evaluate(`(async () => {
    const e = window.evosim.engine;
    const orig = e.onFrame;
    let n = 0;
    e.onFrame = () => { n++; if (orig) orig(); };
    await new Promise((r) => setTimeout(r, 450));
    e.onFrame = orig;
    return n;
  })()`);
  check('暂停后每帧回调仍在触发', frames >= 8, `450ms 内 ${frames} 帧`);

  // 像素级：暂停状态下移动鼠标，准星框必须跟着动
  const cursorPt = await evaluate(`(() => {
    const wrap = document.querySelector('.canvas-wrap').getBoundingClientRect();
    const cx = Math.round(wrap.left + wrap.width / 2);
    const cy = Math.round(wrap.top + wrap.height / 2);
    return { a: { x: cx - 180, y: cy }, b: { x: cx + 180, y: cy } };
  })()`);
  const regionHash = (pt) => evaluate(`(() => {
    const c = document.getElementById('world-canvas');
    const r = c.getBoundingClientRect();
    const ctx = c.getContext('2d');
    const sx = c.width / r.width, sy = c.height / r.height;
    const x = Math.round((${pt.x} - r.left) * sx);
    const y = Math.round((${pt.y} - r.top) * sy);
    const R = 34;
    const x0 = Math.max(0, x - R), y0 = Math.max(0, y - R);
    const w = Math.max(1, Math.min(c.width, x + R) - x0);
    const h = Math.max(1, Math.min(c.height, y + R) - y0);
    const img = ctx.getImageData(x0, y0, w, h).data;
    let hsh = 2166136261;
    for (let i = 0; i < img.length; i += 4) {
      hsh = ((hsh ^ (img[i] + img[i + 1] * 3 + img[i + 2] * 7)) * 16777619) >>> 0;
    }
    return hsh;
  })()`);

  // 基线：把准星清掉、强制整帧重绘，取 A / B 两处像素指纹
  await evaluate(`(() => { const r = window.evosim.renderer; r.cursor = null; r.requestFullRedraw(); })()`);
  await sleep(220);
  const baseA = await regionHash(cursorPt.a);
  const baseB = await regionHash(cursorPt.b);

  await client.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: cursorPt.a.x, y: cursorPt.a.y, button: 'none', buttons: 0 });
  await sleep(240);
  const atA_A = await regionHash(cursorPt.a);
  const atA_B = await regionHash(cursorPt.b);
  check('暂停时准星被画到了鼠标位置', atA_A !== baseA, `A 处指纹 ${baseA} → ${atA_A}`);
  check('暂停时准星不会画在别处', atA_B === baseB);

  await client.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: cursorPt.b.x, y: cursorPt.b.y, button: 'none', buttons: 0 });
  await sleep(240);
  const atB_A = await regionHash(cursorPt.a);
  const atB_B = await regionHash(cursorPt.b);
  check('暂停时准星跟着鼠标移走', atB_A === baseA && atB_B !== baseB,
    `A 回到基线 ${atB_A === baseA} · B 指纹 ${baseB} → ${atB_B}`);

  await evaluate(`window.evosim.engine.start()`);
  await sleep(200);

  // ── 9c. 橡皮擦：只清地形，不碰细胞 ──
  console.log('\n=== 9c. 橡皮擦 ===');
  await evaluate(`window.evosim.engine.pause()`);
  // 先把整个世界装进视口：这样后面找"画布上的生物"一定找得到，
  // 不会因为个体跑到视口外而偶发失败
  await evaluate(`window.evosim.toolbar.fitToWindow()`);
  await sleep(300);

  const brushLabels = await evaluate(
    `[...document.querySelectorAll('.swatch-label')].map((s) => s.textContent)`);
  check('工具栏里有「橡皮擦」笔刷', brushLabels.includes('橡皮擦'), brushLabels.join(' / '));

  // 在空地上放一堵 3×3 的墙
  const wallSpot = await evaluate(`(() => {
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
            if (i < 0 || grid.types[i] !== 0 || grid.owners[i] >= 0) { clear = false; break; }
          }
        }
        if (!clear) continue;
        for (let dr = -1; dr <= 1; dr++) for (let dc = -1; dc <= 1; dc++) {
          world.setCell(col + dc, row + dr, 2, -1);
        }
        const z = camera.zoom;
        return {
          col, row,
          x: rect.left + camera.worldToScreenX(col) + z / 2,
          y: rect.top + camera.worldToScreenY(row) + z / 2,
        };
      }
    }
    return null;
  })()`);
  check('找到空地并放好一堵 3×3 墙', !!wallSpot,
    wallSpot ? `(${wallSpot.col}, ${wallSpot.row})` : '没找到');

  await evaluate(`(() => {
    const { editor } = window.evosim;
    editor.cancelClone();
    editor.setTool('paint');
    editor.setBrushType('eraser');
    editor.setBrushSize(3);
  })()`);
  await sleep(120);
  const brushNow = await evaluate(`window.evosim.editor.brushType`);
  check('切到了橡皮擦笔刷', brushNow === 'eraser', brushNow);

  await clickAt(wallSpot.x, wallSpot.y);
  const afterEraseWall = await evaluate(`(() => {
    const grid = window.evosim.world.grid;
    let walls = 0;
    for (let dr = -1; dr <= 1; dr++) for (let dc = -1; dc <= 1; dc++) {
      const i = grid.index(${wallSpot.col} + dc, ${wallSpot.row} + dr);
      if (i >= 0 && grid.types[i] === 2) walls++;
    }
    return { walls };
  })()`);
  check('橡皮擦擦掉了墙', afterEraseWall.walls === 0, `剩余墙格 ${afterEraseWall.walls}`);

  // 对着一个生物擦：生物必须毫发无损
  const bioSpot = await evaluate(`(() => {
    const { world, camera } = window.evosim;
    const canvas = document.getElementById('world-canvas');
    const rect = canvas.getBoundingClientRect();
    const grid = world.grid;
    const panel = document.querySelector('.inspector-panel');
    const pr = panel ? panel.getBoundingClientRect() : null;
    const inPanel = (x, y) => pr && x >= pr.left - 8 && x <= pr.right + 8
      && y >= pr.top - 8 && y <= pr.bottom + 8;
    for (const org of world.organisms) {
      const col = org.col, row = org.row;
      const i = grid.index(col, row);
      if (i < 0 || grid.owners[i] !== org.id) continue;
      const z = camera.zoom;
      const sx = camera.worldToScreenX(col) + z / 2;
      const sy = camera.worldToScreenY(row) + z / 2;
      if (sx < 6 || sy < 6 || sx > rect.width - 6 || sy > rect.height - 6) continue;
      const cx = rect.left + sx, cy = rect.top + sy;
      if (inPanel(cx, cy)) continue;
      return { id: org.id, col, row, type: grid.types[i], owner: grid.owners[i],
               x: cx, y: cy, count: world.organisms.length };
    }
    return null;
  })()`);
  check('找到一个可以对着擦的生物', !!bioSpot,
    bioSpot ? `#${bioSpot.id} (${bioSpot.col}, ${bioSpot.row})` : '没找到');

  await clickAt(bioSpot.x, bioSpot.y);
  const afterEraseBio = await evaluate(`(() => {
    const { world } = window.evosim;
    const org = world.organismById(${bioSpot.id});
    const grid = world.grid;
    const i = grid.index(${bioSpot.col}, ${bioSpot.row});
    return {
      alive: !!(org && org.alive),
      count: world.organisms.length,
      cellType: i >= 0 ? grid.types[i] : -1,
      cellOwner: i >= 0 ? grid.owners[i] : -2,
    };
  })()`);
  check('橡皮擦没有杀掉生物',
    afterEraseBio.alive === true && afterEraseBio.count === bioSpot.count,
    `存活 ${afterEraseBio.alive} · 个体数 ${bioSpot.count} → ${afterEraseBio.count}`);
  check('生物身体细胞还在格子上（没被擦掉）',
    afterEraseBio.cellType !== 0 && afterEraseBio.cellOwner === bioSpot.id,
    `type=${afterEraseBio.cellType} owner=${afterEraseBio.cellOwner}`);

  await evaluate(`(() => {
    const { editor, engine } = window.evosim;
    editor.setBrushType(2);          // 换回"墙"
    editor.setBrushSize(2);
    engine.start();
  })()`);
  await sleep(200);

  // ── 9d. 全局消耗量倍率 / 食物量倍率 ──
  console.log('\n=== 9d. 全局消耗量倍率 / 食物量倍率 ===');
  const newParams = await evaluate(`(() => {
    const { controlPanel, Config } = window.evosim;
    controlPanel.open();
    const rows = [...document.querySelectorAll('.param-row')];
    const find = (t) => rows.find((r) => {
      const l = r.querySelector('.param-label');
      return l && l.textContent === t;
    }) || null;
    const burn = find('全局消耗量倍率');
    const food = find('食物量倍率');
    return {
      hasBurn: !!burn, hasFood: !!food,
      foodValue: food ? food.querySelector('.param-value').textContent : null,
      cfgBurn: Config.life.metabolismScale,
      cfgFood: Config.life.foodEnergyScale,
    };
  })()`);
  check('参数面板有「全局消耗量倍率」', newParams.hasBurn === true);
  check('参数面板有「食物量倍率」', newParams.hasFood === true,
    newParams.foodValue !== null ? `当前 ${newParams.foodValue}` : '');
  check('两个倍率的默认值正确（消耗 0 / 食物 1）',
    newParams.cfgBurn === 0 && newParams.cfgFood === 1,
    `消耗 ${newParams.cfgBurn} · 食物 ${newParams.cfgFood}`);

  const foodSlide = await evaluate(`(() => {
    const { Config } = window.evosim;
    const row = window.__findRow('食物量倍率');
    if (!row) return { found: false };
    const s = row.querySelector('.param-slider');
    s.value = '3';
    s.dispatchEvent(new Event('input', { bubbles: true }));
    return { found: true, after: Config.life.foodEnergyScale,
             label: row.querySelector('.param-value').textContent };
  })()`);
  check('拖动「食物量倍率」实时写入 Config',
    foodSlide.found === true && foodSlide.after === 3,
    foodSlide.found ? `${foodSlide.after}（标签 ${foodSlide.label}）` : '找不到滑块');

  const burnSlide = await evaluate(`(() => {
    const { Config } = window.evosim;
    const row = window.__findRow('全局消耗量倍率');
    if (!row) return { found: false };
    const s = row.querySelector('.param-slider');
    s.value = '1';
    s.dispatchEvent(new Event('input', { bubbles: true }));
    const after = Config.life.metabolismScale;
    Config.life.metabolismScale = 0;     // 还原，别影响后面的性能段
    Config.life.foodEnergyScale = 1;
    window.evosim.controlPanel.close();
    return { found: true, after };
  })()`);
  check('拖动「全局消耗量倍率」实时写入 Config',
    burnSlide.found === true && burnSlide.after === 1,
    burnSlide.found ? `倍率 ${burnSlide.after}` : '找不到滑块');

  // ── 10. Esc 关闭面板 / 性能 ──
  console.log('\n=== 10. Esc 关闭面板 / 性能 ===');
  // Esc 是"先关检视器、再关参数面板"的分层逻辑，测面板之前先把检视器关掉
  await evaluate(`window.evosim.inspector.close()`);
  await clickEl('#btn-params');
  await sleep(200);
  const reopen = await evaluate(`window.evosim.controlPanel.isOpen`);
  check('重新打开参数抽屉', reopen === true);

  await client.send('Input.dispatchKeyEvent', {
    type: 'rawKeyDown', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27, nativeVirtualKeyCode: 27,
  });
  await client.send('Input.dispatchKeyEvent', {
    type: 'keyUp', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27, nativeVirtualKeyCode: 27,
  });
  await sleep(300);
  const closed = await evaluate(`window.evosim.controlPanel.isOpen`);
  check('Esc 关闭参数抽屉', closed === false);

  await evaluate(`(() => { const e = window.evosim.engine; e.setSpeed(1); e.start(); })()`);
  // 无头浏览器的 RAF 会被合成器/CPU 竞争偶发抖动（尤其连续跑多个 CDP 脚本时）。
  // 取多次采样的最好值，避免把"环境抖动"误判成"引擎慢"。
  let bestFps = 0;
  let bestTps = 0;
  let running = false;
  let perfOrgs = 0;
  for (let i = 0; i < 8; i++) {
    await sleep(600);
    const s = await evaluate(`(() => {
      const { engine, world } = window.evosim;
      return { fps: engine.fps, tps: engine.tps, running: engine.running,
               organisms: world.organisms.length };
    })()`);
    bestFps = Math.max(bestFps, s.fps);
    bestTps = Math.max(bestTps, s.tps);
    running = s.running;
    perfOrgs = s.organisms;
  }
  check('引擎在跑', running === true);
  check('帧率健康', bestFps >= 50, `${bestFps.toFixed(1)} fps`);
  check('tick 速率达标', bestTps >= 55, `${bestTps.toFixed(1)} tick/s`);

  const tickMs = await evaluate(`(() => {
    const { world, engine } = window.evosim;
    const wasRunning = engine.running;
    engine.pause();
    for (let i = 0; i < 20; i++) world.step();
    const t0 = performance.now();
    for (let i = 0; i < 200; i++) world.step();
    const ms = (performance.now() - t0) / 200;
    if (wasRunning) engine.start();
    return { ms, organisms: world.organisms.length };
  })()`);
  check('单 tick 耗时在预算内', tickMs.ms < 8,
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
