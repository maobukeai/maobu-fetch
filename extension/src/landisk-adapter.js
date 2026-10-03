// 猫步下载器 · 局域网互联 Pro / LanDisk 专属网页适配器
//
// 职责：
//   1. 同时兼容 Chrome MV3 Isolated World（负责与 background Service Worker 通信）
//      与 Main World（负责拦截页面 `window.FileBatchManager.prototype.downloadZip` 与原生表单提交）；
//   2. 将文件夹/多文件打包下载统一转化为单个 16 路并发、支持随时暂停与断点续传的 `.zip` 任务，
//      杜绝拆散为上千个散碎子任务、杜绝通知刷屏、杜绝暂停后进度归零；
//   3. 桌面端离线或未配对时 100% 安全回退到网页原生下载，符合 AGENTS.md §5 规范。

(() => {
  if (typeof window === "undefined") return;

  const hasChromeRuntime = Boolean(
    typeof chrome !== "undefined" && chrome?.runtime?.id && typeof chrome?.runtime?.sendMessage === "function"
  );

  // ── 1. Isolated World 桥接层：接收来自 Main World 的 postMessage 并转发至 background.js ──
  if (hasChromeRuntime && !window.__maobuLanDiskBridgeListening) {
    window.__maobuLanDiskBridgeListening = true;
    window.addEventListener("message", (event) => {
      if (event.source !== window) return;
      const data = event.data;
      if (!data || data.source !== "maobu-landisk-main" || data.type !== "SEND_LANDISK_BATCH_TASK") {
        return;
      }
      const { reqId, payload } = data;
      try {
        chrome.runtime.sendMessage(
          {
            type: "send-landisk-batch-task",
            url: payload.url,
            fileName: payload.fileName,
            body: payload.body,
            pin: payload.pin || "",
            contentType: payload.contentType || "application/x-www-form-urlencoded",
          },
          (response) => {
            const err = chrome?.runtime?.lastError;
            const ok = Boolean(!err && response && response.ok);
            window.postMessage(
              {
                source: "maobu-landisk-isolated",
                type: "LANDISK_BATCH_RESULT",
                reqId,
                ok,
                error: err?.message || response?.error || "",
              },
              "*"
            );
          }
        );
      } catch (e) {
        window.postMessage(
          {
            source: "maobu-landisk-isolated",
            type: "LANDISK_BATCH_RESULT",
            reqId,
            ok: false,
            error: String(e?.message || e || "extension context invalidated"),
          },
          "*"
        );
      }
    });
  }

  // 防止同一执行环境重复注入钩子
  if (window.__maobuLanDiskHookInstalled) return;
  window.__maobuLanDiskHookInstalled = true;

  const isTailscaleOrLanHost = (hostname = "") => {
    const h = (hostname || window.location.hostname || "").toLowerCase();
    if (h === "localhost" || h === "127.0.0.1" || h === "::1" || h.startsWith("[") || h.endsWith(".local") || h.endsWith(".lan")) return true;
    if (h.endsWith(".ts.net") || h.endsWith(".tailscale.net")) return true;
    const m = h.match(/^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/);
    if (m) {
      const b0 = parseInt(m[1], 10);
      const b1 = parseInt(m[2], 10);
      if (b0 === 100 && b1 >= 64 && b1 <= 127) return true; // Tailscale CGNAT
      if (b0 === 10 || b0 === 127 || (b0 === 172 && b1 >= 16 && b1 <= 31) || (b0 === 192 && b1 === 168)) return true; // Private LAN
    }
    return false;
  };

  const getPin = () => {
    try {
      if (typeof window.LanDiskAuth !== "undefined" && typeof window.LanDiskAuth.getPin === "function") {
        return window.LanDiskAuth.getPin() || "";
      }
      return localStorage.getItem("lan_disk_pin") || "";
    } catch {
      return "";
    }
  };

  const getAuthQuery = () => {
    try {
      let q = "";
      if (window.LanDiskAuth && typeof window.LanDiskAuth.authQuery === "function") {
        q = window.LanDiskAuth.authQuery() || "";
      }
      const pin = getPin();
      if (pin && !q.includes("pin=")) {
        q += (q ? "&" : "?") + "pin=" + encodeURIComponent(pin);
      }
      return q;
    } catch {
      return "";
    }
  };

  // 统一派发单任务打包下载（自动兼容 Isolated World 直发与 Main World 跨界转发）
  function dispatchBatchTask(payload, callback) {
    if (hasChromeRuntime) {
      try {
        chrome.runtime.sendMessage(
          {
            type: "send-landisk-batch-task",
            url: payload.url,
            fileName: payload.fileName,
            body: payload.body,
            pin: payload.pin || "",
            contentType: payload.contentType || "application/x-www-form-urlencoded",
          },
          (response) => {
            const err = chrome?.runtime?.lastError;
            callback(Boolean(!err && response && response.ok), response?.error || err?.message);
          }
        );
        return;
      } catch {
        // Fallback to postMessage
      }
    }

    const reqId = "maobu_" + Date.now() + "_" + Math.random().toString(36).slice(2);
    let settled = false;
    const timer = setTimeout(() => {
      if (settled) return;
      settled = true;
      window.removeEventListener("message", onMsg);
      callback(false, "timeout");
    }, 3500);

    function onMsg(event) {
      if (event.source !== window) return;
      const d = event.data;
      if (!d || d.source !== "maobu-landisk-isolated" || d.type !== "LANDISK_BATCH_RESULT" || d.reqId !== reqId) {
        return;
      }
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      window.removeEventListener("message", onMsg);
      callback(Boolean(d.ok), d.error);
    }

    window.addEventListener("message", onMsg);
    window.postMessage(
      {
        source: "maobu-landisk-main",
        type: "SEND_LANDISK_BATCH_TASK",
        reqId,
        payload,
      },
      "*"
    );
  }

  function buildBatchRequestPayload(rawFilesArr, customFolderName, instance) {
    let filesArr = rawFilesArr;
    if (typeof filesArr === "string") filesArr = [filesArr];
    else if (filesArr instanceof Set) filesArr = Array.from(filesArr);
    else if (!Array.isArray(filesArr)) {
      try {
        filesArr = Array.from(filesArr || []);
      } catch {
        filesArr = filesArr ? [filesArr] : [];
      }
    }
    filesArr = filesArr.filter((f) => typeof f === "string" && f.trim());
    if (filesArr.length === 0) return null;

    let resolvedFolderName = customFolderName;
    if (!resolvedFolderName || resolvedFolderName === "batch_download") {
      if (filesArr.length === 1) {
        resolvedFolderName = filesArr[0].split(/[\\/]/).filter(Boolean).pop() || "batch_download";
      } else {
        resolvedFolderName = `batch_download_${filesArr.length}_items`;
      }
    }

    const pin = (instance?.getPin ? instance.getPin() : getPin()) || "";
    const authQ = getAuthQuery();
    const getApiUrl = instance?.getApiUrl ? instance.getApiUrl.bind(instance) : (u) => u;
    const baseBatchPath = getApiUrl("/api/download/batch");
    const sep = baseBatchPath.includes("?") ? "&" : "?";
    const cleanAuthQ = authQ ? authQ.replace(/^[?&]/, "") : "";
    const apiUrl = cleanAuthQ ? `${baseBatchPath}${sep}${cleanAuthQ}` : baseBatchPath;
    const fullUrl = new URL(apiUrl, window.location.href).href;

    const bodyParams = new URLSearchParams();
    bodyParams.append("folderName", resolvedFolderName);
    filesArr.forEach((f) => bodyParams.append("files", f));
    if (pin) bodyParams.append("pin", pin);

    return {
      url: fullUrl,
      fileName: `${resolvedFolderName}.zip`,
      body: bodyParams.toString(),
      pin,
      contentType: "application/x-www-form-urlencoded",
    };
  }

  // ── 2. 拦截 FileBatchManager.prototype.downloadZip（Main World） ──
  function hookBatchManager() {
    if (!window.FileBatchManager || !window.FileBatchManager.prototype) return;
    if (window.FileBatchManager.prototype.__maobuHooked) return;
    window.FileBatchManager.prototype.__maobuHooked = true;

    const originalDownloadZip = window.FileBatchManager.prototype.downloadZip;

    window.FileBatchManager.prototype.downloadZip = function (customFilesArr = null, customFolderName = "batch_download") {
      const filesSource = customFilesArr || Array.from(this.selectedFiles || []);
      const payload = buildBatchRequestPayload(filesSource, customFolderName, this);
      if (!payload) {
        return originalDownloadZip.call(this, customFilesArr, customFolderName);
      }

      dispatchBatchTask(payload, (ok) => {
        if (ok) {
          if (window.LanDiskUI && typeof window.LanDiskUI.toast === "function") {
            window.LanDiskUI.toast(`⚡ 猫步下载器已接管：${payload.fileName}（16 路并发 · 支持随时断点续传）`, "success");
          }
        } else {
          window.__maobuBypassFormSubmit = true;
          try {
            originalDownloadZip.call(this, customFilesArr, customFolderName);
          } finally {
            setTimeout(() => {
              window.__maobuBypassFormSubmit = false;
            }, 500);
          }
        }
      });
    };
  }

  // ── 3. 兜底拦截 HTMLFormElement.prototype.submit ──
  function hookFormSubmit() {
    if (typeof HTMLFormElement === "undefined" || !HTMLFormElement.prototype) return;
    if (HTMLFormElement.prototype.__maobuHooked) return;
    HTMLFormElement.prototype.__maobuHooked = true;

    const originalSubmit = HTMLFormElement.prototype.submit;
    HTMLFormElement.prototype.submit = function () {
      if (window.__maobuBypassFormSubmit) {
        return originalSubmit.call(this);
      }
      try {
        const actionUrl = this.action || "";
        if (actionUrl.includes("/api/download/batch")) {
          const formData = new FormData(this);
          const bodyParams = new URLSearchParams();
          for (const [k, v] of formData.entries()) {
            bodyParams.append(k, String(v));
          }
          const pin = getPin();
          if (pin && !bodyParams.has("pin")) {
            bodyParams.append("pin", pin);
          }
          const filesArr = bodyParams.getAll("files");
          let folderName = String(formData.get("folderName") || "batch_download");
          if ((!folderName || folderName === "batch_download") && filesArr.length === 1) {
            folderName = filesArr[0].split(/[\\/]/).filter(Boolean).pop() || "batch_download";
          }
          const targetUrlObj = new URL(actionUrl, window.location.href);
          if (pin && !targetUrlObj.searchParams.has("pin")) {
            targetUrlObj.searchParams.set("pin", pin);
          }

          dispatchBatchTask(
            {
              url: targetUrlObj.href,
              fileName: `${folderName}.zip`,
              body: bodyParams.toString(),
              pin,
              contentType: "application/x-www-form-urlencoded",
            },
            (ok) => {
              if (ok) {
                if (window.LanDiskUI && typeof window.LanDiskUI.toast === "function") {
                  window.LanDiskUI.toast(`⚡ 猫步下载器已接管：${folderName}.zip（16 路并发 · 支持随时断点续传）`, "success");
                }
              } else {
                originalSubmit.call(this);
              }
            }
          );
          return;
        }
      } catch {}
      return originalSubmit.call(this);
    };
  }

  // ── 4. 在 #apple-floating-batch-bar 注入“⚡ 猫步并发极速下载”按钮 ──
  function injectTurboButton() {
    // 仅在一个环境中注入 DOM 按钮（优先 Isolated World，避免双环境重复创建）
    if (!hasChromeRuntime && typeof chrome !== "undefined") return;
    const bar = document.getElementById("apple-floating-batch-bar");
    if (!bar) return;
    if (document.getElementById("btn-maobu-batch-turbo")) return;

    const zipBtn = document.getElementById("btn-batch-zip");
    if (!zipBtn) return;

    const turboBtn = document.createElement("button");
    turboBtn.id = "btn-maobu-batch-turbo";
    turboBtn.className = "apple-btn apple-btn-sm";
    turboBtn.innerHTML = "⚡ 猫步并发极速下载";
    turboBtn.style.cssText =
      "background: linear-gradient(135deg, #10b981 0%, #059669 100%);" +
      "color: #ffffff;" +
      "font-weight: 600;" +
      "margin-left: 6px;" +
      "border: none;" +
      "cursor: pointer;" +
      "border-radius: 6px;" +
      "padding: 5px 12px;" +
      "display: inline-flex;" +
      "align-items: center;" +
      "gap: 4px;" +
      "font-size: 13px;" +
      "box-shadow: 0 2px 6px rgba(16, 185, 129, 0.3);" +
      "transition: all 0.2s ease;";
    turboBtn.title = "【Tailscale/IPv6 远程加速】单任务 16 路并发分桶加速下载为单个 ZIP，支持随时暂停与断点续传！";

    turboBtn.onclick = async () => {
      let selectedPaths = [];
      if (window.FileExplorerComponent?.batchManager?.selectedFiles) {
        selectedPaths = Array.from(window.FileExplorerComponent.batchManager.selectedFiles);
      } else if (window.fileBatchManager?.selectedFiles) {
        selectedPaths = Array.from(window.fileBatchManager.selectedFiles);
      } else {
        document
          .querySelectorAll(".file-select-checkbox:checked, input[type='checkbox'][data-path]:checked, .file-item.selected[data-path]")
          .forEach((el) => {
            const p = el.getAttribute("data-path") || el.value;
            if (p) selectedPaths.push(p);
          });
      }

      selectedPaths = selectedPaths.filter(Boolean);
      if (selectedPaths.length === 0) {
        // 如果 DOM checkbox 未挂 data-path，直接触发已 Hook 的 #btn-batch-zip
        zipBtn.click();
        return;
      }

      const payload = buildBatchRequestPayload(selectedPaths, "batch_download", null);
      if (!payload) {
        zipBtn.click();
        return;
      }

      turboBtn.textContent = "正在下发…";
      dispatchBatchTask(payload, (ok, errMsg) => {
        turboBtn.textContent = "⚡ 猫步并发极速下载";
        if (ok) {
          if (window.LanDiskUI?.toast) {
            window.LanDiskUI.toast(`⚡ 猫步已接管：${payload.fileName}（16 路并发 · 支持续传）`, "success");
          }
        } else {
          const msg = errMsg || "请确保猫步下载器已运行并完成配对";
          if (window.LanDiskUI?.toast) window.LanDiskUI.toast(msg, "error");
        }
      });
    };

    zipBtn.insertAdjacentElement("afterend", turboBtn);
  }

  const observer = new MutationObserver(() => {
    hookBatchManager();
    hookFormSubmit();
    injectTurboButton();
  });

  const init = () => {
    hookBatchManager();
    hookFormSubmit();
    injectTurboButton();

    if (document.body) {
      observer.observe(document.body, { childList: true, subtree: true });
    } else {
      document.addEventListener("DOMContentLoaded", () => {
        if (document.body) {
          observer.observe(document.body, { childList: true, subtree: true });
        }
      });
    }

    setInterval(() => {
      hookBatchManager();
      hookFormSubmit();
      injectTurboButton();
    }, 1000);
  };

  init();
})();
