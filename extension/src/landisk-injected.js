// 猫步下载器 · 局域网互联 Pro / LanDisk Main World 深度拦截器
//
// 职责：
//   1. 运行在页面宿主 JS 环境 (MAIN world) 中；
//   2. 拦截 FileBatchManager.prototype.downloadZip 与表单 POST /api/download/batch；
//   3. 将拦截到的批量文件/目录路径安全转发给 Isolated World 的 landisk-adapter.js，
//      由后者执行纯客户端递归解构 (Path A) 并转化为 16 线程 HTTP Range 直连任务；
//   4. 若桌面端离线或解构失败，接收回退信号执行原生下载，保证 0 破坏原有体验。

(() => {
  if (typeof window === "undefined") return;
  if (window.__MAOBU_LANDISK_INJECTED__) return;
  window.__MAOBU_LANDISK_INJECTED__ = true;

  const isTailscaleOrLanHost = () => {
    const h = (window.location?.hostname || "").toLowerCase().trim();
    if (h === "localhost" || h === "127.0.0.1" || h === "[::1]" || h === "::1" || h.endsWith(".local") || h.endsWith(".lan")) return true;
    if (h === "ts.net" || h.endsWith(".ts.net") || h === "tailscale.net" || h.endsWith(".tailscale.net")) return true;
    const m = h.match(/^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/);
    if (m) {
      const b0 = parseInt(m[1], 10);
      const b1 = parseInt(m[2], 10);
      if (b0 === 100 && b1 >= 64 && b1 <= 127) return true;
      if (b0 === 10 || b0 === 127 || (b0 === 172 && b1 >= 16 && b1 <= 31) || (b0 === 192 && b1 === 168)) return true;
    }
    const cleanH = h.replace(/^\[|\]$/g, "");
    if (cleanH === "::1" || cleanH.startsWith("fe80:") || cleanH.startsWith("fc") || cleanH.startsWith("fd")) {
      return true;
    }
    return false;
  };

  if (!isTailscaleOrLanHost()) return;

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

  // 暴露获取页面当前选中的文件列表给 Isolated World
  window.__MAOBU_LANDISK_GET_SELECTED__ = () => {
    try {
      if (window.FileExplorerComponent?.batchManager?.selectedFiles) {
        return Array.from(window.FileExplorerComponent.batchManager.selectedFiles);
      }
      if (window.fileBatchManager?.selectedFiles) {
        return Array.from(window.fileBatchManager.selectedFiles);
      }
    } catch {}
    return [];
  };

  let originalDownloadZip = null;
  function hookBatchManager() {
    if (!window.FileBatchManager || !window.FileBatchManager.prototype) return;
    if (window.FileBatchManager.prototype.__maobuInjectedHooked) return;
    window.FileBatchManager.prototype.__maobuInjectedHooked = true;

    originalDownloadZip = window.FileBatchManager.prototype.downloadZip;
    window.FileBatchManager.prototype.downloadZip = function (customFilesArr = null, customFolderName = "batch_download") {
      let filesArr = customFilesArr || (this.selectedFiles ? Array.from(this.selectedFiles) : []);
      if (typeof filesArr === "string") filesArr = [filesArr];
      else if (filesArr instanceof Set) filesArr = Array.from(filesArr);
      else if (!Array.isArray(filesArr)) {
        try { filesArr = Array.from(filesArr); } catch { filesArr = [filesArr]; }
      }
      filesArr = (filesArr || []).filter((f) => typeof f === "string" && f.trim());

      const pin = (this.getPin ? this.getPin() : (window.LanDiskAuth?.getPin ? window.LanDiskAuth.getPin() : "")) || "";

      window.dispatchEvent(
        new CustomEvent("MAOBU_LANDISK_BATCH_INTERCEPT", {
          detail: {
            files: filesArr,
            folderName: customFolderName,
            pin,
          },
        })
      );
    };
  }

  let originalFormSubmit = null;
  function hookFormSubmit() {
    if (typeof HTMLFormElement === "undefined" || !HTMLFormElement.prototype) return;
    if (HTMLFormElement.prototype.__maobuInjectedHooked) return;
    HTMLFormElement.prototype.__maobuInjectedHooked = true;

    originalFormSubmit = HTMLFormElement.prototype.submit;
    HTMLFormElement.prototype.submit = function () {
      const action = this.action || "";
      if (action.includes("/api/download/batch")) {
        const formData = new FormData(this);
        const files = formData.getAll("files").map(String).filter(Boolean);
        const folderName = String(formData.get("folderName") || "batch_download");
        const pin = String(formData.get("pin") || "");

        window.dispatchEvent(
          new CustomEvent("MAOBU_LANDISK_BATCH_INTERCEPT", {
            detail: {
              files,
              folderName,
              pin,
              isFormSubmit: true,
            },
          })
        );
        return;
      }
      return originalFormSubmit.call(this);
    };
  }

  // 监听回退信号：当 Isolated World 解构异常或桌面端离线时，优雅执行网页原生下载
  window.addEventListener("MAOBU_LANDISK_FALLBACK_DOWNLOAD", (e) => {
    const detail = e.detail || {};
    if (originalDownloadZip && window.FileBatchManager?.prototype) {
      originalDownloadZip.call(window.fileBatchManager || {}, detail.files, detail.folderName);
    }
  });

  const init = () => {
    if (!isTailscaleOrLanHost()) return;
    hookBatchManager();
    hookFormSubmit();
  };

  init();
  if (typeof document !== "undefined" && isTailscaleOrLanHost()) {
    document.addEventListener("DOMContentLoaded", init);
    setInterval(init, 1500);
  }
})();
