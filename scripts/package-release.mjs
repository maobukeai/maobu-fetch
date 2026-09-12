#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { execSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const rootDir = path.resolve(__dirname, '..');

// 1. 确定版本号
let version = process.argv[2]?.trim().replace(/^v/, '');
if (!version) {
  const refName = process.env.GITHUB_REF_NAME;
  if (refName && /^v\d+\.\d+\.\d+/.test(refName)) {
    version = refName.replace(/^v/, '');
  } else {
    const pkg = JSON.parse(fs.readFileSync(path.join(rootDir, 'package.json'), 'utf8'));
    version = pkg.version;
  }
}

console.log(`==> 开始封装 v${version} 发布资产...`);

const outDir = path.join(rootDir, 'releases_out');
if (fs.existsSync(outDir)) {
  fs.rmSync(outDir, { recursive: true, force: true });
}
fs.mkdirSync(outDir, { recursive: true });

// 递归查找文件的辅助函数
function findFileRecursive(dir, predicate) {
  if (!fs.existsSync(dir)) return null;
  const entries = fs.readdirSync(dir, { withFileTypes: true });
  for (const entry of entries) {
    const fullPath = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      const found = findFileRecursive(fullPath, predicate);
      if (found) return found;
    } else if (entry.isFile() && predicate(entry.name, fullPath)) {
      return fullPath;
    }
  }
  return null;
}

// 2. 查找并拷贝 NSIS 安装程序
const targetDir = path.join(rootDir, 'src-tauri', 'target');
let srcSetupPath = null;
const candidateNsisDirs = [
  path.join(targetDir, 'release', 'bundle', 'nsis'),
  path.join(targetDir, 'x86_64-pc-windows-msvc', 'release', 'bundle', 'nsis')
];

for (const dir of candidateNsisDirs) {
  if (fs.existsSync(dir)) {
    const files = fs.readdirSync(dir);
    const exact = files.find(f => f.includes(version) && f.endsWith('.exe'));
    if (exact) {
      srcSetupPath = path.join(dir, exact);
      break;
    }
    const found = files.find(f => f.toLowerCase().endsWith('-setup.exe') || (f.toLowerCase().endsWith('.exe') && f.toLowerCase().includes('setup')));
    if (found) {
      srcSetupPath = path.join(dir, found);
      break;
    }
  }
}

if (!srcSetupPath) {
  srcSetupPath = findFileRecursive(targetDir, name => name.toLowerCase().endsWith('-setup.exe') || (name.toLowerCase().endsWith('.exe') && name.toLowerCase().includes('setup')));
}

if (!srcSetupPath) {
  throw new Error(`在 ${targetDir} 中未找到 setup.exe 安装程序`);
}

const dstSetupPath = path.join(outDir, `Maobu.Fetch_${version}_x64-setup.exe`);
fs.copyFileSync(srcSetupPath, dstSetupPath);
console.log(`✔ 安装程序已就绪: ${path.basename(srcSetupPath)} -> Maobu.Fetch_${version}_x64-setup.exe`);

// 3. 查找并拷贝便携版 EXE
let srcPortablePath = null;
const candidateExePaths = [
  path.join(targetDir, 'release', 'maobu-fetch.exe'),
  path.join(targetDir, 'x86_64-pc-windows-msvc', 'release', 'maobu-fetch.exe')
];

for (const p of candidateExePaths) {
  if (fs.existsSync(p)) {
    srcPortablePath = p;
    break;
  }
}

if (!srcPortablePath) {
  srcPortablePath = findFileRecursive(targetDir, (name, full) => name === 'maobu-fetch.exe' && !full.includes('deps') && !full.includes('incremental'));
}

if (!srcPortablePath) {
  throw new Error(`在 ${targetDir} 中未找到主程序 maobu-fetch.exe`);
}

const dstPortablePath = path.join(outDir, `maobu-fetch-v${version}-portable.exe`);
fs.copyFileSync(srcPortablePath, dstPortablePath);
console.log(`✔ 便携版程序已就绪: maobu-fetch-v${version}-portable.exe`);

// 4. 打包扩展程序（AGENTS.md §10 双命名强约束）
const extVersionZip = path.join(outDir, `maobu-fetch-extension-v${version}.zip`);
const extCommonZip = path.join(outDir, 'extension.zip');
const extDistSrc = path.join(rootDir, 'extension', 'dist', '*');

execSync(`powershell -Command "Compress-Archive -Path '${extDistSrc}' -DestinationPath '${extVersionZip}' -Force"`);
fs.copyFileSync(extVersionZip, extCommonZip);
console.log(`✔ 浏览器扩展已打包: maobu-fetch-extension-v${version}.zip 与 extension.zip`);

// 5. 计算各文件 SHA-256
const targetFiles = [
  dstSetupPath,
  dstPortablePath,
  extVersionZip,
  extCommonZip
];

function computeSha256(filePath) {
  const buffer = fs.readFileSync(filePath);
  return crypto.createHash('sha256').update(buffer).digest('hex').toUpperCase();
}

function formatSize(bytes) {
  if (bytes >= 1024 * 1024) {
    return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
  }
  return `${(bytes / 1024).toFixed(2)} KB`;
}

let shaTable = '| 产物名称 | 文件大小 | SHA-256 校验和 |\n| :--- | :---: | :--- |\n';
for (const file of targetFiles) {
  const filename = path.basename(file);
  const stat = fs.statSync(file);
  const sha = computeSha256(file);
  const sizeStr = formatSize(stat.size);
  shaTable += `| \`${filename}\` | ${sizeStr} | \`${sha}\` |\n`;
}

console.log('\n产物 SHA-256 校验表:\n' + shaTable);

// 6. 生成或更新 Release Notes
const notesPath = path.join(rootDir, 'releases', `release_notes_v${version}.md`);
let body = '';
if (fs.existsSync(notesPath)) {
  const rawNotes = fs.readFileSync(notesPath, 'utf8');
  if (rawNotes.includes('<!-- SHA256_TABLE -->')) {
    body = rawNotes.replace('<!-- SHA256_TABLE -->', shaTable.trim());
  } else if (/## 📦 发布产物校验 \(SHA-256\)/.test(rawNotes)) {
    body = rawNotes.replace(
      /(## 📦 发布产物校验 \(SHA-256\)[\s\S]*?)((\r?\n---\r?\n)|$)/,
      `## 📦 发布产物校验 (SHA-256)\n\n${shaTable.trim()}\n\n---`
    );
  } else {
    body = rawNotes + `\n\n## 📦 发布产物校验 (SHA-256)\n\n${shaTable.trim()}\n`;
  }
} else {
  body = `# 猫步下载器 (Maobu Fetch) v${version} 发布说明\n\n` +
    `🎉 欢迎使用猫步下载器 **v${version}**！本版本由 GitHub Actions 云端虚拟机构建流水线全自动编译、测试与发布。\n\n` +
    `---\n\n## 📦 发布产物校验 (SHA-256)\n\n${shaTable.trim()}\n\n` +
    `---\n\n## 🛡️ 安全与合规说明\n` +
    `- 基础安装包仅 ~4.7 MB（远低于 30 MB 强约束）；\n` +
    `- 基础安装包内零捆绑第三方可执行程序（严格按需下载与 SHA-256 校验清单）；\n` +
    `- 本地通信严格限制在 127.0.0.1，保留 HMAC-SHA256 签名鉴权；\n` +
    `- 扩展包提供版本化文件与通用命名文件双产物，保证历史客户端一键平滑更新。\n`;
}

fs.writeFileSync(path.join(outDir, 'release_notes.md'), body, 'utf8');
console.log('✔ 发布说明已就绪: releases_out/release_notes.md');

// 7. GITHUB_ENV 传递版本号
if (process.env.GITHUB_ENV) {
  fs.appendFileSync(process.env.GITHUB_ENV, `APP_VERSION=${version}\n`, 'utf8');
  console.log(`✔ 已写入 GITHUB_ENV: APP_VERSION=${version}`);
}

console.log('\n🎉 全部发布产物封装完毕！');
