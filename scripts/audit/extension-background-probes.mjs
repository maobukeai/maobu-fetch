// Exercises the real background message/event handlers with isolated Chrome/fetch mocks.
export async function runBackgroundProbes() {
  const previousChrome = globalThis.chrome;
  const previousFetch = globalThis.fetch;
  const state = { bridgeToken: "synthetic-audit-token", intercept: false, minSizeMb: 1 };
  const handlers = {};
  const captured = [];
  let failTask = false;
  const area = {
    get: async (keys) => Object.fromEntries((Array.isArray(keys) ? keys : [keys]).filter((key) => key in state).map((key) => [key, state[key]])),
    set: async (values) => { Object.assign(state, values); },
    remove: async (key) => { delete state[key]; },
  };
  globalThis.chrome = {
    storage: { local: area, session: { get: async () => ({}), set: async () => {} } },
    runtime: {
      id: "synthetic-audit-extension", onInstalled: { addListener() {} },
      onMessage: { addListener: (fn) => { handlers.message = fn; } },
    },
    contextMenus: { removeAll: (fn) => fn?.(), create() {}, onClicked: { addListener() {} } },
    downloads: { onCreated: { addListener: (fn) => { handlers.download = fn; } }, resume: async () => {} },
    tabs: { query: async () => [], sendMessage: async () => ({}), onUpdated: { addListener() {} } },
    notifications: { create() {} },
    cookies: { getAll: async () => [{ name: "audit_session", value: "SYNTHETIC" }] },
  };
  globalThis.fetch = async (url, options = {}) => {
    if (String(url).includes("/v1/health")) return { ok: true, status: 200, json: async () => ({ bt_magnet_enabled: true, takeover_enabled: true }) };
    if (String(url).endsWith("/v1/tasks")) {
      captured.push(JSON.parse(options.body));
      return failTask
        ? { ok: false, status: 401, text: async () => "签名验证失败" }
        : { ok: true, status: 201, json: async () => ({ id: "synthetic-task" }) };
    }
    if (String(url).endsWith("/v1/focus")) return { ok: true, status: 200 };
    throw new Error("Audit blocked unexpected fetch destination");
  };
  try {
    const background = await import("../../extension/src/background.js");
    const send = (message) => new Promise((resolve) => handlers.message(message, {
      tab: { id: 1, url: "https://unrelated.example/" },
    }, resolve));
    const lanDisk = await send({
      type: "send-landisk-batch-task", url: "https://target.example/action", fileName: "audit.bin", body: "operation=synthetic",
    });
    await handlers.download({ id: 7, url: "https://example.com/a.txt", totalBytes: 100, state: "in_progress" });
    state.bridgeToken = "synthetic-audit-token";
    state.interceptMagnet = true;
    state.intercept = true;
    failTask = true;
    background.resetDesktopGateCacheForTest();
    const magnet = await send({ type: "send-magnet", url: "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567" });
    return {
      lanDiskTaskAcceptedWithInterceptionDisabled: lanDisk.ok === true,
      lanDiskForwardsPostAndCookie: captured[0]?.method === "POST" && Boolean(captured[0]?.headers?.Cookie),
      explicitOneMbThresholdRewrittenToZero: state.minSizeMb === 0,
      magnet401Result: { ok: magnet.ok, fallback: magnet.fallback },
    };
  } finally {
    globalThis.chrome = previousChrome;
    globalThis.fetch = previousFetch;
  }
}
