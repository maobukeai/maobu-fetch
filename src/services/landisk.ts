/**
 * 局域网互联 Pro / LanDisk 客户端文件树解构与极速下载服务。
 *
 * 核心原理（Path A: 纯客户端任务树解构）：
 * 1. 识别 LanDisk 网页或分享链接（局域网 / Tailscale CGNAT 100.64.0.0/10）；
 * 2. 避免使用服务端的 POST /api/download/batch 流式动态 ZIP 打包（无 Content-Length、不支持 HTTP 206 续传、易清空为 0 字节）；
 * 3. 递归探测目录树（/api/files?path=...），将目录或批量选择解构为独立的文件下载任务；
 * 4. 每个文件直接通过 GET /api/download?path=... 建立 16 线程 HTTP Range 206 并发直连，
 *    自动保留相对子目录结构（folder/sub/file.ext）；
 * 5. 全程零修改 LanDisk 服务端代码，符合 AGENTS.md 规范与极速下载要求。
 */

import { api, isDesktop } from "../api";
import type { LanDiskInspectionResult, LanDiskDeconstructedFile } from "../types";

export interface ParsedLanDiskUrl {
  baseUrl: string;
  targetPath?: string;
  folderName?: string;
  pin?: string;
  files: string[];
  isBatch: boolean;
}

/**
 * 判断主机名是否为 Tailscale CGNAT (100.64.0.0/10) 或局域网私网 IP / 本地主机。
 */
export function isTailscaleOrLanHost(hostname = ""): boolean {
  const h = String(hostname || "").toLowerCase().trim();
  if (
    h === "localhost" ||
    h === "127.0.0.1" ||
    h.endsWith(".local") ||
    h.endsWith(".lan") ||
    h.endsWith(".ts.net") ||
    h.endsWith(".tailscale.net")
  ) {
    return true;
  }
  const m = h.match(/^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/);
  if (m) {
    const b0 = parseInt(m[1], 10);
    const b1 = parseInt(m[2], 10);
    if (b0 === 100 && b1 >= 64 && b1 <= 127) return true; // Tailscale
    if (
      b0 === 10 ||
      b0 === 127 ||
      (b0 === 172 && b1 >= 16 && b1 <= 31) ||
      (b0 === 192 && b1 === 168)
    ) {
      return true; // LAN
    }
  }
  return false;
}

/**
 * 判断给定 URL 是否为局域网互联 (LanDisk) 链接。
 */
export function isLanDiskUrl(rawUrl: string): boolean {
  const trimmed = rawUrl.trim();
  if (!trimmed) return false;
  try {
    const parsed = new URL(trimmed);
    const pathname = parsed.pathname;
    const query = parsed.search.toLowerCase();
    const hash = parsed.hash.toLowerCase();

    // 明确的 LanDisk API 路径
    if (
      pathname.includes("/api/download/batch") ||
      pathname.includes("/api/download") ||
      pathname.includes("/api/files")
    ) {
      return true;
    }

    // 运行在私网/Tailscale 环境下的前端页面
    if (isTailscaleOrLanHost(parsed.hostname)) {
      if (
        query.includes("path=") ||
        query.includes("files=") ||
        hash.includes("path=") ||
        query.includes("pin=")
      ) {
        return true;
      }
      if (pathname === "/" || pathname === "") {
        return true;
      }
    }
  } catch {
    return false;
  }
  return false;
}

/**
 * 解析局域网互联 URL，提取 baseUrl、targetPath、folderName、pin 与批量文件列表。
 */
export function parseLanDiskUrl(rawUrl: string): ParsedLanDiskUrl | null {
  const trimmed = rawUrl.trim();
  if (!trimmed) return null;
  try {
    const parsed = new URL(trimmed);
    const baseUrl = `${parsed.protocol}//${parsed.host}`;
    const pathname = parsed.pathname;
    const isBatch = pathname.includes("/api/download/batch");

    let targetPath: string | undefined = undefined;
    let folderName: string | undefined = undefined;
    let pin: string | undefined = undefined;
    const files: string[] = [];

    // 从 Query 参数提取
    for (const [k, v] of parsed.searchParams.entries()) {
      const kl = k.toLowerCase();
      if (kl === "path" && !targetPath && v) {
        targetPath = v;
      } else if ((kl === "foldername" || kl === "name") && !folderName && v) {
        folderName = v;
      } else if ((kl === "pin" || kl === "token") && !pin && v) {
        pin = v;
      } else if ((kl === "files" || kl === "files[]") && v) {
        files.push(v);
      }
    }

    // 从 Hash fragment 提取（SPA 路由支持：#/files?path=...）
    if (parsed.hash && parsed.hash.includes("?")) {
      const hashParams = new URLSearchParams(parsed.hash.substring(parsed.hash.indexOf("?")));
      for (const [k, v] of hashParams.entries()) {
        const kl = k.toLowerCase();
        if (kl === "path" && !targetPath && v) {
          targetPath = v;
        } else if ((kl === "pin" || kl === "token") && !pin && v) {
          pin = v;
        }
      }
    }

    if (!targetPath && files.length === 0) {
      return null;
    }

    return {
      baseUrl,
      targetPath,
      folderName,
      pin,
      files,
      isBatch,
    };
  } catch {
    return null;
  }
}

/**
 * 构造 LanDisk 单文件下载直链（支持 Range 并发分片）。
 */
export function buildLanDiskDownloadUrl(baseUrl: string, remotePath: string, pin?: string): string {
  const base = baseUrl.replace(/\/+$/, "");
  const params = new URLSearchParams();
  params.set("path", remotePath);
  if (pin && pin.trim()) {
    params.set("pin", pin.trim());
  }
  return `${base}/api/download?${params.toString()}`;
}

/**
 * 提取路径最后一个节点作为文件夹或文件名。
 */
export function extractLeafName(filePath: string): string {
  const parts = filePath.replace(/\\/g, "/").split("/").filter(Boolean);
  return parts.pop() || "download";
}

/**
 * 探测并解构 LanDisk URL。在桌面环境下直接通过 Rust 后端调用，无跨域且支持代理。
 */
export async function inspectLanDiskShare(url: string, pin?: string): Promise<LanDiskInspectionResult> {
  if (isDesktop()) {
    return api.landiskInspect(url, pin);
  }

  // 纯浏览器环境/测试环境回退降级实现
  const parsed = parseLanDiskUrl(url);
  if (!parsed) {
    throw new Error("无效的局域网互联 URL");
  }

  const effectivePin = pin || parsed.pin;
  const leaf = parsed.targetPath ? extractLeafName(parsed.targetPath) : (parsed.folderName || "batch_download");

  // 如果指定了单文件或单目录
  if (parsed.targetPath) {
    const filesUrl = new URL(`${parsed.baseUrl}/api/files`);
    filesUrl.searchParams.set("path", parsed.targetPath);
    if (effectivePin) filesUrl.searchParams.set("pin", effectivePin);

    try {
      const resp = await fetch(filesUrl.toString());
      if (resp.status === 400) {
        // 单文件
        const downloadUrl = buildLanDiskDownloadUrl(parsed.baseUrl, parsed.targetPath, effectivePin);
        return {
          rootName: leaf,
          totalSize: 0,
          fileCount: 1,
          folderCount: 0,
          files: [
            {
              name: leaf,
              remotePath: parsed.targetPath,
              relativeDir: "",
              size: 0,
              downloadUrl,
            },
          ],
        };
      }
      if (!resp.ok) {
        throw new Error(`HTTP ${resp.status}`);
      }
      const items: Array<{ name: string; path: string; size?: number; isDirectory?: boolean }> = await resp.json();
      const files: LanDiskDeconstructedFile[] = [];
      let totalSize = 0;
      let folderCount = 0;

      for (const item of items) {
        if (item.isDirectory) {
          folderCount++;
        } else {
          const sz = Number(item.size || 0);
          totalSize += sz;
          files.push({
            name: item.name,
            remotePath: item.path,
            relativeDir: "",
            size: sz,
            downloadUrl: buildLanDiskDownloadUrl(parsed.baseUrl, item.path, effectivePin),
          });
        }
      }

      return {
        rootName: leaf,
        totalSize,
        fileCount: files.length,
        folderCount,
        files,
      };
    } catch {
      // 无法访问探测接口时作为单文件直链回退
      const downloadUrl = buildLanDiskDownloadUrl(parsed.baseUrl, parsed.targetPath, effectivePin);
      return {
        rootName: leaf,
        totalSize: 0,
        fileCount: 1,
        folderCount: 0,
        files: [
          {
            name: leaf,
            remotePath: parsed.targetPath,
            relativeDir: "",
            size: 0,
            downloadUrl,
          },
        ],
      };
    }
  }

  // 批量文件列表场景
  if (parsed.files.length > 0) {
    const files: LanDiskDeconstructedFile[] = parsed.files.map((p) => {
      const name = extractLeafName(p);
      return {
        name,
        remotePath: p,
        relativeDir: "",
        size: 0,
        downloadUrl: buildLanDiskDownloadUrl(parsed.baseUrl, p, effectivePin),
      };
    });
    return {
      rootName: parsed.folderName || `batch_${files.length}_items`,
      totalSize: 0,
      fileCount: files.length,
      folderCount: 0,
      files,
    };
  }

  throw new Error("URL 中未包含任何待下载的路径或文件");
}
