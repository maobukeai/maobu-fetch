import test from "node:test";
import assert from "node:assert/strict";
import vm from "node:vm";
import { readFileSync } from "node:fs";
import { isLanDiskHost as isTailscaleOrLanHost } from "./landisk-security.js";

// 直接运行经典脚本中的原始函数；避免测试重写同一份产品逻辑。
const productSource = readFileSync(new URL("landisk-adapter.js", import.meta.url), "utf8");
const functionStart = productSource.indexOf("  function buildBatchRequestPayload(");
const functionEnd = productSource.indexOf("  // ── 1.5", functionStart);
assert.ok(functionStart >= 0 && functionEnd > functionStart);
function buildActualRequest(filesArr, folderName, pin) {
  const build = vm.runInNewContext(`(${productSource.slice(functionStart, functionEnd).trim()})`, {
    URL, URLSearchParams, window: { location: { href: "http://192.168.1.2/files" } },
    getPin: () => pin, getAuthQuery: () => "",
  });
  return build(filesArr, folderName, null);
}

function buildBatchParams(filesArr, folderName, pin) {
  return buildActualRequest(filesArr, folderName, pin).body;
}

function resolveFolderName(filesArr, customFolderName) {
  return new URLSearchParams(buildActualRequest(filesArr, customFolderName, "").body).get("folderName");
}

test("LanDisk: Tailscale 与本地局域网主机名识别", () => {
  assert.equal(isTailscaleOrLanHost("100.100.1.5"), true);
  assert.equal(isTailscaleOrLanHost("100.64.0.1"), true);
  assert.equal(isTailscaleOrLanHost("100.127.255.254"), true);
  assert.equal(isTailscaleOrLanHost("my-server.ts.net"), true);
  assert.equal(isTailscaleOrLanHost("node.tailscale.net"), true);
  assert.equal(isTailscaleOrLanHost("192.168.1.100"), true);
  assert.equal(isTailscaleOrLanHost("10.0.0.1"), true);
  assert.equal(isTailscaleOrLanHost("172.16.1.1"), true);
  assert.equal(isTailscaleOrLanHost("localhost"), true);
  assert.equal(isTailscaleOrLanHost("127.0.0.1"), true);

  // 公网 IP
  assert.equal(isTailscaleOrLanHost("8.8.8.8"), false);
  assert.equal(isTailscaleOrLanHost("1.1.1.1"), false);
  assert.equal(isTailscaleOrLanHost("example.com"), false);
});

test("LanDisk: 打包下载表单参数序列化正确", () => {
  const files = ["D:\\Photos\\IMG_001.JPG", "D:\\Photos\\IMG_002.JPG"];
  const folder = resolveFolderName(files, "");
  assert.equal(folder, "batch_download_2_items");

  const body = buildBatchParams(files, folder, "123456");
  const parsed = new URLSearchParams(body);
  assert.equal(parsed.get("folderName"), "batch_download_2_items");
  assert.deepEqual(parsed.getAll("files"), files);
  assert.equal(parsed.get("pin"), "123456");
});

test("LanDisk: 单目录打包智能提炼文件名", () => {
  const singleDir = ["/home/user/workspace/project-alpha"];
  const folder = resolveFolderName(singleDir, "");
  assert.equal(folder, "project-alpha");

  const body = buildBatchParams(singleDir, folder, "");
  const parsed = new URLSearchParams(body);
  assert.equal(parsed.get("folderName"), "project-alpha");
  assert.deepEqual(parsed.getAll("files"), ["/home/user/workspace/project-alpha"]);
  assert.equal(parsed.has("pin"), false);
});
