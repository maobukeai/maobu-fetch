/**
 * 百度网盘 (Baidu Pan / Baidu Netdisk) 分享解析与直链获取服务。
 *
 * 遵循猫步下载器本地优先、无外部重依赖与零隐私收集规范：
 * 1. 优先调用 Rust 后端原生网络请求（零 CORS 跨域限制，支持桌面代理）；
 * 2. 纯客户端直连官方 API，解析公开/加密分享的文件树与 HTTP Range 下载直链；
 * 3. 严格错误处理，返回可读中文提示。
 */

import { api, isDesktop } from "../api";

export interface BaiduFileItem {
  id: string; // fs_id
  name: string;
  kind: "drive#file" | "drive#folder" | string;
  size: number;
  path: string;
  md5?: string;
  category?: number;
}

export interface BaiduShareInfo {
  surl: string;
  shareId?: string;
  share_id?: string;
  uk?: string;
  title: string;
  files: BaiduFileItem[];
  totalSize?: number;
  total_size?: number;
  fileCount?: number;
  file_count?: number;
  folderCount?: number;
  folder_count?: number;
  passCodeRequired?: boolean;
  pass_code_required?: boolean;
  randsk?: string;
  sign?: string;
  timestamp?: number;
  seckey?: string;
}

export interface BaiduDirectUrlResult {
  url: string;
  headers: Record<string, string>;
}

export const BAIDU_API_HOST = "https://pan.baidu.com";
export const BAIDU_DLINK_USER_AGENT = "pan.baidu.com";

/**
 * 判断文本是否包含百度网盘分享链接
 */
export function isBaiduUrl(url: string): boolean {
  if (!url || typeof url !== "string") return false;
  const trimmed = url.trim();
  return (
    /https?:\/\/(?:pan|yun)\.baidu\.com\/(?:s\/|share\/init\?surl=)[a-zA-Z0-9_-]+/i.test(
      trimmed
    )
  );
}

/**
 * 从 URL/文本中解析百度网盘 surl 和提取码
 */
export function parseBaiduUrl(raw: string): {
  surl: string;
  passCode?: string;
} | null {
  if (!raw || typeof raw !== "string") return null;
  const trimmed = raw.trim();

  const surlMatch = trimmed.match(
    /https?:\/\/(?:pan|yun)\.baidu\.com\/(?:s\/1?([a-zA-Z0-9_-]+)|share\/init\?surl=1?([a-zA-Z0-9_-]+))/i
  );
  if (!surlMatch) return null;

  const surl = surlMatch[1] || surlMatch[2];
  if (!surl) return null;

  const codeMatch = trimmed.match(
    /(?:pwd|code|提取码|密码)[：:\s=]*([a-zA-Z0-9]{4})/i
  );
  const passCode = codeMatch ? codeMatch[1] : undefined;

  return { surl, passCode };
}

/**
 * 解析百度网盘分享信息与目录树
 */
export async function inspectBaiduShare(
  url: string,
  passCode?: string,
  cookie?: string
): Promise<BaiduShareInfo> {
  if (isDesktop() && api.baidupanInspectShare) {
    return await api.baidupanInspectShare({
      url,
      pass_code: passCode,
      cookie,
    });
  }

  throw new Error("请在猫步下载器桌面客户端中使用百度网盘分享解析功能");
}

/**
 * 获取单文件下载直链
 */
export async function resolveBaiduFile(
  surl: string,
  fsId: string,
  shareId?: string,
  uk?: string,
  sign?: string,
  timestamp?: number,
  seckey?: string,
  randsk?: string,
  cookie?: string
): Promise<BaiduDirectUrlResult> {
  if (isDesktop() && api.baidupanResolveFile) {
    return await api.baidupanResolveFile({
      surl,
      fs_id: fsId,
      share_id: shareId,
      uk,
      sign,
      timestamp,
      seckey,
      randsk,
      cookie,
    });
  }

  throw new Error("请在猫步下载器桌面客户端中使用百度网盘直链下载功能");
}
