/**
 * 单文件打包器 —— 把全部 ES Modules 打成一份自包含的 HTML。
 *
 * 为什么要它：ES Modules 在 file:// 下会被 CORS 拦死，双击 HTML 跑不起来。
 * 把模块内联进 HTML 就能彻底摆脱服务器，双击即玩；这份产物同时也是 exe 的内嵌资源。
 *
 * 做法：每个模块包一层 IIFE、注册进一个极简模块表，import 改写成 __req()。
 * 不用"直接拼接"是因为模块之间会重名——比如 Mutation.js 与 Camera.js 都定义了
 * 顶层 `clamp`，拼接后后一个会覆盖前一个，且不会有任何报错。包 IIFE 给每个模块
 * 独立的词法作用域，从根上消除这类冲突；代价只是一点包装开销（一次性）。
 *
 * 用法：node tools/bundle.mjs [输出路径]
 */

import { readFileSync, writeFileSync, mkdirSync, existsSync } from 'node:fs';
import { dirname, resolve, relative, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const ENTRY = join(ROOT, 'src/main.js');
const OUT = process.argv[2] ? resolve(process.argv[2]) : join(ROOT, 'dist/evosim.html');

/** 匹配 import 语句（含跨行）：捕获导入子句与模块路径 */
const IMPORT_RE = /^import\s+([\s\S]*?)\s+from\s+['"]([^'"]+)['"];?[ \t]*$/gm;
/** 具名导出声明：export function/class/const/let/var NAME */
const EXPORT_DECL_RE = /^export\s+(?:async\s+)?(?:function|class|const|let|var)\s+([A-Za-z_$][\w$]*)/gm;
/** 具名导出清单：export { A, B as C }; */
const EXPORT_LIST_RE = /^export\s*\{([^}]*)\};?[ \t]*$/gm;

const idOf = (file) => relative(ROOT, file).replace(/\\/g, '/');

/** 解析导入子句，支持 {A, B} / {A as B} / default / * as ns */
function parseClause(clause) {
  const names = [];
  const trimmed = clause.trim();
  if (trimmed.startsWith('{')) {
    const inner = trimmed.slice(trimmed.indexOf('{') + 1, trimmed.lastIndexOf('}'));
    for (const part of inner.split(',')) {
      const p = part.trim();
      if (!p) continue;
      const as = p.split(/\s+as\s+/);
      names.push({ imported: as[0].trim(), local: (as[1] || as[0]).trim() });
    }
    return { names, namespace: null, defaultName: null };
  }
  if (trimmed.startsWith('*')) {
    const m = trimmed.match(/\*\s+as\s+([A-Za-z_$][\w$]*)/);
    return { names: [], namespace: m ? m[1] : null, defaultName: null };
  }
  // 默认导入（本项目目前没用到，但别静默产出坏包）
  return { names: [], namespace: null, defaultName: trimmed.split(',')[0].trim() };
}

// ── 构建依赖图（DFS 后序 = 拓扑序）──
const order = [];
const visiting = new Set();
const visited = new Set();
const cycles = [];

function visit(file) {
  if (visited.has(file)) return;
  if (visiting.has(file)) {
    cycles.push([...visiting, file].map(idOf).join(' → '));
    return;
  }
  visiting.add(file);

  const src = readFileSync(file, 'utf8');
  const deps = [];
  let m;
  IMPORT_RE.lastIndex = 0;
  while ((m = IMPORT_RE.exec(src)) !== null) {
    const dep = resolve(dirname(file), m[2]);
    if (!existsSync(dep)) throw new Error(`找不到模块：${m[2]}（被 ${idOf(file)} 引用）`);
    deps.push(dep);
  }
  for (const d of deps) visit(d);

  visiting.delete(file);
  visited.add(file);
  order.push({ file, src });
}

visit(ENTRY);

if (cycles.length) {
  console.error('检测到循环依赖，无法安全打包：\n  ' + cycles.join('\n  '));
  process.exit(1);
}

// ── 逐模块生成 IIFE ──
const wrapped = [];

for (const { file, src } of order) {
  const id = idOf(file);
  const dir = dirname(file);

  // 1. 收集导出名
  const exports = new Set();
  let m;
  EXPORT_DECL_RE.lastIndex = 0;
  while ((m = EXPORT_DECL_RE.exec(src)) !== null) exports.add(m[1]);
  EXPORT_LIST_RE.lastIndex = 0;
  while ((m = EXPORT_LIST_RE.exec(src)) !== null) {
    for (const part of m[1].split(',')) {
      const p = part.trim();
      if (!p) continue;
      const as = p.split(/\s+as\s+/);
      exports.add((as[1] || as[0]).trim());
    }
  }

  // 2. 改写 import → const {...} = __req('id')
  const body = src.replace(IMPORT_RE, (_full, clause, path) => {
    const depId = idOf(resolve(dir, path));
    const parsed = parseClause(clause);
    if (parsed.namespace) return `const ${parsed.namespace} = __req('${depId}');`;
    if (parsed.defaultName) {
      throw new Error(`${id} 使用了默认导入（本项目不支持）：${clause.trim()}`);
    }
    if (parsed.names.length === 0) return `__req('${depId}');`;
    const binding = parsed.names.map((n) => (n.imported === n.local ? n.imported : `${n.imported}: ${n.local}`)).join(', ');
    return `const { ${binding} } = __req('${depId}');`;
  })
    // 3. 剥掉 export 关键字
    .replace(/^export\s+(?=(?:async\s+)?(?:function|class|const|let|var)\b)/gm, '')
    .replace(/^export\s*\{[^}]*\};?[ \t]*$/gm, '');

  const exportList = [...exports].join(', ');
  wrapped.push(
    `__define(${JSON.stringify(id)}, function () {\n${body.trim()}\n\n  return { ${exportList} };\n});`,
  );
}

const bundledJs = [
  '/* evosim 单文件打包产物 —— 由 tools/bundle.mjs 生成 */',
  '(function () {',
  '  "use strict";',
  '  const __cache = {};',
  '  const __factory = {};',
  '  function __define(id, fn) { __factory[id] = fn; }',
  '  function __req(id) {',
  '    if (id in __cache) return __cache[id];',
  '    const fn = __factory[id];',
  '    if (!fn) throw new Error("模块未注册: " + id);',
  '    const exports = (__cache[id] = {});',
  '    const produced = fn();',
  '    for (const k in produced) exports[k] = produced[k];',
  '    return exports;',
  '  }',
  '',
  wrapped.join('\n\n'),
  '',
  '  __req(' + JSON.stringify(idOf(ENTRY)) + ');',
  '})();',
].join('\n');

// ── 组装 HTML ──
let html = readFileSync(join(ROOT, 'index.html'), 'utf8');
const css = readFileSync(join(ROOT, 'styles/main.css'), 'utf8');

// 用函数式替换，避免 CSS/JS 里的 `$` 被当成替换模式（$$、$&、$` 等）
html = html.replace(
  /<link rel="stylesheet" href="styles\/main\.css">/,
  () => `<style>\n${css}\n</style>`,
);

const faviconPath = join(ROOT, 'favicon.svg');
if (existsSync(faviconPath) && html.includes('href="favicon.svg"')) {
  const svg = readFileSync(faviconPath, 'utf8').trim();
  const dataUri = `data:image/svg+xml;base64,${Buffer.from(svg, 'utf8').toString('base64')}`;
  html = html.replace('href="favicon.svg"', `href="${dataUri}"`);
}

html = html.replace(
  /<script type="module" src="src\/main\.js"><\/script>/,
  () => `<script>\n${bundledJs}\n</script>`,
);

// 注意：不能用 html.includes('src/main.js') 判断——打包后的脚本里
// 本身就含 "src/main.js"（作为入口模块 id）。要检查原始标签是否已被替换。
if (html.includes('<script type="module"')) {
  console.error('入口脚本替换失败——index.html 结构与预期不符');
  process.exit(1);
}

html = html.replace('</head>',
  `<!-- 由 tools/bundle.mjs 打包生成 · ${order.length} 个模块 · 零依赖单文件 -->\n</head>`);

mkdirSync(dirname(OUT), { recursive: true });
writeFileSync(OUT, html, 'utf8');

const kb = (Buffer.byteLength(html, 'utf8') / 1024).toFixed(1);
console.log(`已打包 ${order.length} 个模块 → ${relative(ROOT, OUT)}（${kb} KB）`);
