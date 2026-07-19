import { normalizeExtension, normalizeHost, parseRules } from "./rules.js";

const $ = (id) => document.getElementById(id);
const fields = ["allowHosts", "blockHosts", "extensions"];

function showMessage(text, error = false) {
  $("message").textContent = text;
  $("message").classList.toggle("error", error);
}

function render(settings) {
  for (const field of fields) $(field).value = (settings[field] || []).join("\n");
}

function collect() {
  const allowHosts = parseRules($("allowHosts").value, normalizeHost);
  const blockHosts = parseRules($("blockHosts").value, normalizeHost);
  const extensions = parseRules($("extensions").value, normalizeExtension);
  const invalid = [...allowHosts.invalid, ...blockHosts.invalid, ...extensions.invalid];
  if (invalid.length) throw new Error(`以下规则格式无效：${invalid.slice(0, 5).join("、")}`);
  return { allowHosts: allowHosts.values, blockHosts: blockHosts.values, extensions: extensions.values };
}

render(await chrome.storage.local.get(fields));

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
    render(payload);
    const rules = collect();
    await chrome.storage.local.set(rules);
    render(rules);
    showMessage("规则已导入并保存。", false);
  } catch (error) { showMessage(error.message || String(error), true); }
  event.target.value = "";
};
