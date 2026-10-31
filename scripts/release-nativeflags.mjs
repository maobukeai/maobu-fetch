import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import { spawnSync } from "node:child_process";

export function nativePathFlags(environment, root, home = os.homedir()) {
  const prefixes = [[path.resolve(home), "/build-user/"],
    [path.resolve(environment.CARGO_HOME || path.join(home, ".cargo")), "/cargo/"],
    [path.resolve(root), "/maobu-fetch/"]];
  // One home mapping covers nested paths and permits spaces/Chinese source names below it.
  const selected = prefixes.filter(([source], index) => index === 0 || !source.toLowerCase().startsWith(`${prefixes[0][0].toLowerCase()}${path.sep}`));
  return ["/experimental:deterministic", ...selected.map(([source, target]) =>
    `/pathmap:${source.replaceAll("/", "\\").replace(/[\\/]+$/, "")}\\=${target}`)];
}

export function releaseNativeEnvironment(environment, root, target = "x86_64-pc-windows-msvc") {
  if (process.platform !== "win32") return { ...environment };
  if (!target.endsWith("-pc-windows-msvc")) throw new Error("发布路径验收当前需要 Windows MSVC 目标");
  const flags = nativePathFlags(environment, root);
  const escaped = /^(1|true|yes)$/i.test(environment.CC_SHELL_ESCAPED_FLAGS ?? "");
  if (!escaped && flags.some(flag => /\s/.test(flag))) {
    throw new Error("原生发布映射前缀含空格；请先明确启用并验证 CC_SHELL_ESCAPED_FLAGS，未启动不安全构建");
  }
  const encoded = flags.map(flag => escaped ? JSON.stringify(flag) : flag).join(" ");
  const result = { ...environment };
  // cc merges generic/host/target flags; append at the most specific priority without rewriting old values.
  for (const kind of ["CFLAGS", "CXXFLAGS"]) {
    const key = `${kind}_${target}`;
    result[key] = [environment[key], encoded].filter(Boolean).join(" ");
  }
  return result;
}

function compilerPath(environment, target) {
  const normalized = target.replaceAll("-", "_");
  const configured = environment[`AWS_LC_SYS_CC_${normalized}`] || environment.AWS_LC_SYS_CC
    || environment[`CC_${target}`] || environment[`CC_${normalized}`] || environment.TARGET_CC || environment.CC;
  if (configured) {
    const executable = configured.replace(/^"|"$/g, "");
    if (!fs.existsSync(executable) || path.basename(executable).toLowerCase() !== "cl.exe") throw new Error("需对实际 CC 单独完成原生路径映射验收；未切换或忽略自定义编译器");
    return executable;
  }
  const vswhere = path.join(environment["ProgramFiles(x86)"] || "C:/Program Files (x86)", "Microsoft Visual Studio/Installer/vswhere.exe");
  const discovery = spawnSync(vswhere, ["-latest", "-products", "*", "-requires", "Microsoft.VisualStudio.Component.VC.Tools.x86.x64", "-property", "installationPath"], { encoding: "utf8", windowsHide: true });
  if (discovery.status !== 0 || !discovery.stdout.trim()) throw new Error("找不到可验证的 MSVC，未开始发布构建");
  const installation = discovery.stdout.trim();
  const version = fs.readFileSync(path.join(installation, "VC/Auxiliary/Build/Microsoft.VCToolsVersion.default.txt"), "utf8").trim();
  const architecture = target.startsWith("aarch64") ? "arm64" : target.startsWith("i686") ? "x86" : "x64";
  const compiler = path.join(installation, "VC/Tools/MSVC", version, "bin/Hostx64", architecture, "cl.exe");
  if (!fs.existsSync(compiler)) throw new Error("当前目标的 MSVC 编译器不存在，未开始发布构建");
  return compiler;
}

function execute(executable, args) {
  const result = spawnSync(executable, args, { encoding: "utf8", windowsHide: true });
  const output = `${result.stdout ?? ""}\n${result.stderr ?? ""}`;
  if (result.status !== 0 || /D9002|D9007|unknown (?:argument|option)|unrecognized/i.test(output)) {
    throw new Error(`原生路径映射预检失败，编译选项未获证实：${output}`);
  }
}

function compileFixture(compiler, folder, source, flags, name) {
  const object = path.join(folder, `${name}.obj`);
  const executable = path.join(folder, `${name}.exe`);
  execute(compiler, ["/nologo", "/c", "/O1", "/W4", "/WX", "/utf-8", "/Zl", `/I${path.dirname(source)}`, `/Fo${object}`, ...flags, source]);
  execute(path.join(path.dirname(compiler), "link.exe"), ["/NOLOGO", "/NODEFAULTLIB", "/SUBSYSTEM:CONSOLE", "/ENTRY:mainCRTStartup", "/DEBUG:NONE", "/OPT:REF", `/OUT:${executable}`, object]);
  return fs.readFileSync(executable);
}

export function verifyNativeReleaseMapping(environment, root, target = "x86_64-pc-windows-msvc") {
  if (process.platform !== "win32") return { skipped: "MSVC-only preflight" };
  const compiler = compilerPath(environment, target);
  for (const [name,value] of Object.entries(environment)) {
    if (value && /^(?:AWS_LC_SYS_)?(?:(?:HOST|TARGET)_)?C(?:C|XX)(?:_[A-Za-z0-9_-]+)?$/.test(name)
      && path.resolve(value.replace(/^"|"$/g, "")).toLowerCase() !== path.resolve(compiler).toLowerCase()) {
      throw new Error("发布设置含不同的 CC/CXX 编译器；需分别验收其原生路径映射，未静默切换编译器");
    }
  }
  const folder = fs.mkdtempSync(path.join(os.tmpdir(), "maobu-native-remap-"));
  try {
    const sources = path.join(folder, "中文 source");
    fs.mkdirSync(sources);
    const header = `#define W2(x) L##x\n#define W(x) W2(x)\n#ifdef __cplusplus\n#define Wide wchar_t\n#else\n#define Wide unsigned short\n#endif\n__declspec(dllexport) extern const char header_narrow[] = __FILE__;\n__declspec(dllexport) extern const Wide header_wide[] = W(__FILE__);\n`;
    fs.writeFileSync(path.join(sources, "probe.h"), header);
    for (const language of ["c", "cpp"]) {
      const source = path.join(sources, `路径 fixture.${language}`);
      fs.writeFileSync(source, `#ifdef __cplusplus\nextern "C" {\n#endif\n#include "probe.h"\n__declspec(dllexport) extern const char source_narrow[] = __FILE__;\n__declspec(dllexport) extern const Wide source_wide[] = W(__FILE__);\nvoid mainCRTStartup(void) {}\n#ifdef __cplusplus\n}\n#endif\n`);
      const before = compileFixture(compiler, folder, source, [], `${language}-control`);
      const after = compileFixture(compiler, folder, source, nativePathFlags(environment, root), `${language}-mapped`);
      for (const encoding of ["utf8", "utf16le"]) {
        const sourceBytes = Buffer.from(source, encoding);
        assert.ok(before.includes(sourceBytes), `${language}/${encoding}: control must contain an actual __FILE__ source path`);
        assert.ok(!after.includes(sourceBytes), `${language}/${encoding}: absolute source path survived mapping`);
        assert.ok(!after.includes(Buffer.from(os.homedir(), encoding)), `${language}/${encoding}: home path survived mapping`);
        assert.ok(after.includes(Buffer.from("/build-user/", encoding)) || after.includes(Buffer.from("/maobu-fetch/", encoding)), `${language}/${encoding}: mapping was silently ignored`);
      }
    }
    return { compiler, languages: ["C", "C++"], encodings: ["UTF-8", "UTF-16LE"], passed: true };
  } finally {
    assert.equal(path.dirname(path.resolve(folder)), path.resolve(os.tmpdir()));
    fs.rmSync(folder, { recursive: true, force: true });
  }
}
