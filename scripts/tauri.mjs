#!/usr/bin/env node
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { releaseRustFlags } from "./release-rustflags.mjs";
import { releaseNativeEnvironment, verifyNativeReleaseMapping } from "./release-nativeflags.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const environment = { ...process.env };
if (args[0] === "build" && !args.includes("--debug")) {
  // 编译器参数用 Cargo 的数组编码，中文目录和空格不会被拆成其它参数。
  environment.CARGO_ENCODED_RUSTFLAGS = releaseRustFlags(environment, root).join("\x1f");
  delete environment.RUSTFLAGS;
  const targetAt=args.indexOf("--target");
  const target=targetAt>=0 ? args[targetAt+1] : args.find(arg=>arg.startsWith("--target="))?.slice(9) || environment.CARGO_BUILD_TARGET || (process.arch==="arm64"?"aarch64-pc-windows-msvc":"x86_64-pc-windows-msvc");
  const native=verifyNativeReleaseMapping(environment,root,target);
  Object.assign(environment,releaseNativeEnvironment(environment,root,target));
  if(native.passed) console.log("原生 C/C++ 发布路径映射预检通过（UTF-8 / UTF-16LE）");
}
const result = spawnSync(process.execPath, [path.join(root,"node_modules/@tauri-apps/cli/tauri.js"), ...args], {
  cwd:root, env:environment, stdio:"inherit", windowsHide:true,
});
if (result.error) {
  console.error(`无法启动 Tauri：${result.error.message}`);
  process.exit(1);
}
process.exit(result.status ?? 1);
