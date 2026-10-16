/**
 * exe 写盘通道验证：确认"导出 / 存档保存到程序启动目录"这条链路真的通。
 *
 * 浏览器版做不到"写进程序目录"，只能丢进下载文件夹。桌面版（dist/evosim.exe）
 * 内嵌单文件 HTML、起一个本地服务，并额外提供两个内部端点：
 *   GET  /__evosim/ping  —— 页面问"你能写盘吗？写到哪儿？"
 *   POST /__evosim/save  —— 把一段文本写进程序所在目录
 * 同时在 HTML 的 </head> 前注入 window.__EVOSIM_HOST__ 标记，
 * 浏览器版没有这个标记就一个探测请求都不发（避免控制台报错）。
 *
 * 这里直接对着真 exe 打，验证：
 *   1. exe 起得来，服务能响应
 *   2. 页面里确实注入了宿主标记，且 dir 指向我们指定的目录
 *   3. ping 返回的目录 = 指定目录
 *   4. POST save 真把文件写进了那个目录，内容逐字节一致
 *   5. 文件名里的路径穿越（../../evil.txt）被清洗掉，写不出目录
 *   6. 非法文件名（空 / 全是非法字符）被拒
 *
 * 用法：node tools/verify-exe.mjs
 *   EVOSIM_SAVE_DIR 会指向一个临时目录，避免污染 dist/。
 */

import { spawn } from 'node:child_process';
import { setTimeout as sleep } from 'node:timers/promises';
import {
  rmSync, mkdirSync, existsSync, readFileSync, readdirSync, statSync,
} from 'node:fs';
import { resolve, dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const EXE = join(ROOT, 'dist/evosim.exe');
const SAVE_DIR = 'C:\\Users\\安\\AppData\\Local\\Temp\\evosim-save-test';

if (!existsSync(EXE)) {
  console.error(`找不到产物：${EXE}\n先跑 node tools/build-exe.mjs`);
  process.exit(1);
}

let failures = 0;
let checks = 0;
function check(name, condition, extra = '') {
  checks++;
  const ok = !!condition;
  if (!ok) failures++;
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}${extra ? '  → ' + extra : ''}`);
}

rmSync(SAVE_DIR, { recursive: true, force: true });
mkdirSync(SAVE_DIR, { recursive: true });

const child = spawn(EXE, [], {
  env: { ...process.env, EVOSIM_NO_OPEN: '1', EVOSIM_SAVE_DIR: SAVE_DIR },
  stdio: ['ignore', 'pipe', 'pipe'],
});

let stdout = '';
let stderr = '';
child.stdout.on('data', (d) => { stdout += d.toString(); });
child.stderr.on('data', (d) => { stderr += d.toString(); });

function killTree() {
  try {
    if (process.platform === 'win32') {
      spawn('taskkill', ['/F', '/T', '/PID', String(child.pid)], { stdio: 'ignore' });
    } else {
      child.kill('SIGKILL');
    }
  } catch { /* 已经退了 */ }
}

async function waitForPort() {
  for (let i = 0; i < 80; i++) {
    const m = stdout.match(/127\.0\.0\.1:(\d+)\//);
    if (m) return Number(m[1]);
    if (child.exitCode !== null) throw new Error(`exe 提前退出（code ${child.exitCode}）\n${stderr}`);
    await sleep(250);
  }
  throw new Error('等不到 exe 打印端口\nstdout: ' + stdout + '\nstderr: ' + stderr);
}

async function main() {
  console.log(`启动：${EXE}`);
  console.log(`导出目录：${SAVE_DIR}\n`);

  const port = await waitForPort();
  const base = `http://127.0.0.1:${port}`;
  check('exe 起来了并打印了访问地址', port > 0, `端口 ${port}`);
  check('启动日志里写明了导出目录', stdout.includes(SAVE_DIR));

  // ── 1. 页面注入的宿主标记 ──
  const html = await (await fetch(base + '/')).text();
  const markerMatch = html.match(/window\.__EVOSIM_HOST__=(\{.*?\})<\/script>/);
  check('页面注入了 window.__EVOSIM_HOST__ 标记', !!markerMatch,
    markerMatch ? markerMatch[1].slice(0, 80) : '没找到');
  let marker = null;
  if (markerMatch) { try { marker = JSON.parse(markerMatch[1]); } catch { /* 忽略 */ } }
  check('标记里 app 是 evosim', marker && marker.app === 'evosim', marker ? marker.app : 'null');
  check('标记里的目录 = 指定的导出目录', marker && marker.dir === SAVE_DIR,
    marker ? marker.dir : 'null');
  check('页面本体也送达了（内嵌 HTML 非空）', html.length > 100000,
    `${(html.length / 1024).toFixed(0)} KB`);

  // ── 2. ping ──
  const ping = await (await fetch(base + '/__evosim/ping')).json();
  check('ping 返回可写盘', ping.ok !== false && ping.app === 'evosim',
    JSON.stringify(ping));
  check('ping 返回的目录正确', ping.dir === SAVE_DIR, ping.dir);

  // ── 3. 写盘：普通文件名 ──
  const fname = 'evosim-creatures-verify.json';
  const content = '{"hello":"世界","n":42}';
  const saveRes = await fetch(base + '/__evosim/save', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ filename: fname, content }),
  });
  const saveJson = await saveRes.json();
  check('save 返回 200 且 ok', saveRes.status === 200 && saveJson.ok === true,
    JSON.stringify(saveJson).slice(0, 120));
  const written = join(SAVE_DIR, fname);
  check('文件真的落在了指定目录', existsSync(written), written);
  if (existsSync(written)) {
    const round = readFileSync(written, 'utf8');
    check('写入内容逐字节一致', round === content, `${round.length} 字符`);
  }

  // ── 4. 路径穿越必须被清洗 ──
  const evil = await fetch(base + '/__evosim/save', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ filename: '../../evil.txt', content: 'x' }),
  });
  const evilJson = await evil.json();
  const evilLanded = existsSync(join(SAVE_DIR, 'evil.txt'));
  const escaped = existsSync(join(SAVE_DIR, '..', '..', 'evil.txt'))
    || existsSync(resolve(SAVE_DIR, '..', 'evil.txt'));
  check('路径穿越被清洗成纯文件名', evilJson.ok === true && evilLanded,
    evilJson.path ? evilJson.path : JSON.stringify(evilJson));
  check('没有文件被写到目录之外', escaped === false);
  const outside = readdirSync(resolve(SAVE_DIR, '..')).filter((n) => n === 'evil.txt');
  check('上级目录里没有多出 evil.txt', outside.length === 0);

  // ── 5. 非法文件名被拒 ──
  const bad = await fetch(base + '/__evosim/save', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ filename: '   ', content: 'x' }),
  });
  check('空文件名被拒绝', bad.status === 400, `HTTP ${bad.status}`);

  // ── 6. 未知路径 404 ──
  const nf = await fetch(base + '/nope');
  check('未知路径返回 404', nf.status === 404, `HTTP ${nf.status}`);

  // 目录里最终应该只有预期的两个文件
  const files = readdirSync(SAVE_DIR).sort();
  check('导出目录内容符合预期', files.length === 2 && files.includes(fname) && files.includes('evil.txt'),
    files.join(', '));
  const sizes = files.map((f) => `${f}=${statSync(join(SAVE_DIR, f)).size}B`);
  console.log(`     目录：${sizes.join(' · ')}`);

  console.log(`\n${'='.repeat(52)}`);
  if (failures === 0) console.log(`全部通过 ✓  (${checks} 项)`);
  else console.log(`通过 ${checks - failures} 项，失败 ${failures} 项 ✗`);
}

main()
  .catch((err) => {
    console.error('验证崩溃:', err.message);
    failures++;
  })
  .finally(async () => {
    killTree();
    await sleep(400);
    try { rmSync(SAVE_DIR, { recursive: true, force: true }); } catch { /* 忽略 */ }
    process.exit(failures === 0 ? 0 : 1);
  });
