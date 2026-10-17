#!/usr/bin/env node
/**
 * 打包交付用的单 exe。
 *
 *   node tools/package.mjs
 *
 * 做三件事：cargo build --release → 拷到仓库根的 dist/evosim2.exe → 报告体积。
 * 产物是**自包含**的：内嵌了世界渲染着色器、类型调色板、场色表和中文界面字体子集，
 * 不依赖浏览器、不依赖系统装了什么字体。
 */

import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync, statSync, existsSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const RUST = resolve(HERE, '..');
const REPO = resolve(RUST, '..');
const SRC = join(RUST, 'target', 'release', 'evosim-app.exe');
const OUT_DIR = join(REPO, 'dist');
const OUT = join(OUT_DIR, 'evosim2.exe');

console.log('构建 release …');
execFileSync('cargo', ['build', '--release', '-p', 'evosim-app'], {
  cwd: RUST,
  stdio: 'inherit',
});

if (!existsSync(SRC)) {
  console.error('构建没有产出 exe：' + SRC);
  process.exit(1);
}

mkdirSync(OUT_DIR, { recursive: true });
copyFileSync(SRC, OUT);

const kb = statSync(OUT).size / 1024;
console.log(`已写出 ${OUT}（${(kb / 1024).toFixed(2)} MB）`);
console.log('双击即可运行；控制台窗口会显示 GPU 与字体信息，关掉即退出。');
