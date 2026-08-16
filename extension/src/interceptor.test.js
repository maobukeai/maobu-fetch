import test from "node:test";
import assert from "node:assert/strict";
import { evaluateDownload, interceptBrowserDownload, refreshDownload, resetNotificationCooldownsForTest, skipUnpairedDownload } from "./interceptor.js";

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
  resetNotificationCooldownsForTest();
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

test("throttles repeated failure notifications within cooldown period", async () => {
  resetNotificationCooldownsForTest();
  const item = { id: 6, url: "https://example.com/file.zip", finalUrl: "https://example.com/file.zip", filename: "file.zip", totalBytes: 3_000_000 };
  const downloads = {
    pause: async () => {}, search: async () => [item], resume: async () => {}, cancel: async () => {}, erase: async () => {},
  };
  const messages = [];
  const options = {
    downloads, settings, runtimeId: "extension-id", wait: async () => {},
    sendTask: async () => { throw new Error("请求过于频繁"); },
    notify: (...args) => messages.push(args),
    isDesktopOfflineError: () => false,
  };

  const handled1 = await interceptBrowserDownload(item, options);
  const handled2 = await interceptBrowserDownload(item, options);

  assert.equal(handled1, false);
  assert.equal(handled2, false);
  // First call should trigger notification, second call should be throttled by cooldown
  assert.equal(messages.length, 1);
  assert.equal(messages[0][0], "接管失败，已回退浏览器下载");
  assert.equal(messages[0][1], "请求过于频繁");
  assert.equal(messages[0][2], "takeover-error");
});

test("ignores restored history download items with past startTime or existing progress", () => {
  const swStartTime = 1000000;
  // 1. 过去的 startTime
  const oldItem = {
    id: 10,
    url: "https://example.com/old.zip",
    filename: "old.zip",
    totalBytes: 5_000_000,
    startTime: new Date(swStartTime - 10000).toISOString(),
  };
  const resultOld = evaluateDownload(oldItem, settings, "extension-id", swStartTime);
  assert.equal(resultOld.eligible, false);
  assert.equal(resultOld.reason, "restored-history");

  // 2. 带已有进度的任务
  const progressItem = {
    id: 11,
    url: "https://example.com/progress.zip",
    filename: "progress.zip",
    totalBytes: 5_000_000,
    bytesReceived: 1024,
    startTime: new Date(swStartTime + 100).toISOString(),
  };
  const resultProgress = evaluateDownload(progressItem, settings, "extension-id", swStartTime);
  assert.equal(resultProgress.eligible, false);
  assert.equal(resultProgress.reason, "restored-history");

  // 3. 被暂停/可恢复的任务
  const pausedItem = {
    id: 12,
    url: "https://example.com/paused.zip",
    filename: "paused.zip",
    totalBytes: 5_000_000,
    paused: true,
    startTime: new Date(swStartTime + 100).toISOString(),
  };
  const resultPaused = evaluateDownload(pausedItem, settings, "extension-id", swStartTime);
  assert.equal(resultPaused.eligible, false);
  assert.equal(resultPaused.reason, "restored-history");

  // 4. 真正的新新建任务
  const newItem = {
    id: 13,
    url: "https://example.com/new.zip",
    filename: "new.zip",
    totalBytes: 5_000_000,
    bytesReceived: 0,
    paused: false,
    state: "in_progress",
    startTime: new Date(swStartTime + 500).toISOString(),
  };
  const resultNew = evaluateDownload(newItem, settings, "extension-id", swStartTime);
  assert.equal(resultNew.eligible, true);
});

test("rejects downloads with interrupted/complete/cancelled state as restored-history", () => {
  const swStartTime = Date.now();
  for (const state of ["interrupted", "complete", "cancelled"]) {
    const item = {
      id: 20, url: "https://example.com/file.zip", filename: "file.zip",
      totalBytes: 5_000_000, state,
      startTime: new Date(swStartTime + 100).toISOString(),
    };
    const result = evaluateDownload(item, settings, "extension-id", swStartTime);
    assert.equal(result.eligible, false, `state=${state} should be rejected`);
    assert.equal(result.reason, "restored-history");
  }
});

test("rejects downloads with canResume=true as restored-history", () => {
  const swStartTime = Date.now();
  const item = {
    id: 21, url: "https://example.com/file.zip", filename: "file.zip",
    totalBytes: 5_000_000, canResume: true,
    startTime: new Date(swStartTime + 100).toISOString(),
  };
  const result = evaluateDownload(item, settings, "extension-id", swStartTime);
  assert.equal(result.eligible, false);
  assert.equal(result.reason, "restored-history");
});

test("timestamp check uses 2-second tolerance correctly", () => {
  const swStartTime = 1000000;
  // 1.5 秒前：在 2 秒容差内，应该通过时间检查（不被 startTime 拦截）
  const withinTolerance = {
    id: 22, url: "https://example.com/edge.zip", filename: "edge.zip",
    totalBytes: 5_000_000, bytesReceived: 0, paused: false, state: "in_progress",
    startTime: new Date(swStartTime - 1500).toISOString(),
  };
  const result1 = evaluateDownload(withinTolerance, settings, "extension-id", swStartTime);
  assert.equal(result1.eligible, true, "item within 2s tolerance should pass");

  // 恰好 2 秒前：边界值，不应被拦截（需要严格小于 swStartTime - 2000）
  const exactBoundary = {
    id: 23, url: "https://example.com/boundary.zip", filename: "boundary.zip",
    totalBytes: 5_000_000, bytesReceived: 0, paused: false, state: "in_progress",
    startTime: new Date(swStartTime - 2000).toISOString(),
  };
  const result2 = evaluateDownload(exactBoundary, settings, "extension-id", swStartTime);
  assert.equal(result2.eligible, true, "item at exact 2s boundary should pass (not strictly less)");

  // 2.1 秒前：超出容差，应被拦截
  const beyondTolerance = {
    id: 24, url: "https://example.com/old.zip", filename: "old.zip",
    totalBytes: 5_000_000,
    startTime: new Date(swStartTime - 2100).toISOString(),
  };
  const result3 = evaluateDownload(beyondTolerance, settings, "extension-id", swStartTime);
  assert.equal(result3.eligible, false, "item beyond 2s tolerance should be rejected");
  assert.equal(result3.reason, "restored-history");
});

test("without swStartTime, falls back to state/progress checks only", () => {
  // swStartTime=0 时跳过时间校验，仅靠 bytesReceived/paused/state 兜底
  const pausedItem = {
    id: 30, url: "https://example.com/file.zip", filename: "file.zip",
    totalBytes: 5_000_000, paused: true,
    startTime: new Date(Date.now() - 86400000).toISOString(),
  };
  const result1 = evaluateDownload(pausedItem, settings, "extension-id", 0);
  assert.equal(result1.eligible, false, "paused item should be rejected even without swStartTime");
  assert.equal(result1.reason, "restored-history");

  // 干净的新任务，无 swStartTime 时应通过
  const freshItem = {
    id: 31, url: "https://example.com/file.zip", filename: "file.zip",
    totalBytes: 5_000_000, bytesReceived: 0, paused: false, state: "in_progress",
  };
  const result2 = evaluateDownload(freshItem, settings, "extension-id", 0);
  assert.equal(result2.eligible, true, "fresh item should pass without swStartTime");
});

test("interceptBrowserDownload skips restored history items without sending task", async () => {
  resetNotificationCooldownsForTest();
  const swStartTime = Date.now();
  const calls = [];
  const restoredItem = {
    id: 40, url: "https://example.com/restored.zip", finalUrl: "https://example.com/restored.zip",
    filename: "restored.zip", totalBytes: 5_000_000,
    startTime: new Date(swStartTime - 60000).toISOString(),
  };
  const downloads = {
    pause: async () => calls.push("pause"),
    search: async () => [restoredItem],
    resume: async () => calls.push("resume"),
    cancel: async () => calls.push("cancel"),
    erase: async () => calls.push("erase"),
  };
  const handled = await interceptBrowserDownload(restoredItem, {
    downloads, settings, runtimeId: "extension-id", wait: async () => {},
    sendTask: async () => { calls.push("send"); },
    swStartTime,
  });
  assert.equal(handled, false, "restored item should not be handled");
  assert.ok(!calls.includes("send"), "sendTask must NOT be called for restored items");
  assert.ok(!calls.includes("pause"), "pause must NOT be called for restored items");
  assert.ok(calls.includes("resume"), "resume should be called to let browser handle it");
});

test("re-evaluation passes for a download paused by the extension itself", () => {
  const swStartTime = Date.now();
  const pausedItem = {
    id: 50, url: "https://example.com/file.zip", finalUrl: "https://example.com/file.zip",
    filename: "file.zip", totalBytes: 3_000_000,
    paused: true, canResume: true, bytesReceived: 2048, state: "in_progress",
    startTime: new Date(swStartTime + 100).toISOString(),
  };
  // 首次评估（onCreated 入口）应拒绝：这些标志在真实新下载上意味着会话恢复。
  const first = evaluateDownload(pausedItem, settings, "extension-id", swStartTime);
  assert.equal(first.eligible, false);
  assert.equal(first.reason, "restored-history");
  // 重评估（扩展自己 pause() 之后）应放行：标志由拦截流程自身造成。
  const reevaluation = evaluateDownload(pausedItem, settings, "extension-id", swStartTime, { reevaluation: true });
  assert.equal(reevaluation.eligible, true);
  assert.equal(reevaluation.url, pausedItem.finalUrl);
});

test("interceptBrowserDownload completes takeover when search reflects the extension pause", async () => {
  // 回归测试（v0.6.9 修复）：真实 Chrome 中 pause() 之后 search() 返回的快照带
  // paused/canResume/bytesReceived，此前二次评估必被误判 restored-history，
  // 导致接管 100% 失效、下载全部回退浏览器。
  resetNotificationCooldownsForTest();
  const swStartTime = Date.now();
  const calls = [];
  const sent = [];
  const pausedSnapshot = {
    id: 51, url: "https://example.com/file.zip", finalUrl: "https://example.com/file.zip",
    filename: "file.zip", totalBytes: 3_000_000,
    paused: true, canResume: true, bytesReceived: 4096, state: "in_progress",
    startTime: new Date(swStartTime + 100).toISOString(),
  };
  const downloads = {
    pause: async () => calls.push("pause"),
    search: async () => [pausedSnapshot],
    cancel: async () => calls.push("cancel"),
    erase: async () => calls.push("erase"),
    resume: async () => calls.push("resume"),
  };
  const initial = { ...pausedSnapshot, paused: false, canResume: undefined, bytesReceived: 0 };
  const handled = await interceptBrowserDownload(initial, {
    downloads, settings, runtimeId: "extension-id", wait: async () => {},
    sendTask: async (...args) => { calls.push("send"); sent.push(args); },
    swStartTime,
  });
  assert.equal(handled, true, "paused-by-extension download must still be taken over");
  assert.deepEqual(calls, ["pause", "send", "cancel", "erase"]);
  assert.equal(sent[0][0], pausedSnapshot.finalUrl);
});

test("skipUnpairedDownload falls back to browser and records reason when unpaired", async () => {
  resetNotificationCooldownsForTest();
  const originalChrome = globalThis.chrome;
  const stored = {};
  const messages = [];
  globalThis.chrome = {
    storage: {
      local: {
        get: async (key) => (key === "bridgeToken" ? {} : {}),
        set: async (entries) => { Object.assign(stored, entries); },
      },
      session: { get: async () => ({}), set: async () => {}, remove: async () => {} },
    },
  };
  try {
    const item = {
      id: 60, url: "https://example.com/file.zip", finalUrl: "https://example.com/file.zip",
      filename: "file.zip", totalBytes: 3_000_000,
    };
    const skipped = await skipUnpairedDownload(item, (...args) => messages.push(args));
    assert.equal(skipped, true, "unpaired download should be skipped (browser fallback)");
    assert.equal(stored.lastIgnored?.reason, "unpaired");
    assert.equal(messages.length, 1);
    assert.match(messages[0][0], /尚未与桌面端配对/);
    // 未配对提示按 5 分钟长周期节流，第二次不再弹。
    const skippedAgain = await skipUnpairedDownload(item, (...args) => messages.push(args));
    assert.equal(skippedAgain, true);
    assert.equal(messages.length, 1, "unpaired notification must be throttled");
  } finally {
    globalThis.chrome = originalChrome;
  }
});

test("skipUnpairedDownload returns false when bridge token exists", async () => {
  resetNotificationCooldownsForTest();
  const originalChrome = globalThis.chrome;
  globalThis.chrome = {
    storage: {
      local: { get: async () => ({ bridgeToken: "token" }), set: async () => {} },
      session: { get: async () => ({}), set: async () => {}, remove: async () => {} },
    },
  };
  try {
    const skipped = await skipUnpairedDownload(
      { url: "https://example.com/a.zip", filename: "a.zip" },
      () => assert.fail("已配对时不应发通知"),
    );
    assert.equal(skipped, false);
  } finally {
    globalThis.chrome = originalChrome;
  }
});

test("respects desktop-side takeover switch when provided", () => {
  const swStartTime = Date.now();
  const item = {
    id: 70, url: "https://example.com/file.zip", finalUrl: "https://example.com/file.zip",
    filename: "file.zip", totalBytes: 3_000_000,
    startTime: new Date(swStartTime + 100).toISOString(),
  };
  // 桌面端开关未知（旧桌面端/离线，字段缺失）：不阻断。
  const unknown = evaluateDownload(item, settings, "extension-id", swStartTime);
  assert.equal(unknown.eligible, true);
  // 桌面端显式关闭接管：拒绝并给出独立原因。
  const disabled = evaluateDownload(item, { ...settings, desktopTakeoverEnabled: false }, "extension-id", swStartTime);
  assert.equal(disabled.eligible, false);
  assert.equal(disabled.reason, "desktop-disabled");
  // 桌面端开启（true）：正常评估。
  const enabled = evaluateDownload(item, { ...settings, desktopTakeoverEnabled: true }, "extension-id", swStartTime);
  assert.equal(enabled.eligible, true);
});

