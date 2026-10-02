// 猫步下载器 · 局域网互联 Pro / LanDisk 专属网页适配器
//
// 职责：
//   1. 识别局域网互联 (LanDisk) 网页环境（Tailscale 远程 / 局域网私网环境）；
//   2. 拦截并接管原生的 POST /api/download/batch 流式打包下载，
//      通过客户端递归文件树解构（Path A），将打包下载解构为独立文件的 16 线程 HTTP Range 直连，
//      保留完整子目录层级，彻底解决服务端动态 ZIP 流式打包无法断点续传（暂停清空为 0 字节）与速度慢的痛点；
//   3. 在悬浮操作条注入“⚡ 猫步并发极速下载”按钮，多文件/多目录自动递归展开满速并发；
//   4. 失败或桌面端离线时安全回退到网页原生行为，符合 AGENTS.md §5 规范。

(() => {
  if (typeof window === "undefined" && typeof globalThis === "undefined") return;
  const root = typeof window !== "undefined" ? window : globalThis;
  if (root.__maobuLanDiskInjected) return;
  root.__maobuLanDiskInjected = true;

  const isTailscaleOrLanHost = (hostname = "") => {
    const h = (hostname || (typeof window !== "undefined" ? window.location?.hostname : "") || "").toLowerCase().trim();
    if (h === "localhost" || h === "127.0.0.1" || h === "[::1]" || h === "::1" || h.endsWith(".local") || h.endsWith(".lan")) return true;
    if (h === "ts.net" || h.endsWith(".ts.net") || h === "tailscale.net" || h.endsWith(".tailscale.net")) return true;
    // 100.64.0.0/10 Tailscale CGNAT
    const m = h.match(/^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/);
    if (m) {
      const b0 = parseInt(m[1], 10);
      const b1 = parseInt(m[2], 10);
      if (b0 === 100 && b1 >= 64 && b1 <= 127) return true; // Tailscale
      if (b0 === 10 || b0 === 127 || (b0 === 172 && b1 >= 16 && b1 <= 31) || (b0 === 192 && b1 === 168)) return true; // Private LAN
    }
    const cleanH = h.replace(/^\[|\]$/g, "");
    if (cleanH === "::1" || cleanH.startsWith("fe80:") || cleanH.startsWith("fc") || cleanH.startsWith("fd")) {
      return true;
    }
    return false;
  };

  const isLanDiskPage = () => {
    if (typeof window === "undefined") return false;
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
      if (typeof window !== "undefined" && window.LanDiskAuth && typeof window.LanDiskAuth.getPin === "function") {
        return window.LanDiskAuth.getPin() || "";
      }
      if (typeof localStorage !== "undefined") {
        return localStorage.getItem("lan_disk_pin") || "";
      }
      return "";
    } catch {
      return "";
    }
  };

  const getAuthQuery = () => {
    try {
      let q = "";
      if (typeof window !== "undefined" && window.LanDiskAuth && typeof window.LanDiskAuth.authQuery === "function") {
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

  const extractLeafName = (p) => {
    const parts = String(p || "").replace(/\\/g, "/").split("/").filter(Boolean);
    return parts.pop() || "download";
  };

  const resolveFolderName = (filesArr, customFolderName) => {
    if (customFolderName && customFolderName !== "batch_download") {
      return customFolderName;
    }
    if (Array.isArray(filesArr) && filesArr.length === 1) {
      return extractLeafName(filesArr[0]);
    }
    return `batch_download_${Array.isArray(filesArr) ? filesArr.length : 0}_items`;
  };

  const buildBatchParams = (filesArr, folderName, pin) => {
    const bodyParams = new URLSearchParams();
    bodyParams.append("folderName", folderName);
    (filesArr || []).forEach((f) => bodyParams.append("files", f));
    if (pin) bodyParams.append("pin", pin);
    return bodyParams.toString();
  };

  const formatBytes = (value) => {
    if (!value || value <= 0) return "0 B";
    const units = ["B", "KB", "MB", "GB", "TB"];
    const index = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1);
    return `${(value / 1024 ** index).toFixed(index ? 1 : 0)} ${units[index]}`;
  };

  /**
   * 客户端递归文件树解构核心逻辑 (Path A)
   * 将选中的单文件/单目录/多选项目递归拆解为带相对子目录的独立下载直链列表
   */
  async function deconstructLanDiskSelection(filesArr, folderName, pin = "", fetchFn = null) {
    const activeFetch = fetchFn || (typeof window !== "undefined" ? window.fetch.bind(window) : fetch);
    const authQ = pin ? `?pin=${encodeURIComponent(pin)}` : "";
    const deconstructed = [];
    const isMultiple = Array.isArray(filesArr) && filesArr.length > 1;

    for (const itemPath of filesArr) {
      const itemLeaf = extractLeafName(itemPath);
      let probeUrl = `/api/files?path=${encodeURIComponent(itemPath)}${authQ ? authQ.replace(/^\?/, "&") : ""}`;
      if (typeof window !== "undefined" && window.location) {
        probeUrl = new URL(probeUrl, window.location.href).href;
      }

      let isDir = false;
      let dirItems = null;

      try {
        const resp = await activeFetch(probeUrl, {
          headers: { "Accept-Encoding": "identity", ...(pin ? { "x-pin": pin } : {}) },
        });
        if (resp && resp.ok) {
          const json = await resp.json();
          if (Array.isArray(json)) {
            isDir = true;
            dirItems = json;
          }
        }
      } catch {
        // 网络异常或非目录，按单文件处理
      }

      if (!isDir || !dirItems) {
        // 单文件直链
        let dlPath = `/api/download?path=${encodeURIComponent(itemPath)}${authQ ? (authQ.startsWith("?") ? authQ : ("&" + authQ)) : ""}`;
        if (typeof window !== "undefined" && window.location) {
          dlPath = new URL(dlPath, window.location.href).href;
        }
        deconstructed.push({
          name: itemLeaf,
          remotePath: itemPath,
          relativeDir: "",
          size: 0,
          downloadUrl: dlPath,
        });
        continue;
      }

      // 目录递归解构
      const queue = [
        {
          remotePath: itemPath,
          relDir: isMultiple ? itemLeaf : "",
          items: dirItems,
        },
      ];
      const visited = new Set([itemPath]);
      const MAX_FILES = 10000;
      const MAX_FOLDERS = 1000;
      let folderCount = 1;

      while (queue.length > 0 && deconstructed.length < MAX_FILES && folderCount < MAX_FOLDERS) {
        const current = queue.shift();
        const currentItems = current.items || [];

        for (const child of currentItems) {
          if (child.isDirectory) {
            folderCount++;
            if (!visited.has(child.path)) {
              visited.add(child.path);
              const nextRel = current.relDir ? `${current.relDir}/${child.name}` : child.name;
              try {
                let subUrl = `/api/files?path=${encodeURIComponent(child.path)}${authQ ? authQ.replace(/^\?/, "&") : ""}`;
                if (typeof window !== "undefined" && window.location) {
                  subUrl = new URL(subUrl, window.location.href).href;
                }
                const subResp = await activeFetch(subUrl, {
                  headers: { "Accept-Encoding": "identity", ...(pin ? { "x-pin": pin } : {}) },
                });
                if (subResp && subResp.ok) {
                  const subJson = await subResp.json();
                  if (Array.isArray(subJson)) {
                    queue.push({
                      remotePath: child.path,
                      relDir: nextRel,
                      items: subJson,
                    });
                  }
                }
              } catch {}
            }
          } else {
            let dlPath = `/api/download?path=${encodeURIComponent(child.path)}${authQ ? (authQ.startsWith("?") ? authQ : ("&" + authQ)) : ""}`;
            if (typeof window !== "undefined" && window.location) {
              dlPath = new URL(dlPath, window.location.href).href;
            }
            deconstructed.push({
              name: child.name,
              remotePath: child.path,
              relativeDir: current.relDir,
              size: child.size || 0,
              downloadUrl: dlPath,
            });
          }
        }
      }
    }

    return deconstructed;
  }

  // 0. 监听来自 Main World (landisk-injected.js) 的批量下载拦截事件
  if (typeof window !== "undefined" && isTailscaleOrLanHost()) {
    window.addEventListener("MAOBU_LANDISK_BATCH_INTERCEPT", async (event) => {
      const detail = event.detail || {};
      const filesArr = detail.files || [];
      if (!filesArr || filesArr.length === 0) return;

      const folderName = resolveFolderName(filesArr, detail.folderName);
      const pin = detail.pin || getPin();

      if (window.LanDiskUI && typeof window.LanDiskUI.toast === "function") {
        window.LanDiskUI.toast("⚡ 猫步下载器正在解构文件树…", "info");
      }

      try {
        const deconstructedFiles = await deconstructLanDiskSelection(filesArr, folderName, pin);
        if (!deconstructedFiles || deconstructedFiles.length === 0) {
          throw new Error("未解析到任何可下载的文件");
        }

        const totalBytes = deconstructedFiles.reduce((acc, f) => acc + (f.size || 0), 0);
        const sizeText = totalBytes > 0 ? `，总计 ${formatBytes(totalBytes)}` : "";

        if (chrome?.runtime?.id) {
          chrome.runtime.sendMessage({
            type: "send-landisk-deconstructed-tasks",
            folderName,
            batch_id: folderName,
            tasks: deconstructedFiles.map((f) => ({
              url: f.downloadUrl,
              fileName: f.name,
              destination: f.relativeDir ? `${folderName}/${f.relativeDir}` : folderName,
              connection_count: 16,
              total_bytes: f.size || undefined,
              size: f.size || undefined,
              batch_id: folderName,
            })),
          }, (response) => {
            const err = chrome?.runtime?.lastError;
            if (!err && response && response.ok) {
              if (window.LanDiskUI && typeof window.LanDiskUI.toast === "function") {
                window.LanDiskUI.toast(`⚡ 猫步极速下载已接管：共 ${deconstructedFiles.length} 个文件${sizeText}，正在极速并发传输！`, "success");
              }
            } else {
              window.dispatchEvent(new CustomEvent("MAOBU_LANDISK_FALLBACK_DOWNLOAD", { detail }));
            }
          });
        } else {
          window.dispatchEvent(new CustomEvent("MAOBU_LANDISK_FALLBACK_DOWNLOAD", { detail }));
        }
      } catch {
        window.dispatchEvent(new CustomEvent("MAOBU_LANDISK_FALLBACK_DOWNLOAD", { detail }));
      }
    });
  }

  // 1. 拦截 FileBatchManager.prototype.downloadZip
  function hookBatchManager() {
    if (typeof window === "undefined" || !window.FileBatchManager || !window.FileBatchManager.prototype) return;
    if (window.FileBatchManager.prototype.__maobuHooked) return;
    window.FileBatchManager.prototype.__maobuHooked = true;

    const originalDownloadZip = window.FileBatchManager.prototype.downloadZip;

    window.FileBatchManager.prototype.downloadZip = async function (customFilesArr = null, customFolderName = "batch_download") {
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

      const resolvedFolderName = resolveFolderName(filesArr, customFolderName);
      const pin = (this.getPin ? this.getPin() : getPin()) || "";

      // 优先走客户端任务树解构 (Path A)
      try {
        if (chrome?.runtime?.id) {
          if (window.LanDiskUI && typeof window.LanDiskUI.toast === "function") {
            window.LanDiskUI.toast("⚡ 猫步下载器正在解构文件树…", "info");
          }

          const deconstructedFiles = await deconstructLanDiskSelection(filesArr, resolvedFolderName, pin);
          if (deconstructedFiles && deconstructedFiles.length > 0) {
            const totalBytes = deconstructedFiles.reduce((acc, f) => acc + (f.size || 0), 0);
            const sizeText = totalBytes > 0 ? `，总计 ${formatBytes(totalBytes)}` : "";

            chrome.runtime.sendMessage({
              type: "send-landisk-deconstructed-tasks",
              folderName: resolvedFolderName,
              batch_id: resolvedFolderName,
              tasks: deconstructedFiles.map((f) => ({
                url: f.downloadUrl,
                fileName: f.name,
                destination: f.relativeDir ? `${resolvedFolderName}/${f.relativeDir}` : resolvedFolderName,
                connection_count: 16,
                total_bytes: f.size || undefined,
                size: f.size || undefined,
                batch_id: resolvedFolderName,
              })),
            }, (response) => {
              const err = chrome?.runtime?.lastError;
              if (!err && response && response.ok) {
                if (window.LanDiskUI && typeof window.LanDiskUI.toast === "function") {
                  window.LanDiskUI.toast(`⚡ 猫步极速下载已接管：共 ${deconstructedFiles.length} 个文件${sizeText}，正在极速并发传输！`, "success");
                }
              } else {
                // 桌面端未就绪或未配对，安全回退至原生下载
                originalDownloadZip.call(this, customFilesArr, customFolderName);
              }
            });
            return;
          }
        }
      } catch {
        // 解构异常，回退原生
      }

      return originalDownloadZip.call(this, customFilesArr, customFolderName);
    };
  }

  // 2. 兜底拦截 HTMLFormElement.prototype.submit（防范其他组件单独构建表单 POST /api/download/batch）
  function hookFormSubmit() {
    if (typeof HTMLFormElement === "undefined" || !HTMLFormElement.prototype) return;
    if (HTMLFormElement.prototype.__maobuHooked) return;
    HTMLFormElement.prototype.__maobuHooked = true;

    const originalSubmit = HTMLFormElement.prototype.submit;
    HTMLFormElement.prototype.submit = async function () {
      try {
        const actionUrl = this.action || "";
        if (actionUrl.includes("/api/download/batch") && chrome?.runtime?.id) {
          const formData = new FormData(this);
          const files = formData.getAll("files").map(String).filter(Boolean);
          const folderName = String(formData.get("folderName") || "batch_download");
          const pin = String(formData.get("pin") || getPin() || "");

          if (files.length > 0) {
            const deconstructedFiles = await deconstructLanDiskSelection(files, folderName, pin);
            if (deconstructedFiles && deconstructedFiles.length > 0) {
              const totalBytes = deconstructedFiles.reduce((acc, f) => acc + (f.size || 0), 0);
              const sizeText = totalBytes > 0 ? `，总计 ${formatBytes(totalBytes)}` : "";

              chrome.runtime.sendMessage({
                type: "send-landisk-deconstructed-tasks",
                folderName,
                batch_id: folderName,
                tasks: deconstructedFiles.map((f) => ({
                  url: f.downloadUrl,
                  fileName: f.name,
                  destination: f.relativeDir ? `${folderName}/${f.relativeDir}` : folderName,
                  connection_count: 16,
                  total_bytes: f.size || undefined,
                  size: f.size || undefined,
                  batch_id: folderName,
                })),
              }, (response) => {
                const err = chrome?.runtime?.lastError;
                if (!err && response && response.ok) {
                  if (window.LanDiskUI && typeof window.LanDiskUI.toast === "function") {
                    window.LanDiskUI.toast(`⚡ 猫步极速下载已接管：共 ${deconstructedFiles.length} 个文件${sizeText}，正在极速并发传输！`, "success");
                  }
                } else {
                  originalSubmit.call(this);
                }
              });
              return;
            }
          }
        }
      } catch {}
      return originalSubmit.call(this);
    };
  }

  // 3. 在 #apple-floating-batch-bar 注入“⚡ 猫步并发极速下载”按钮
  function injectTurboButton() {
    if (typeof document === "undefined") return;
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
    turboBtn.title = "【Tailscale远程加速首选】纯客户端递归解构文件树并建立多任务独立 HTTP Range 并发（每任务 16 连接），免除服务端压缩等待，打满链路带宽！";

    turboBtn.onclick = async () => {
      let selectedPaths = [];
      if (typeof window !== "undefined" && typeof window.__MAOBU_LANDISK_GET_SELECTED__ === "function") {
        selectedPaths = window.__MAOBU_LANDISK_GET_SELECTED__() || [];
      }
      if (!selectedPaths.length) {
        if (window.FileExplorerComponent?.batchManager?.selectedFiles) {
          selectedPaths = Array.from(window.FileExplorerComponent.batchManager.selectedFiles);
        } else if (window.fileBatchManager?.selectedFiles) {
          selectedPaths = Array.from(window.fileBatchManager.selectedFiles);
        } else {
          document.querySelectorAll(".file-select-checkbox:checked, .cb-file-select:checked, input[type='checkbox'][data-path]:checked").forEach((el) => {
            const p = el.getAttribute("data-path") || el.value;
            if (p) selectedPaths.push(p);
          });
        }
      }

      selectedPaths = selectedPaths.filter(Boolean);
      if (selectedPaths.length === 0) {
        if (window.LanDiskUI?.toast) window.LanDiskUI.toast("请先勾选需要下载的文件或目录", "warning");
        else alert("请先勾选需要下载的文件或目录");
        return;
      }

      const folderName = resolveFolderName(selectedPaths, "batch_turbo_download");
      const pin = getPin();

      turboBtn.textContent = "正在解构文件树…";
      turboBtn.disabled = true;

      try {
        const deconstructedFiles = await deconstructLanDiskSelection(selectedPaths, folderName, pin);
        if (!deconstructedFiles || deconstructedFiles.length === 0) {
          throw new Error("未解析到任何可下载的文件");
        }

        const totalBytes = deconstructedFiles.reduce((acc, f) => acc + (f.size || 0), 0);
        const sizeText = totalBytes > 0 ? `，总计 ${formatBytes(totalBytes)}` : "";

        turboBtn.textContent = "正在下发…";
        chrome.runtime.sendMessage({
          type: "send-landisk-deconstructed-tasks",
          folderName,
          batch_id: folderName,
          tasks: deconstructedFiles.map((f) => ({
            url: f.downloadUrl,
            fileName: f.name,
            destination: f.relativeDir ? `${folderName}/${f.relativeDir}` : folderName,
            connection_count: 16,
            total_bytes: f.size || undefined,
            size: f.size || undefined,
            batch_id: folderName,
          })),
        }, (res) => {
          turboBtn.textContent = "⚡ 猫步并发极速下载";
          turboBtn.disabled = false;
          if (res && res.ok) {
            if (window.LanDiskUI?.toast) {
              window.LanDiskUI.toast(`⚡ 猫步极速下载已接管：共 ${deconstructedFiles.length} 个文件${sizeText}，正在极速并发传输！`, "success");
            }
          } else {
            const err = res?.error || "请确保猫步下载器已运行并完成配对";
            if (window.LanDiskUI?.toast) window.LanDiskUI.toast(err, "error");
            else alert(err);
          }
        });
      } catch (e) {
        turboBtn.textContent = "⚡ 猫步并发极速下载";
        turboBtn.disabled = false;
        const msg = e?.message || "解构失败";
        if (window.LanDiskUI?.toast) window.LanDiskUI.toast(msg, "error");
        else alert(msg);
      }
    };

    zipBtn.insertAdjacentElement("afterend", turboBtn);
  }

  // 4. 定时与 DOM 变动监听器
  const observer = typeof MutationObserver !== "undefined"
    ? new MutationObserver(() => {
        hookBatchManager();
        hookFormSubmit();
        injectTurboButton();
      })
    : null;

  const init = () => {
    if (!isTailscaleOrLanHost()) return;
    hookBatchManager();
    hookFormSubmit();
    injectTurboButton();

    if (typeof document !== "undefined") {
      if (document.body && observer) {
        observer.observe(document.body, { childList: true, subtree: true });
      } else if (observer) {
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
      }, 1500);
    }
  };

  init();

  // 挂载公共方法供单元测试与外部调用
  const adapterExports = {
    init,
    isTailscaleOrLanHost,
    isLanDiskPage,
    extractLeafName,
    resolveFolderName,
    buildBatchParams,
    deconstructLanDiskSelection,
    hookBatchManager,
    hookFormSubmit,
    injectTurboButton,
    formatBytes,
  };
  root.MaobuLanDisk = adapterExports;
})();
