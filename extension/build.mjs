import { cp, mkdir, readdir, rm, stat } from "node:fs/promises";
import path from "node:path";
await rm("dist", { recursive: true, force: true });
await mkdir("dist/src", { recursive: true });
// 逐项拷贝源码，排除 *.test.js：单元测试不随扩展分发。
for (const entry of await readdir("src", { withFileTypes: true })) {
  if (entry.isFile() && entry.name.endsWith(".test.js")) continue;
  await cp(`src/${entry.name}`, `dist/src/${entry.name}`, { recursive: true });
}
await cp("manifest.json", "dist/manifest.json");

const appData = process.env.APPDATA;
if (appData) {
  const installedExtDir = path.join(appData, "app.lumaget.desktop", "extension");
  try {
    const info = await stat(installedExtDir);
    if (info.isDirectory()) {
      await rm(path.join(installedExtDir, "src", "landisk-injected.js"), { force: true });
      await cp("dist/manifest.json", path.join(installedExtDir, "manifest.json"), { force: true });
      await cp("dist/src", path.join(installedExtDir, "src"), { recursive: true, force: true });
    }
  } catch {
    // 忽略未安装目录
  }
}
