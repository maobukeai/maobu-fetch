// 猫步下载器 · 局域网互联 Pro / LanDisk 专属网页适配器
//
// 职责：
//   1. 识别局域网互联 (LanDisk) 网页环境（Tailscale 远程 / 局域网私网环境）；
//   2. 接管原生的 POST /api/download/batch 流式打包下载，杜绝浏览器单线程或未被接管丢失；
//   3. 在悬浮操作条注入“⚡ 猫步并发极速下载”按钮，多文件独立 16 线程 HTTP Range 并发直连，
//      彻底解决 Tailscale 远程高延迟链路下服务端单线程实时 ZIP 打包的性能瓶颈；
//   4. 失败时安全回退到网页原生行为，符合 AGENTS.md §5 规范。

(() => {
  if (typeof window === "undefined") return;
  if (window.__maobuLanDiskInjected) return;
  window.__maobuLanDiskInjected = true;

  const isTailscaleOrLanHost = (hostname = "") => {
    const h = (hostname || window.location.hostname || "").toLowerCase();
    if (h === "localhost" || h === "127.0.0.1" || h.endsWith(".local") || h.endsWith(".lan")) return true;
    if (h.endsWith(".ts.net") || h.endsWith(".tailscale.net")) return true;
    // 100.64.0.0/10 Tailscale CGNAT
    const m = h.match(/^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/);
    if (m) {
      const b0 = parseInt(m[1], 10);
      const b1 = parseInt(m[2], 10);
      if (b0 === 100 && b1 >= 64 && b1 <= 127) return true; // Tailscale
      if (b0 === 10 || b0 === 127 || (b0 === 172 && b1 >= 16 && b1 <= 31) || (b0 === 192 && b1 === 168)) return true; // Private LAN
    }
    return false;
  };

  const isLanDiskPage = () => {
    return Boolean(
      window.FileBatchManager ||
      window.LanDiskUI ||
      window.LanDiskAuth ||
      window.FileExplorerComponent ||
      document.getElementById("apple-floating-batch-bar") ||
      document.querySelector(".apple-floating-batch-bar") ||
      document.title.includes("局域网互联") ||
      document.title.includes("LanDisk") ||
      document.querySelector("script[src*='file-batch']") ||
      (isTailscaleOrLanHost() && document.querySelector("meta[name='application-name'][content*='局域网互联']"))
    );
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

  // 1. 拦截 FileBatchManager.prototype.downloadZip
  function hookBatchManager() {
    if (!window.FileBatchManager || !window.FileBatchManager.prototype) return;
    if (window.FileBatchManager.prototype.__maobuHooked) return;
    window.FileBatchManager.prototype.__maobuHooked = true;

    const originalDownloadZip = window.FileBatchManager.prototype.downloadZip;

    window.FileBatchManager.prototype.downloadZip = function (customFilesArr = null, customFolderName = "batch_download") {
      let filesArr = customFilesArr || Array.from(this.selectedFiles || []);
      if (typeof filesArr === "string") filesArr = [filesArr];
      else if (filesArr instanceof Set) filesArr = Array.from(filesArr);
      else if (!Array.isArray(filesArr)) {
        try { filesArr = Array.from(filesArr); } catch { filesArr = [filesArr]; }
      }
      filesArr = filesArr.filter((f) => typeof f === "string" && f.trim());

      if (!filesArr || filesArr.length === 0) {
        return originalDownloadZip.call(this, customFilesArr, customFolderName);
      }

      // 计算文件名
      let resolvedFolderName = customFolderName;
      if (!resolvedFolderName || resolvedFolderName === "batch_download") {
        if (filesArr.length === 1) {
          resolvedFolderName = filesArr[0].split(/[\\/]/).filter(Boolean).pop() || "batch_download";
        } else {
          resolvedFolderName = `batch_download_${filesArr.length}_items`;
        }
      }

      const pin = (this.getPin ? this.getPin() : getPin()) || "";
      let authQ = "";
      if (window.LanDiskAuth && typeof window.LanDiskAuth.authQuery === "function") {
        const q = window.LanDiskAuth.authQuery();
        if (q) authQ = q.replace(/^\?/, "&");
      }
      if (pin && !authQ.includes("pin=")) {
        authQ += "&pin=" + encodeURIComponent(pin);
      }

      const getApiUrl = this.getApiUrl || ((u) => u);
      const apiUrl = getApiUrl("/api/download/batch") + (authQ ? ("?" + authQ.replace(/^&/, "")) : "");
      const fullUrl = new URL(apiUrl, window.location.href).href;

      const bodyParams = new URLSearchParams();
      bodyParams.append("folderName", resolvedFolderName);
      filesArr.forEach((f) => bodyParams.append("files", f));
      if (pin) bodyParams.append("pin", pin);

      try {
        if (chrome?.runtime?.id) {
          chrome.runtime.sendMessage({
            type: "send-landisk-batch-task",
            url: fullUrl,
            fileName: `${resolvedFolderName}.zip`,
            body: bodyParams.toString(),
            contentType: "application/x-www-form-urlencoded",
          }, (response) => {
            const err = chrome?.runtime?.lastError;
            if (!err && response && response.ok) {
              if (window.LanDiskUI && typeof window.LanDiskUI.toast === "function") {
                window.LanDiskUI.toast("⚡ 猫步下载器已接管打包下载", "success");
              }
            } else {
              // 桌面端未就绪或未配对，安全回退至原生表单下载
              originalDownloadZip.call(this, customFilesArr, customFolderName);
            }
          });
          return;
        }
      } catch {
        // 上下文失效，回退原生
      }

      return originalDownloadZip.call(this, customFilesArr, customFolderName);
    };
  }

  // 2. 兜底拦截 HTMLFormElement.prototype.submit（防范其他组件单独构建表单 POST /api/download/batch）
  function hookFormSubmit() {
    if (HTMLFormElement.prototype.__maobuHooked) return;
    HTMLFormElement.prototype.__maobuHooked = true;

    const originalSubmit = HTMLFormElement.prototype.submit;
    HTMLFormElement.prototype.submit = function () {
      try {
        const actionUrl = this.action || "";
        if (actionUrl.includes("/api/download/batch") && chrome?.runtime?.id) {
          const formData = new FormData(this);
          const bodyParams = new URLSearchParams();
          for (const [k, v] of formData.entries()) {
            bodyParams.append(k, v);
          }
          const folderName = formData.get("folderName") || "batch_download";
          chrome.runtime.sendMessage({
            type: "send-landisk-batch-task",
            url: new URL(actionUrl, window.location.href).href,
            fileName: `${folderName}.zip`,
            body: bodyParams.toString(),
            contentType: "application/x-www-form-urlencoded",
          }, (response) => {
            const err = chrome?.runtime?.lastError;
            if (err || !response || !response.ok) {
              originalSubmit.call(this);
            } else if (window.LanDiskUI && typeof window.LanDiskUI.toast === "function") {
              window.LanDiskUI.toast("⚡ 猫步下载器已接管打包下载", "success");
            }
          });
          return;
        }
      } catch {}
      return originalSubmit.call(this);
    };
  }

  // 3. 在 #apple-floating-batch-bar 注入“⚡ 猫步并发极速下载”按钮
  function injectTurboButton() {
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
    turboBtn.title = "【Tailscale远程加速首选】为所选文件建立多任务独立 HTTP Range 并发（每任务 16 连接），免除服务端压缩等待，打满链路带宽！";

    turboBtn.onclick = async () => {
      // 提取选中的文件集合
      let selectedPaths = [];
      if (window.FileExplorerComponent?.batchManager?.selectedFiles) {
        selectedPaths = Array.from(window.FileExplorerComponent.batchManager.selectedFiles);
      } else if (window.fileBatchManager?.selectedFiles) {
        selectedPaths = Array.from(window.fileBatchManager.selectedFiles);
      } else {
        document.querySelectorAll(".file-select-checkbox:checked, input[type='checkbox'][data-path]:checked").forEach((el) => {
          const p = el.getAttribute("data-path") || el.value;
          if (p) selectedPaths.push(p);
        });
      }

      selectedPaths = selectedPaths.filter(Boolean);
      if (selectedPaths.length === 0) {
        if (window.LanDiskUI?.toast) window.LanDiskUI.toast("请先勾选需要下载的文件", "warning");
        else alert("请先勾选需要下载的文件");
        return;
      }

      const authQ = getAuthQuery();
      const items = selectedPaths.map((p) => {
        const name = p.split(/[\\/]/).filter(Boolean).pop() || "file";
        const downloadPath = `/api/download?path=${encodeURIComponent(p)}${authQ ? (authQ.startsWith("?") ? authQ.replace(/^\?/, "&") : ("&" + authQ)) : ""}`;
        return {
          url: new URL(downloadPath, window.location.href).href,
          fileName: name,
        };
      });

      turboBtn.textContent = "正在下发…";
      try {
        chrome.runtime.sendMessage({
          type: "send-landisk-concurrent-files",
          items,
        }, (res) => {
          turboBtn.textContent = "⚡ 猫步并发极速下载";
          if (res && res.ok) {
            if (window.LanDiskUI?.toast) {
              window.LanDiskUI.toast(`已向猫步下载器添加 ${items.length} 个 16 线程极速任务！`, "success");
            }
          } else {
            const err = res?.error || "请确保猫步下载器已运行并完成配对";
            if (window.LanDiskUI?.toast) window.LanDiskUI.toast(err, "error");
            else alert(err);
          }
        });
      } catch {
        turboBtn.textContent = "⚡ 猫步并发极速下载";
      }
    };

    zipBtn.insertAdjacentElement("afterend", turboBtn);
  }

  // 4. 定时与 DOM 变动监听器
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

    // 周期性巡检（适应单页应用异步挂载）
    setInterval(() => {
      hookBatchManager();
      hookFormSubmit();
      injectTurboButton();
    }, 1500);
  };

  init();
})();
