import test from "node:test";
import assert from "node:assert/strict";
import { isLanDiskHost, validateLanDiskRequest } from "./landisk-security.js";

const sender = { tab: { id: 7, url: "http://192.168.1.2/files" }, frameId: 0, documentId: "doc-a", url: "http://192.168.1.2/files", origin: "http://192.168.1.2" };
const message = { url: "http://192.168.1.2/api/download/batch", fileName: "文档.zip", body: "files=report.txt&folderName=Documents" };
const settings = { intercept: true, blockHosts: [], allowHosts: [] };
const valid = () => validateLanDiskRequest(message, sender, settings);

test("LanDisk validates same-origin exact API, settings and source frame before asking for credentials", () => {
  assert.equal(valid().fileCount, 1);
  for (const url of ["http://127.0.0.1/api/download/batch", "http://192.168.1.2/api/delete", "file:///api/download/batch"]) {
    assert.throws(() => validateLanDiskRequest({ ...message, url }, sender, settings));
  }
  assert.throws(() => validateLanDiskRequest(message, { ...sender, frameId: 1 }, settings));
  assert.throws(() => validateLanDiskRequest(message, sender, { ...settings, intercept: false }));
  assert.throws(() => validateLanDiskRequest(message, sender, { ...settings, bypassUntil: Date.now() + 10000 }));
  assert.throws(() => validateLanDiskRequest(message, sender, settings, { enabled: false }));
  assert.throws(() => validateLanDiskRequest({ ...message, body: "files=x&operation=delete" }, sender, settings));
});

test("LanDisk public hosts need explicit user allow-list and private IPv6 is precisely identified", () => {
  assert.equal(isLanDiskHost("[fd00::1]"), true);
  assert.equal(isLanDiskHost("[2001:4860:4860::8888]"), false);
  assert.equal(isLanDiskHost("999.1.1.1"), false);
  const publicSender = { ...sender, tab: { id: 7, url: "https://files.example/" }, url: "https://files.example/", origin: "https://files.example" };
  const request = { ...message, url: "https://files.example/api/download/batch" };
  assert.throws(() => validateLanDiskRequest(request, publicSender, settings));
  assert.equal(validateLanDiskRequest(request, publicSender, { ...settings, allowHosts: ["files.example"] }).fileCount, 1);
});
