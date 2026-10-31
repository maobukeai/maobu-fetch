import { validateLanDiskRequest } from "../../extension/src/landisk-security.js";
import { getDownloadAuthHeaders, evaluateDownload } from "../../extension/src/interceptor.js";
import { attachSniffer } from "../../extension/src/sniffer.js";

function probeLanDiskMessage() {
  let rejected = false;
  try {
    validateLanDiskRequest({ url: "https://target.example/action", body: "operation=synthetic" }, {
      tab: { id: 7, url: "https://unrelated.example/" }, frameId: 0, url: "https://unrelated.example/",
    }, { intercept: true });
  } catch { rejected = true; }
  return { unrelatedPageMessageForwarded: !rejected };
}

async function probeStaleSniffer() {
  const listeners = {};
  const chrome = {
    storage: { local: { get: async () => ({ snifferHosts: ["source.example"] }) } },
    tabs: {
      sendMessage: async () => {},
      onRemoved: { addListener: (fn) => { listeners.removed = fn; } },
      onUpdated: { addListener: (fn) => { listeners.updated = fn; } },
    },
    webRequest: { onBeforeRequest: { addListener: (fn) => { listeners.request = fn; }, removeListener() {} } },
  };
  const sniffer = attachSniffer({ chrome });
  await new Promise((resolve) => setImmediate(resolve));
  listeners.request({ tabId: 7, initiator: "https://source.example", url: "https://cdn.example/old.mp4" });
  listeners.updated?.(7, { url: "https://other.example" }, { id: 7, url: "https://other.example" });
  const result = { navigationListenerRegistered: Boolean(listeners.updated), oldMediaRemainsAfterNavigation: sniffer.getItems(7).length === 1 };
  sniffer.stop();
  return result;
}

export async function runLocalProbes(root) {
  const sourceUrl = "https://account.example/private";
  const targetUrl = "https://cdn.example/file.bin";
  const headers = await getDownloadAuthHeaders({ url: targetUrl, referrer: sourceUrl }, {
    tab: { id: 1, url: sourceUrl }, url: targetUrl,
    cookies: { getAllCookieStores: async () => [{ id: "normal", tabIds: [1] }], getAll: async ({ url }) => url === sourceUrl ? [{ name: "audit_session", value: "SYNTHETIC" }] : [] },
  });
  const decision = evaluateDownload({
    url: "https://example.com/small.zip", filename: "small.zip", totalBytes: 10, state: "in_progress",
  }, { intercept: true, minSizeMb: 100 }, "synthetic");
  return {
    cookieScope: { unrelatedPageCookieForwarded: headers.Cookie === "audit_session=SYNTHETIC" },
    sizeThreshold: { tenByteArchiveEligibleWith100MbMinimum: decision.eligible },
    lanDiskMessage: probeLanDiskMessage(),
    snifferNavigation: await probeStaleSniffer(),
  };
}
