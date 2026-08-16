import { cp, mkdir, readdir, rm } from "node:fs/promises";
await rm("dist", { recursive: true, force: true });
await mkdir("dist/src", { recursive: true });
// 逐项拷贝源码，排除 *.test.js：单元测试不随扩展分发。
for (const entry of await readdir("src", { withFileTypes: true })) {
  if (entry.isFile() && entry.name.endsWith(".test.js")) continue;
  await cp(`src/${entry.name}`, `dist/src/${entry.name}`, { recursive: true });
}
await cp("manifest.json", "dist/manifest.json");
