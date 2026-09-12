#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const rootDir = path.resolve(__dirname, '..');

const newVersion = process.argv[2]?.trim().replace(/^v/, '');
if (!newVersion || !/^\d+\.\d+\.\d+(-[a-zA-Z0-9.]+)?$/.test(newVersion)) {
  console.error('用法: node scripts/bump-version.mjs <新版本号>');
  console.error('示例: node scripts/bump-version.mjs 0.9.6');
  process.exit(1);
}

console.log(`🚀 准备将全项目版本号同步升级为: ${newVersion}`);

// 1. package.json
const pkgPath = path.join(rootDir, 'package.json');
const pkg = JSON.parse(fs.readFileSync(pkgPath, 'utf8'));
const oldVersion = pkg.version;
pkg.version = newVersion;
fs.writeFileSync(pkgPath, JSON.stringify(pkg, null, 2) + '\n', 'utf8');
console.log(`✔ [1/6] package.json: ${oldVersion} -> ${newVersion}`);

// 2. src-tauri/Cargo.toml
const cargoPath = path.join(rootDir, 'src-tauri', 'Cargo.toml');
let cargoContent = fs.readFileSync(cargoPath, 'utf8');
cargoContent = cargoContent.replace(/^version\s*=\s*"[^"]+"/m, `version = "${newVersion}"`);
fs.writeFileSync(cargoPath, cargoContent, 'utf8');
console.log(`✔ [2/6] src-tauri/Cargo.toml: -> ${newVersion}`);

// 3. src-tauri/tauri.conf.json
const tauriConfPath = path.join(rootDir, 'src-tauri', 'tauri.conf.json');
const tauriConf = JSON.parse(fs.readFileSync(tauriConfPath, 'utf8'));
tauriConf.version = newVersion;
fs.writeFileSync(tauriConfPath, JSON.stringify(tauriConf, null, 2) + '\n', 'utf8');
console.log(`✔ [3/6] src-tauri/tauri.conf.json: -> ${newVersion}`);

// 4. extension/package.json
const extPkgPath = path.join(rootDir, 'extension', 'package.json');
const extPkg = JSON.parse(fs.readFileSync(extPkgPath, 'utf8'));
extPkg.version = newVersion;
fs.writeFileSync(extPkgPath, JSON.stringify(extPkg, null, 2) + '\n', 'utf8');
console.log(`✔ [4/6] extension/package.json: -> ${newVersion}`);

// 5. extension/manifest.json
const extManifestPath = path.join(rootDir, 'extension', 'manifest.json');
const extManifest = JSON.parse(fs.readFileSync(extManifestPath, 'utf8'));
extManifest.version = newVersion;
fs.writeFileSync(extManifestPath, JSON.stringify(extManifest, null, 2) + '\n', 'utf8');
console.log(`✔ [5/6] extension/manifest.json: -> ${newVersion}`);

// 6. src/components/settings/SettingsPage.tsx
const settingsPath = path.join(rootDir, 'src', 'components', 'settings', 'SettingsPage.tsx');
if (fs.existsSync(settingsPath)) {
  let settingsContent = fs.readFileSync(settingsPath, 'utf8');
  settingsContent = settingsContent.replace(
    /app_version:\s*appInfo\?\.version\s*\|\|\s*"[^"]+"/g,
    `app_version: appInfo?.version || "${newVersion}"`
  );
  settingsContent = settingsContent.replace(
    /如\s*\d+\.\d+\.\d+，可在浏览器扩展管理页查看/g,
    `如 ${newVersion}，可在浏览器扩展管理页查看`
  );
  fs.writeFileSync(settingsPath, settingsContent, 'utf8');
  console.log(`✔ [6/6] SettingsPage.tsx: -> ${newVersion}`);
}

console.log(`\n🎉 全项目 6 处版本号已全部同步更新为 v${newVersion}！`);
