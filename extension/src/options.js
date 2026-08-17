import { normalizeExtension, normalizeHost, parseRules } from "./rules.js";
import { evaluateDownload } from "./interceptor.js";
import { describeIgnoredReason } from "./reasons.js";

const $ = (id) => document.getElementById(id);
const fields = ["allowHosts", "blockHosts", "extensions", "snifferHosts", "customMediaDomains"];

function showMessage(text, error = false) {
  $("message").textContent = text;
  $("message").classList.toggle("error", error);
}

function render(settings) {
  for (const field of fields) $(field).value = (settings[field] || []).join("\n");
  // 磁力接管开关：默认开启（与 background.js defaults 保持一致）。
  // 导入的规则文件可能不含此字段——仅在字段存在时更新开关状态，
  // 避免导入旧版规则文件时把用户已关闭的开关在界面上错误地显示为开启。
  if ("interceptMagnet" in settings) {
    $("interceptMagnet").checked = settings.interceptMagnet !== false;
  }
}

function collect() {
  const allowHosts = parseRules($("allowHosts").value, normalizeHost);
  const blockHosts = parseRules($("blockHosts").value, normalizeHost);
  const extensions = parseRules($("extensions").value, normalizeExtension);
  const snifferHosts = parseRules($("snifferHosts").value, normalizeHost);
  const customMediaDomains = parseRules($("customMediaDomains").value, normalizeHost);
  const invalid = [...allowHosts.invalid, ...blockHosts.invalid, ...extensions.invalid, ...snifferHosts.invalid, ...customMediaDomains.invalid];
  if (invalid.length) throw new Error(`以下规则格式无效：${invalid.slice(0, 5).join("、")}`);
  return {
    allowHosts: allowHosts.values, blockHosts: blockHosts.values, extensions: extensions.values,
    snifferHosts: snifferHosts.values, customMediaDomains: customMediaDomains.values,
  };
}

/// 逐字段解析（不抛错），供即时校验展示无效行。
function inspectField(fieldId, normalize) {
  return parseRules($(fieldId).value, normalize);
}

render(await chrome.storage.local.get([...fields, "interceptMagnet"]));

// ---- 即时校验（P3-18）：输入停顿 300ms 后逐字段标出无效条目 ----
// 此前只有点"保存规则"才报错；即时反馈让用户在离开输入框前就能改正。
let validateTimer = 0;
function validateLive() {
  const checks = [
    ["allowHosts", normalizeHost, "allowHostsHint"],
    ["blockHosts", normalizeHost, "blockHostsHint"],
    ["extensions", normalizeExtension, "extensionsHint"],
    ["snifferHosts", normalizeHost, "snifferHostsHint"],
    ["customMediaDomains", normalizeHost, "customMediaDomainsHint"],
  ];
  for (const [fieldId, normalize, hintId] of checks) {
    const { values, invalid } = inspectField(fieldId, normalize);
    const hint = $(hintId);
    if (!hint) continue;
    if (invalid.length) {
      hint.textContent = `无效条目（保存时将被忽略）：${invalid.slice(0, 4).join("、")}${invalid.length > 4 ? " 等" : ""}`;
      hint.classList.remove("hidden", "ok");
      hint.classList.add("bad");
    } else if (values.length) {
      hint.textContent = `已识别 ${values.length} 条有效规则`;
      hint.classList.remove("hidden", "bad");
      hint.classList.add("ok");
    } else {
      hint.classList.add("hidden");
    }
  }
}
for (const fieldId of fields) {
  $(fieldId).addEventListener("input", () => {
    clearTimeout(validateTimer);
    validateTimer = setTimeout(validateLive, 300);
  });
}
validateLive();

// 磁力接管开关即时保存（独立于下方"保存规则"按钮的域名/类型规则）。
$("interceptMagnet").onchange = async (event) => {
  await chrome.storage.local.set({ interceptMagnet: event.target.checked });
  showMessage(event.target.checked ? "已开启磁力链接接管。" : "已关闭磁力链接接管，将使用浏览器默认处理。", false);
};

// ---- 规则测试器（P3-18）----
// 复用 evaluateDownload 的完整判定链（含站点记忆、大小/类型/主机规则），
// 纯本地模拟，不发起网络请求。输入框回车同样触发。
async function runRuleTest() {
  const resultEl = $("testResult");
  const url = $("testUrl").value.trim();
  resultEl.className = "";
  if (!/^https?:\/\//i.test(url)) {
    resultEl.textContent = "请输入以 http:// 或 https:// 开头的链接";
    resultEl.classList.add("bad");
    return;
  }
  let rules;
  try {
    rules = collect();
  } catch {
    resultEl.textContent = "当前规则中有无效条目，请先修正后再测试";
    resultEl.classList.add("bad");
    return;
  }
  const { minSizeMb = 1, siteChoices = {} } = await chrome.storage.local
    .get(["minSizeMb", "siteChoices"]).catch(() => ({}));
  let filename = $("testFile").value.trim();
  if (!filename) {
    try { filename = new URL(url).pathname.split("/").pop() || ""; } catch { filename = ""; }
  }
  const sizeMb = Math.max(0, Number($("testSize").value) || 0);
  const decision = evaluateDownload(
    {
      url,
      finalUrl: url,
      filename,
      totalBytes: Math.round(sizeMb * 1024 * 1024),
      state: "in_progress",
      paused: false,
      bytesReceived: 0,
    },
    { intercept: true, minSizeMb, siteChoices, ...rules },
    "rule-tester",
  );
  if (decision.eligible) {
    resultEl.textContent = "✓ 会被猫步下载器接管";
    resultEl.classList.add("ok");
  } else {
    resultEl.textContent = `✗ 不会被接管：${describeIgnoredReason(decision.reason, minSizeMb)}`;
    resultEl.classList.add("bad");
  }
}
$("runTest").onclick = runRuleTest;
$("testUrl").addEventListener("keydown", (event) => {
  if (event.key === "Enter") runRuleTest();
});

$("saveRules").onclick = async () => {
  try {
    const rules = collect();
    await chrome.storage.local.set(rules);
    render(rules);
    showMessage("规则已保存，新下载会立即使用。", false);
  } catch (error) { showMessage(error.message || String(error), true); }
};

$("exportRules").onclick = () => {
  try {
    const blob = new Blob([JSON.stringify({ schema_version: 1, ...collect() }, null, 2)], { type: "application/json" });
    const link = document.createElement("a");
    link.href = URL.createObjectURL(blob);
    link.download = "maobu-extension-rules.json";
    link.click();
    setTimeout(() => URL.revokeObjectURL(link.href), 1_000);
    showMessage("规则已导出。", false);
  } catch (error) { showMessage(error.message || String(error), true); }
};

$("importRules").onclick = () => $("importFile").click();
$("importFile").onchange = async (event) => {
  const [file] = event.target.files || [];
  if (!file) return;
  try {
    if (file.size > 1024 * 1024) throw new Error("规则文件不能超过 1 MB");
    const payload = JSON.parse(await file.text());
    if (payload.schema_version !== 1) throw new Error("不支持此规则文件版本");
    const stored = await chrome.storage.local.get("interceptMagnet").catch(() => ({}));
    render({ interceptMagnet: stored.interceptMagnet, ...payload });
    const rules = collect();
    await chrome.storage.local.set(rules);
    render(rules);
    showMessage("规则已导入并保存。", false);
  } catch (error) { showMessage(error.message || String(error), true); }
  event.target.value = "";
};
