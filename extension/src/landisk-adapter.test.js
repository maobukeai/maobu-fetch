import test from "node:test";
import assert from "node:assert/strict";

function isTailscaleOrLanHost(hostname = "") {
  const h = String(hostname || "").toLowerCase();
  if (h === "localhost" || h === "127.0.0.1" || h.endsWith(".local") || h.endsWith(".lan")) return true;
  if (h.endsWith(".ts.net") || h.endsWith(".tailscale.net")) return true;
  const m = h.match(/^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/);
  if (m) {
    const b0 = parseInt(m[1], 10);
    const b1 = parseInt(m[2], 10);
    if (b0 === 100 && b1 >= 64 && b1 <= 127) return true; // Tailscale CGNAT
    if (b0 === 10 || b0 === 127 || (b0 === 172 && b1 >= 16 && b1 <= 31) || (b0 === 192 && b1 === 168)) return true; // LAN
  }
  return false;
}

function buildBatchParams(filesArr, folderName, pin) {
  const bodyParams = new URLSearchParams();
  bodyParams.append("folderName", folderName);
  filesArr.forEach((f) => bodyParams.append("files", f));
  if (pin) bodyParams.append("pin", pin);
  return bodyParams.toString();
}

function resolveFolderName(filesArr, customFolderName) {
  if (customFolderName && customFolderName !== "batch_download") {
    return customFolderName;
  }
  if (filesArr.length === 1) {
    return filesArr[0].split(/[\\/]/).filter(Boolean).pop() || "batch_download";
  }
  return `batch_download_${filesArr.length}_items`;
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
