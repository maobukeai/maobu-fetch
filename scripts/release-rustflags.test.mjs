import assert from "node:assert/strict";
import { mkdtemp, mkdir, writeFile, rm } from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import { spawnSync } from "node:child_process";
import { releaseRustFlags } from "./release-rustflags.mjs";
import { releaseNativeEnvironment,verifyNativeReleaseMapping } from "./release-nativeflags.mjs";

const flags = releaseRustFlags({ RUSTFLAGS:"-C opt-level=1" }, process.cwd());
assert.deepEqual(flags.slice(0,2), ["-C","opt-level=1"]);
assert.equal(releaseRustFlags({CARGO_ENCODED_RUSTFLAGS:"-C\x1fdebuginfo=0",RUSTFLAGS:"ignored"},process.cwd())[1],"debuginfo=0");
if(process.platform==="win32") {
  const environment=releaseNativeEnvironment({CFLAGS:"/DKEEP=1","CFLAGS_x86_64-pc-windows-msvc":"/DKEEP_TARGET=1"},process.cwd());
  assert.equal(environment.CFLAGS,"/DKEEP=1");
  assert.ok(environment["CFLAGS_x86_64-pc-windows-msvc"].startsWith("/DKEEP_TARGET=1 "));
  assert.ok(verifyNativeReleaseMapping(process.env,process.cwd()).passed);
  console.log("Release native-path mapping: actual MSVC C/C++ UTF-8/UTF-16 fixture passed");
}
const root = await mkdtemp(path.join(os.tmpdir(),"maobu-remap-"));
try {
  const folder = path.join(root,"中文 source");
  await mkdir(folder);
  const source = path.join(folder,"main.rs");
  const executable = path.join(root,process.platform === "win32" ? "fixture.exe" : "fixture");
  await writeFile(source,'fn main() { println!("{}", file!()); }');
  const result = spawnSync("rustc",[...releaseRustFlags({},root),source,"-o",executable],{encoding:"utf8",windowsHide:true});
  assert.equal(result.status,0,result.stderr);
  const output = spawnSync(executable,[],{encoding:"utf8",windowsHide:true});
  assert.equal(output.status,0,output.stderr);
  assert.ok(output.stdout.trim().replaceAll("\\","/").startsWith("/maobu-fetch/中文 source/"),output.stdout);
  assert.ok(!output.stdout.includes(os.homedir()));
  console.log("Release source-path remapping: actual rustc fixture passed");
} finally {
  // 仅删除本测试创建并校验为 TEMP 直属的唯一目录。
  assert.equal(path.dirname(path.resolve(root)),path.resolve(os.tmpdir()));
  await rm(root,{recursive:true,force:true});
}
