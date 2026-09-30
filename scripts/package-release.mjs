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

// 1.5 版本一致性门禁（AGENTS.md §10）：五处版本必须与目标版本完全一致，
// 防止把旧版本二进制重命名成新版本发布。
function readJsonVersion(file) {
  return JSON.parse(fs.readFileSync(file, 'utf8')).version;
}
function readCargoVersion(file) {
  const m = fs.readFileSync(file, 'utf8').match(/^version\s*=\s*"([^"]+)"/m);
  if (!m) throw new Error(`无法从 ${file} 读取 version 字段`);
  return m[1];
}
function readManifestVersion(file) {
  const v = readJsonVersion(file);
  // Chrome/Edge MV3 manifest.version 仅允许点分隔的非负整数，不接受预发布后缀
  if (!/^\d+(\.\d+){0,3}$/.test(v)) {
    throw new Error(`${file} 的 version "${v}" 不是 Chrome/Edge 扩展允许的纯数字版本`);
  }
  return v;
}

const versionSources = {
  'package.json': readJsonVersion(path.join(rootDir, 'package.json')),
  'src-tauri/Cargo.toml': readCargoVersion(path.join(rootDir, 'src-tauri', 'Cargo.toml')),
  'src-tauri/tauri.conf.json': readJsonVersion(path.join(rootDir, 'src-tauri', 'tauri.conf.json')),
  'extension/package.json': readJsonVersion(path.join(rootDir, 'extension', 'package.json')),
  'extension/manifest.json': readManifestVersion(path.join(rootDir, 'extension', 'manifest.json')),
};
const mismatched = Object.entries(versionSources).filter(([, v]) => v !== version);
if (mismatched.length > 0) {
  throw new Error(
    `版本不一致，拒绝发布：目标 v${version}，但 ` +
      mismatched.map(([f, v]) => `${f}=${v}`).join('、') +
      '。请先运行 node scripts/bump-version.mjs 同步全部版本号。'
  );
}
const refName = process.env.GITHUB_REF_NAME;
if (refName && /^v\d/.test(refName) && refName.replace(/^v/, '') !== version) {
  throw new Error(`Git 标签 ${refName} 与目标版本 v${version} 不一致，拒绝发布`);
}
console.log(`✔ 版本一致性校验通过：五处版本均为 ${version}`);

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
    // 门禁：只接受文件名含精确版本号的产物，禁止"任意 setup.exe 回退"，
    // 防止把旧版本安装包重命名成新版本发布。
    const exact = files.find(f => f.includes(version) && f.endsWith('.exe'));
    if (exact) {
      srcSetupPath = path.join(dir, exact);
      break;
    }
  }
}

if (!srcSetupPath) {
  throw new Error(
    `未找到文件名包含 ${version} 的 NSIS 安装程序（候选目录：${candidateNsisDirs.join(', ')}）。` +
      '禁止回退选择其他版本产物，请确认构建版本与发布版本一致后重试。'
  );
}

const dstSetupPath = path.join(outDir, `Maobu.Fetch_${version}_x64-setup.exe`);
fs.copyFileSync(srcSetupPath, dstSetupPath);
console.log(`✔ 安装程序已就绪: ${path.basename(srcSetupPath)} -> Maobu.Fetch_${version}_x64-setup.exe`);

// 3. 查找并拷贝 MSI 安装程序
let srcMsiPath = null;
const candidateMsiDirs = [
  path.join(targetDir, 'release', 'bundle', 'msi'),
  path.join(targetDir, 'x86_64-pc-windows-msvc', 'release', 'bundle', 'msi')
];

for (const dir of candidateMsiDirs) {
  if (fs.existsSync(dir)) {
    const files = fs.readdirSync(dir);
    // 门禁：MSI 同样只接受精确版本匹配，禁止任意回退（同 NSIS）。
    const exact = files.find(f => f.includes(version) && f.endsWith('.msi'));
    if (exact) {
      srcMsiPath = path.join(dir, exact);
      break;
    }
  }
}

if (!srcMsiPath) {
  throw new Error(
    `未找到文件名包含 ${version} 的 MSI 安装包（候选目录：${candidateMsiDirs.join(', ')}）。` +
      '禁止回退选择其他版本产物，请确认构建版本与发布版本一致后重试。'
  );
}

const dstMsiPath = path.join(outDir, `Maobu.Fetch_${version}_x64.msi`);
fs.copyFileSync(srcMsiPath, dstMsiPath);
console.log(`✔ MSI 安装程序已就绪: ${path.basename(srcMsiPath)} -> Maobu.Fetch_${version}_x64.msi`);

// 4. 查找并拷贝便携版 EXE
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

// 5. 打包扩展程序（AGENTS.md §10 双命名强约束）
const extVersionZip = path.join(outDir, `maobu-fetch-extension-v${version}.zip`);
const extCommonZip = path.join(outDir, 'extension.zip');
const extDistSrc = path.join(rootDir, 'extension', 'dist', '*');

execSync(`powershell -Command "Compress-Archive -Path '${extDistSrc}' -DestinationPath '${extVersionZip}' -Force"`);
fs.copyFileSync(extVersionZip, extCommonZip);
console.log(`✔ 浏览器扩展已打包: maobu-fetch-extension-v${version}.zip 与 extension.zip`);

// 5.5 发布硬门禁（AGENTS.md §6/§10）：未验证通过不得继续封装发布资产。
const MAX_INSTALLER_BYTES = 30 * 1024 * 1024;
for (const [label, file] of [
  ['NSIS 安装包', dstSetupPath],
  ['MSI 安装包', dstMsiPath],
]) {
  const size = fs.statSync(file).size;
  if (size >= MAX_INSTALLER_BYTES) {
    throw new Error(
      `${label} ${(size / 1024 / 1024).toFixed(2)} MB 达到/超过 30 MB 强约束，拒绝发布`
    );
  }
}
console.log('✔ 体积门禁通过：NSIS/MSI 均低于 30 MB');

function assertExeMetadata(exePath, expectedVersion) {
  const out = execSync(
    `powershell -NoProfile -Command "(Get-Item -LiteralPath '${exePath}').VersionInfo | Format-List ProductVersion,ProductName,FileDescription,OriginalFilename | Out-String"`,
    { encoding: 'utf8' }
  );
  const pick = name => {
    const m = out.match(new RegExp(`${name}\\s*:\\s*(.*)`));
    return m ? m[1].trim() : '';
  };
  const productVersion = pick('ProductVersion');
  const productName = pick('ProductName');
  const fileDescription = pick('FileDescription');
  const originalFilename = pick('OriginalFilename');
  if (!productVersion.includes(expectedVersion)) {
    throw new Error(
      `EXE 的 ProductVersion "${productVersion}" 不含目标版本 ${expectedVersion}，Windows 版本资源可能损坏或版本不一致，拒绝发布`
    );
  }
  if (!productName.toLowerCase().includes('maobu') && !fileDescription.toLowerCase().includes('maobu')) {
    throw new Error(
      `EXE 的 ProductName "${productName}" 异常，Windows 版本资源可能损坏，拒绝发布`
    );
  }
  if (originalFilename && !originalFilename.toLowerCase().endsWith('.exe')) {
    throw new Error(
      `EXE 的 OriginalFilename "${originalFilename}" 异常，Windows 版本资源可能损坏，拒绝发布`
    );
  }
}
assertExeMetadata(dstPortablePath, version);
console.log('✔ EXE 版本资源门禁通过（ProductVersion / ProductName / OriginalFilename）');

// 6. 计算各文件 SHA-256
const targetFiles = [
  dstSetupPath,
  dstMsiPath,
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

// 7. 生成或更新 Release Notes
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
    `- 基础安装包（EXE 及 MSI）均远低于 30 MB 强约束；\n` +
    `- 基础安装包内零捆绑第三方可执行程序（严格按需下载与 SHA-256 校验清单）；\n` +
    `- 本地通信严格限制在 127.0.0.1，保留 HMAC-SHA256 签名鉴权；\n` +
    `- 扩展包提供版本化文件与通用命名文件双产物，保证历史客户端一键平滑更新。\n`;
}

// §10 门禁：发布说明叙述正文严禁出现形如 *.exe / *.zip 的文件名，
// 防止 Atom Feed 降级解析器把示例名称误识别为资产包；资产名只允许出现在校验表格中。
const narrative = body
  .split(/\r?\n/)
  .filter(line => !line.trim().startsWith('|'))
  .join('\n');
const interfering = narrative.match(/[A-Za-z0-9._-]+\.(exe|zip)\b/gi);
if (interfering) {
  throw new Error(
    `发布说明正文出现干扰性文件名 ${[...new Set(interfering)].join('、')}，` +
      '请改用非文件名表述（资产名仅允许出现在校验表格中）'
  );
}

fs.writeFileSync(path.join(outDir, 'release_notes.md'), body, 'utf8');
console.log('✔ 发布说明已就绪: releases_out/release_notes.md');

// 8. GITHUB_ENV 传递版本号
if (process.env.GITHUB_ENV) {
  fs.appendFileSync(process.env.GITHUB_ENV, `APP_VERSION=${version}\n`, 'utf8');
  console.log(`✔ 已写入 GITHUB_ENV: APP_VERSION=${version}`);
}

console.log('\n🎉 全部发布产物封装完毕！');
