/**
 * 构建 Windows 可执行文件 —— 用 Node 的 Single Executable Application (SEA)。
 *
 * 思路：把打包好的单文件 HTML 以 base64 内嵌进一个 Node 启动器，再用 SEA
 * 把启动器塞进一份 node.exe 的副本里，产出一个自包含的 evosim.exe。
 * 双击后它会：
 *   1. 在 127.0.0.1 上随机挑一个空闲端口起一个极简 HTTP 服务（只服务那一份 HTML）
 *   2. 用系统默认浏览器打开
 *   3. 控制台窗口留着显示状态，关掉即退出
 *
 * 为什么起服务而不是直接打开文件：部分浏览器（如 Firefox）在 file:// 下
 * 会禁用 localStorage，起服务能保证"快速存档"在所有浏览器里都能用。
 *
 * 依赖：本机 Node（用来跑 SEA 工具链）+ 一次性联网装 postject（注入 blob 用）。
 *
 * 用法：node tools/build-exe.mjs
 */

import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync, mkdirSync, copyFileSync, existsSync, statSync, rmSync } from 'node:fs';
import { resolve, dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const BUILD = join(ROOT, 'build');
const DIST = join(ROOT, 'dist');
const HTML = join(DIST, 'evosim.html');
const LAUNCHER = join(BUILD, 'launcher.cjs');
const SEA_CONFIG = join(BUILD, 'sea-config.json');
const BLOB = join(BUILD, 'sea-prep.blob');
const EXE = join(DIST, 'evosim.exe');

/** Node SEA 的 fuse 标记，各版本一致 */
const SENTINEL = 'NODE_SEA_FUSE_fce680ab2cc467b6e072b8b5df1996b2';

const node = process.execPath;
const run = (cmd, args, opts = {}) => {
  console.log(`$ ${cmd} ${args.join(' ')}`);
  return execFileSync(cmd, args, { stdio: 'inherit', ...opts });
};

// ── 0. 确保单文件产物是最新的 ──
if (!existsSync(HTML)) {
  console.log('未找到 dist/evosim.html，先执行打包…');
  run(node, [join(ROOT, 'tools/bundle.mjs')]);
}

// ── 1. 生成启动器（HTML 以 base64 内嵌，避免任何转义问题）──
mkdirSync(BUILD, { recursive: true });
const htmlB64 = readFileSync(HTML).toString('base64');

const launcher = `/**
 * evosim 桌面启动器（由 tools/build-exe.mjs 生成，请勿手改）。
 *
 * 内嵌单文件 HTML，起一个只服务它的本地 HTTP 服务，再打开默认浏览器。
 *
 * 另外提供两个内部端点，让"导出"能直接落进程序所在目录
 * （浏览器里做不到这件事，只能丢进下载文件夹）：
 *   GET  /__evosim/ping  —— 页面问"你能写盘吗？写到哪儿？"
 *   POST /__evosim/save  —— 把一段文本写进程序所在目录
 */
'use strict';

const http = require('node:http');
const fs = require('node:fs');
const path = require('node:path');
const { exec } = require('node:child_process');

/** 程序所在目录：导出的 CSV / 存档 / 生物库都落在这里 */
const APP_DIR = process.env.EVOSIM_SAVE_DIR || path.dirname(process.execPath);

/** 单次请求体上限，100MB 足够放下任何存档 */
const MAX_BODY = 100 * 1024 * 1024;

const RAW = Buffer.from('${htmlB64}', 'base64').toString('utf8');

/**
 * 往页面里注入一个标记。浏览器版（dist/evosim.html 或开发服务器）
 * 没有这个标记，所以**一个探测请求都不会发**——失败的网络请求会在
 * 控制台留下 "Failed to load resource"，而"无控制台错误"是验收硬指标。
 */
const MARKER = '<script>window.__EVOSIM_HOST__=' +
  JSON.stringify({ app: 'evosim', dir: APP_DIR }) + '</' + 'script>';

const PAGE = Buffer.from(
  RAW.indexOf('</head>') >= 0 ? RAW.replace('</head>', MARKER + '</head>') : MARKER + RAW,
  'utf8',
);

function sendJson(res, code, obj) {
  const body = Buffer.from(JSON.stringify(obj), 'utf8');
  res.writeHead(code, {
    'Content-Type': 'application/json; charset=utf-8',
    'Content-Length': body.length,
    'Cache-Control': 'no-store',
  });
  res.end(body);
}

/** 只取文件名部分，杜绝 ../ 之类的越界写 */
function safeName(raw) {
  const base = path.basename(String(raw === null || raw === undefined ? '' : raw));
  const bad = String.fromCharCode(92) + '/:*?"<>|';
  let out = '';
  for (const ch of base) out += bad.indexOf(ch) >= 0 ? '_' : ch;
  out = out.trim();
  if (!out || out === '.' || out === '..') return '';
  return out;
}

const server = http.createServer((req, res) => {
  const urlPath = (req.url || '/').split('?')[0];

  if (urlPath === '/' || urlPath === '/index.html' || urlPath === '') {
    res.writeHead(200, {
      'Content-Type': 'text/html; charset=utf-8',
      'Content-Length': PAGE.length,
      'Cache-Control': 'no-store',
    });
    res.end(PAGE);
    return;
  }

  if (urlPath === '/__evosim/ping') {
    sendJson(res, 200, { app: 'evosim', dir: APP_DIR });
    return;
  }

  if (urlPath === '/__evosim/save' && req.method === 'POST') {
    let body = '';
    let size = 0;
    let tooBig = false;
    req.on('data', (chunk) => {
      size += chunk.length;
      if (size > MAX_BODY) { tooBig = true; req.destroy(); return; }
      body += chunk;
    });
    req.on('end', () => {
      if (tooBig) return;
      try {
        const payload = JSON.parse(body);
        const name = safeName(payload && payload.filename);
        if (!name) throw new Error('文件名不合法');
        const target = path.join(APP_DIR, name);
        const content = payload && payload.content;
        fs.writeFileSync(target, String(content === null || content === undefined ? '' : content), 'utf8');
        console.log('  已保存：' + target);
        sendJson(res, 200, { ok: true, path: target });
      } catch (err) {
        sendJson(res, 400, { ok: false, error: String((err && err.message) || err) });
      }
    });
    return;
  }

  res.writeHead(404, { 'Content-Type': 'text/plain; charset=utf-8' });
  res.end('404 Not Found');
});

server.on('error', (err) => {
  console.error('启动失败：' + err.message);
  process.exit(1);
});

server.listen(0, '127.0.0.1', () => {
  const url = 'http://127.0.0.1:' + server.address().port + '/';
  console.log('');
  console.log('  evosim · 进化沙盒');
  console.log('  ───────────────────────────────────');
  console.log('  已用默认浏览器打开：' + url);
  console.log('  若浏览器没弹出，请手动访问上面这个地址。');
  console.log('');
  console.log('  导出与存档会保存到：' + APP_DIR);
  console.log('');
  console.log('  关闭这个窗口即退出程序。');
  console.log('');
  // EVOSIM_NO_OPEN=1 时只起服务不开浏览器（自动化验证用）
  if (process.env.EVOSIM_NO_OPEN === '1') return;
  exec('start "" "' + url + '"', { shell: 'cmd.exe' }, () => {});
});

process.on('SIGINT', () => process.exit(0));
process.on('SIGTERM', () => process.exit(0));
`;

writeFileSync(LAUNCHER, launcher, 'utf8');
console.log(`已生成启动器：${LAUNCHER}`);

// ── 2. SEA 配置 + 生成 blob ──
writeFileSync(SEA_CONFIG, JSON.stringify({
  main: LAUNCHER,
  output: BLOB,
  disableExperimentalSEAWarning: true,
}, null, 2), 'utf8');

run(node, ['--experimental-sea-config', SEA_CONFIG]);

// ── 3. 复制 node.exe 作为宿主 ──
// 目标文件可能正被上一轮启动的 evosim.exe 占用（Windows 会报 EBUSY）。
// 先尝试结束同名进程，再带重试地删掉旧文件。
function sleepSync(ms) {
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);
}
try { execFileSync('taskkill', ['/F', '/IM', 'evosim.exe'], { stdio: 'ignore' }); } catch { /* 没在跑 */ }
for (let i = 0; i < 10; i++) {
  try { rmSync(EXE, { force: true }); break; }
  catch { sleepSync(300); }
}
copyFileSync(node, EXE);
console.log(`已复制运行时：${node} → ${EXE}`);

// ── 4. 用 postject 注入 blob ──
// 注意：不能用 shell:true 调 npx——项目路径里有空格（"WorkBuddy AI"），
// shell 会把路径拆成多个参数，postject 会报 "Can't read resource file"。
// 所以把 postject 装到本地，用 node 直接执行它的 CLI（execFile 按 argv 传参，天然安全）。
const POSTJECT = join(ROOT, 'node_modules', 'postject', 'dist', 'cli.js');
if (!existsSync(POSTJECT)) {
  console.log('未找到 postject，正在安装…');
  execFileSync('npm', ['install', '--no-save', '--no-audit', '--no-fund', 'postject'], {
    cwd: ROOT, stdio: 'inherit', shell: true,
  });
}
if (!existsSync(POSTJECT)) throw new Error(`postject 安装后仍找不到：${POSTJECT}`);

run(node, [POSTJECT, EXE, 'NODE_SEA_BLOB', BLOB, '--sentinel-fuse', SENTINEL]);

const mb = (statSync(EXE).size / 1048576).toFixed(1);
console.log(`\n完成 ✓  ${EXE}（${mb} MB）`);
console.log('双击即可运行：它会起一个本地服务并自动打开浏览器。');
