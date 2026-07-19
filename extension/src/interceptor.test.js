import test from "node:test";
import assert from "node:assert/strict";
import { evaluateDownload, interceptBrowserDownload, refreshDownload } from "./interceptor.js";

globalThis.chrome = {
  storage: {
    local: {
      set: async () => {},
      get: async () => ({}),
    }
  }
};

const settings = {
  intercept: true, minSizeMb: 1, allowHosts: [], blockHosts: [], extensions: [], bypassUntil: 0,
};

test("uses the final GitHub redirect URL while matching the original host", () => {
  const item = {
    id: 1,
    url: "https://github.com/example/project/archive/refs/tags/v1.0.0.zip",
    finalUrl: "https://codeload.github.com/example/project/zip/refs/tags/v1.0.0",
    filename: "project-1.0.0.zip",
    totalBytes: 4 * 1024 * 1024,
  };
  const result = evaluateDownload(item, { ...settings, allowHosts: ["github.com"] }, "extension-id");
  assert.equal(result.eligible, true);
  assert.equal(result.url, item.finalUrl);
  assert.equal(result.fileName, "project-1.0.0.zip");
});

test("blocks a redirect when either original or final host is blocked", () => {
  const result = evaluateDownload({
    id: 2, url: "https://github.com/a/b.zip", finalUrl: "https://objects.githubusercontent.com/file", filename: "b.zip", totalBytes: 3_000_000,
  }, { ...settings, blockHosts: ["objects.githubusercontent.com"] }, "extension-id");
  assert.equal(result.eligible, false);
  assert.equal(result.reason, "blocked-host");
});

test("applies file-type rules without intercepting unknown extensions", () => {
  const allowed = evaluateDownload({
    id: 20, url: "https://example.com/archive.zip", filename: "archive.zip", totalBytes: 3_000_000,
  }, { ...settings, extensions: ["zip", "7z"] }, "extension-id");
  const blocked = evaluateDownload({
    id: 21, url: "https://example.com/readme.pdf", filename: "readme.pdf", totalBytes: 3_000_000,
  }, { ...settings, extensions: ["zip", "7z"] }, "extension-id");
  assert.equal(allowed.eligible, true);
  assert.equal(blocked.eligible, false);
  assert.equal(blocked.reason, "extension");
});

test("refreshes a download until the redirected URL and filename stabilize", async () => {
  const snapshots = [
    { id: 3, url: "https://github.com/a/b.zip", finalUrl: "https://codeload.github.com/a/b", filename: "" },
    { id: 3, url: "https://github.com/a/b.zip", finalUrl: "https://codeload.github.com/a/b", filename: "b.zip" },
  ];
  const downloads = { search: async () => [snapshots.shift()] };
  const result = await refreshDownload(downloads, { id: 3, url: "https://github.com/a/b.zip", finalUrl: "", filename: "" }, async () => {});
  assert.equal(result.finalUrl, "https://codeload.github.com/a/b");
  assert.equal(result.filename, "b.zip");
});

test("pauses first, sends the stable final URL, then cancels browser download", async () => {
  const calls = [];
  const fresh = { id: 4, url: "https://github.com/a/b.zip", finalUrl: "https://codeload.github.com/a/b", filename: "b.zip", totalBytes: 3_000_000, referrer: "https://github.com/a/b/releases" };
  const downloads = {
    pause: async () => calls.push("pause"), search: async () => [fresh],
    cancel: async () => calls.push("cancel"), erase: async () => calls.push("erase"), resume: async () => calls.push("resume"),
  };
  const sent = [];
  const handled = await interceptBrowserDownload(fresh, {
    downloads, settings, runtimeId: "extension-id", wait: async () => {},
    sendTask: async (...args) => { calls.push("send"); sent.push(args); },
  });
  assert.equal(handled, true);
  assert.deepEqual(calls, ["pause", "send", "cancel", "erase"]);
  assert.equal(sent[0][0], fresh.finalUrl);
  assert.deepEqual(sent[0][2].headers, { Referer: fresh.referrer });
});

test("resumes browser download when the desktop bridge fails", async () => {
  const calls = [];
  const item = { id: 5, url: "https://example.com/file.zip", finalUrl: "https://example.com/file.zip", filename: "file.zip", totalBytes: 3_000_000 };
  const downloads = {
    pause: async () => calls.push("pause"), search: async () => [item],
    resume: async () => calls.push("resume"), cancel: async () => calls.push("cancel"), erase: async () => calls.push("erase"),
  };
  const messages = [];
  const handled = await interceptBrowserDownload(item, {
    downloads, settings, runtimeId: "extension-id", wait: async () => {},
    sendTask: async () => { throw new Error("desktop offline"); },
    notify: (...args) => messages.push(args),
  });
  assert.equal(handled, false);
  assert.deepEqual(calls, ["pause", "resume"]);
  assert.match(messages[0][0], /回退浏览器下载/);
});
