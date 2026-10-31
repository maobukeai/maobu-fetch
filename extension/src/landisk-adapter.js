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

  // 防止同一执行环境重复注入钩子
  if (window.__maobuLanDiskHookInstalled) return;
  window.__maobuLanDiskHookInstalled = true;

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
    }, 65_000);

    function onMsg(event) {
      if (event.source !== window || event.origin !== window.location.origin) return;
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
      window.location.origin
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

  // ── 1.5 跨执行环境通用悬浮 Toast 提示 ──
  function showFloatingToast(message, isSuccess = true) {
    try {
      let toast = document.getElementById("maobu-landisk-floating-toast");
      if (!toast) {
        toast = document.createElement("div");
        toast.id = "maobu-landisk-floating-toast";
        toast.style.cssText =
          "position: fixed; bottom: 85px; left: 50%; transform: translateX(-50%);" +
          "background: rgba(15, 23, 42, 0.95); color: #ffffff; padding: 10px 22px;" +
          "border-radius: 9999px; font-size: 13px; font-weight: 600;" +
          "box-shadow: 0 10px 25px -5px rgba(0, 0, 0, 0.4), 0 8px 10px -6px rgba(0, 0, 0, 0.3);" +
          "z-index: 100000; pointer-events: none; transition: opacity 0.3s ease, transform 0.3s ease;" +
          "border: 1px solid rgba(255, 255, 255, 0.15); display: flex; align-items: center; gap: 8px;" +
          "backdrop-filter: blur(12px); -webkit-backdrop-filter: blur(12px);";
        document.body.appendChild(toast);
      }
      toast.innerHTML = `<span style="font-size: 16px;">${isSuccess ? "⚡" : "❌"}</span><span>${message}</span>`;
      toast.style.opacity = "1";
      toast.style.transform = "translateX(-50%) translateY(0)";
      clearTimeout(window.__maobuToastTimer);
      window.__maobuToastTimer = setTimeout(() => {
        if (toast) {
          toast.style.opacity = "0";
          toast.style.transform = "translateX(-50%) translateY(10px)";
        }
      }, 3500);
    } catch {}
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
          showFloatingToast(`⚡ 猫步下载器已接管：${payload.fileName}（已添加打包任务）`, true);
          if (window.LanDiskUI && typeof window.LanDiskUI.toast === "function") {
            window.LanDiskUI.toast(`⚡ 猫步下载器已接管：${payload.fileName}（已添加打包任务）`, "success");
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
        const action = new URL(actionUrl, window.location.href);
        if (action.origin === window.location.origin && action.pathname === "/api/download/batch") {
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
                showFloatingToast(`⚡ 猫步下载器已接管：${folderName}.zip（已添加打包任务）`, true);
                if (window.LanDiskUI && typeof window.LanDiskUI.toast === "function") {
                  window.LanDiskUI.toast(`⚡ 猫步下载器已接管：${folderName}.zip（已添加打包任务）`, "success");
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

  const observer = new MutationObserver(() => {
    hookBatchManager();
    hookFormSubmit();
  });

  const init = () => {
    hookBatchManager();
    hookFormSubmit();

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
    }, 1000);
  };

  init();
})();
