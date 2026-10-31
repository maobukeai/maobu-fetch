import { cp, mkdir, readdir, rm } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
const extensionRoot = path.resolve(fileURLToPath(new URL(".", import.meta.url)));
const outputRoot = path.resolve(extensionRoot, "dist");
if (path.dirname(outputRoot) !== extensionRoot || path.basename(outputRoot) !== "dist") {
  throw new Error("扩展构建目录不安全");
}
await rm(outputRoot, { recursive: true, force: true });
await mkdir(path.join(outputRoot, "src"), { recursive: true });
// 逐项拷贝源码，排除 *.test.js：单元测试不随扩展分发。
for (const entry of await readdir(path.join(extensionRoot, "src"), { withFileTypes: true })) {
  if (entry.isFile() && entry.name.endsWith(".test.js")) continue;
  await cp(path.join(extensionRoot, "src", entry.name), path.join(outputRoot, "src", entry.name), { recursive: true });
}
await cp(path.join(extensionRoot, "manifest.json"), path.join(outputRoot, "manifest.json"));
