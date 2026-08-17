/**
 * 窗口拖放新建任务（对标 IDM/FDM 拖放，2026-08-17）。
 *
 * 支持三类拖入物：
 * - `.torrent` 种子文件 → 打开新建对话框，内容以 base64 提交给 BT 内核；
 * - URL 文本（http/https/magnet）→ 预填新建对话框；
 * - 其他文件 → 明确提示不支持，不静默忽略。
 *
 * 纯函数模块：DataTransfer 的读取在调用方完成，这里只做解析与分类，
 * 测试见 `drag-drop.test.ts`。
 */

/** 单次拖入允许提取的最大 URL 数（与多行批量提交的上限保持同量级）。 */
export const MAX_DROPPED_URLS = 200;

/** 拖入 .torrent 的大小上限，与后端 `process::validate_torrent_bytes` 一致。 */
export const MAX_DROPPED_TORRENT_BYTES = 20 * 1024 * 1024;

/** 从拖入文本中提取可下载 URL（http/https/magnet），按行/空白切分。 */
export function extractDroppedUrls(text: string): string[] {
  if (!text) return [];
  const seen = new Set<string>();
  const urls: string[] = [];
  for (const token of text.split(/[\r\n\s]+/)) {
    const trimmed = token.trim();
    if (!/^(https?:\/\/\S+|magnet:\?\S+)$/i.test(trimmed)) continue;
    if (seen.has(trimmed)) continue;
    seen.add(trimmed);
    urls.push(trimmed);
    if (urls.length >= MAX_DROPPED_URLS) break;
  }
  return urls;
}

export interface DroppedFileClassification<T> {
  torrents: T[];
  rejected: T[];
}

/** 把拖入文件分为种子文件与不支持文件（按扩展名判断）。 */
export function classifyDroppedFiles<T extends { name: string }>(files: T[]): DroppedFileClassification<T> {
  const torrents: T[] = [];
  const rejected: T[] = [];
  for (const file of files) {
    if (/\.torrent$/i.test(file.name.trim())) {
      torrents.push(file);
    } else {
      rejected.push(file);
    }
  }
  return { torrents, rejected };
}

/** ArrayBuffer → STANDARD base64（分块转换，避免 String.fromCharCode 参数上限）。 */
export function arrayBufferToBase64(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer);
  let binary = "";
  const CHUNK = 0x8000;
  for (let offset = 0; offset < bytes.length; offset += CHUNK) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + CHUNK));
  }
  return btoa(binary);
}
