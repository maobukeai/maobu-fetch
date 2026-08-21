/**
 * PikPak 网盘免登录分享解析与直链获取服务。
 *
 * 遵循猫步下载器本地优先、无外部重依赖与零隐私收集规范：
 * 1. 优先调用 Rust 后端原生网络请求（零 CORS 跨域限制，支持桌面代理）；
 * 2. 纯本地自包含 MD5 算法，不引入重型三方库；
 * 3. 纯客户端直连官方开放 API，免登录获取公开/加密分享的文件树与 HTTP Range 下载直链；
 * 4. 严格错误处理，返回可读中文提示。
 */

import { api, isDesktop } from "../api";

// ==================== 纯 JS MD5 实现（标准 RFC 1321） ====================
function safeAdd(x: number, y: number): number {
  const lsw = (x & 0xffff) + (y & 0xffff);
  const msw = (x >> 16) + (y >> 16) + (lsw >> 16);
  return (msw << 16) | (lsw & 0xffff);
}

function bitRotateLeft(num: number, cnt: number): number {
  return (num << cnt) | (num >>> (32 - cnt));
}

function md5cmn(q: number, a: number, b: number, x: number, s: number, t: number): number {
  return safeAdd(bitRotateLeft(safeAdd(safeAdd(a, q), safeAdd(x, t)), s), b);
}

function md5ff(a: number, b: number, c: number, d: number, x: number, s: number, t: number): number {
  return md5cmn((b & c) | (~b & d), a, b, x, s, t);
}

function md5gg(a: number, b: number, c: number, d: number, x: number, s: number, t: number): number {
  return md5cmn((b & d) | (c & ~d), a, b, x, s, t);
}

function md5hh(a: number, b: number, c: number, d: number, x: number, s: number, t: number): number {
  return md5cmn(b ^ c ^ d, a, b, x, s, t);
}

function md5ii(a: number, b: number, c: number, d: number, x: number, s: number, t: number): number {
  return md5cmn(c ^ (b | ~d), a, b, x, s, t);
}

function binlMD5(x: number[], len: number): number[] {
  x[len >> 5] |= 0x80 << len % 32;
  x[(((len + 64) >>> 9) << 4) + 14] = len;

  let a = 1732584193;
  let b = -271733879;
  let c = -1732584194;
  let d = 271733878;

  for (let i = 0; i < x.length; i += 16) {
    const olda = a;
    const oldb = b;
    const oldc = c;
    const oldd = d;

    a = md5ff(a, b, c, d, x[i], 7, -680876936);
    d = md5ff(d, a, b, c, x[i + 1], 12, -389564586);
    c = md5ff(c, d, a, b, x[i + 2], 17, 606105819);
    b = md5ff(b, c, d, a, x[i + 3], 22, -1044525330);
    a = md5ff(a, b, c, d, x[i + 4], 7, -176418897);
    d = md5ff(d, a, b, c, x[i + 5], 12, 1200080426);
    c = md5ff(c, d, a, b, x[i + 6], 17, -1473231341);
    b = md5ff(b, c, d, a, x[i + 7], 22, -45705983);
    a = md5ff(a, b, c, d, x[i + 8], 7, 1770035416);
    d = md5ff(d, a, b, c, x[i + 9], 12, -1958414417);
    c = md5ff(c, d, a, b, x[i + 10], 17, -42063);
    b = md5ff(b, c, d, a, x[i + 11], 22, -1990404162);
    a = md5ff(a, b, c, d, x[i + 12], 7, 1804603682);
    d = md5ff(d, a, b, c, x[i + 13], 12, -40341101);
    c = md5ff(c, d, a, b, x[i + 14], 17, -1502002290);
    b = md5ff(b, c, d, a, x[i + 15], 22, 1236535329);

    a = md5gg(a, b, c, d, x[i + 1], 5, -165796510);
    d = md5gg(d, a, b, c, x[i + 6], 9, -1069501632);
    c = md5gg(c, d, a, b, x[i + 11], 14, 643717713);
    b = md5gg(b, c, d, a, x[i], 20, -373897302);
    a = md5gg(a, b, c, d, x[i + 5], 5, -701558691);
    d = md5gg(d, a, b, c, x[i + 10], 9, 38016083);
    c = md5gg(c, d, a, b, x[i + 15], 14, -660478335);
    b = md5gg(b, c, d, a, x[i + 4], 20, -405537848);
    a = md5gg(a, b, c, d, x[i + 9], 5, 568446438);
    d = md5gg(d, a, b, c, x[i + 14], 9, -1019803690);
    c = md5gg(c, d, a, b, x[i + 3], 14, -187363961);
    b = md5gg(b, c, d, a, x[i + 8], 20, 1163531501);
    a = md5gg(a, b, c, d, x[i + 13], 5, -1444681467);
    d = md5gg(d, a, b, c, x[i + 2], 9, -51403784);
    c = md5gg(c, d, a, b, x[i + 7], 14, 1735328473);
    b = md5gg(b, c, d, a, x[i + 12], 20, -1926607734);

    a = md5hh(a, b, c, d, x[i + 5], 4, -378558);
    d = md5hh(d, a, b, c, x[i + 8], 11, -2022574463);
    c = md5hh(c, d, a, b, x[i + 11], 16, 1839030562);
    b = md5hh(b, c, d, a, x[i + 14], 23, -35309556);
    a = md5hh(a, b, c, d, x[i + 1], 4, -1530992060);
    d = md5hh(d, a, b, c, x[i + 4], 11, 1272893353);
    c = md5hh(c, d, a, b, x[i + 7], 16, -155497632);
    b = md5hh(b, c, d, a, x[i + 10], 23, -1094730640);
    a = md5hh(a, b, c, d, x[i + 13], 4, 681279174);
    d = md5hh(d, a, b, c, x[i], 11, -358537222);
    c = md5hh(c, d, a, b, x[i + 3], 16, -722521979);
    b = md5hh(b, c, d, a, x[i + 6], 23, 76029189);
    a = md5hh(a, b, c, d, x[i + 9], 4, -640364487);
    d = md5hh(d, a, b, c, x[i + 12], 11, -421815835);
    c = md5hh(c, d, a, b, x[i + 15], 16, 530742520);
    b = md5hh(b, c, d, a, x[i + 2], 23, -995338651);

    a = md5ii(a, b, c, d, x[i], 6, -198630844);
    d = md5ii(d, a, b, c, x[i + 7], 10, 1126891415);
    c = md5ii(c, d, a, b, x[i + 14], 15, -1416354905);
    b = md5ii(b, c, d, a, x[i + 5], 21, -57434055);
    a = md5ii(a, b, c, d, x[i + 12], 6, 1700485571);
    d = md5ii(d, a, b, c, x[i + 3], 10, -1894986606);
    c = md5ii(c, d, a, b, x[i + 10], 15, -1051523);
    b = md5ii(b, c, d, a, x[i + 1], 21, -2054922799);
    a = md5ii(a, b, c, d, x[i + 8], 6, 1873313359);
    d = md5ii(d, a, b, c, x[i + 15], 10, -30611744);
    c = md5ii(c, d, a, b, x[i + 6], 15, -1560198380);
    b = md5ii(b, c, d, a, x[i + 13], 21, 1309151649);
    a = md5ii(a, b, c, d, x[i + 4], 6, -145523070);
    d = md5ii(d, a, b, c, x[i + 11], 10, -1120210379);
    c = md5ii(c, d, a, b, x[i + 2], 15, 718787259);
    b = md5ii(b, c, d, a, x[i + 9], 21, -343485551);

    a = safeAdd(a, olda);
    b = safeAdd(b, oldb);
    c = safeAdd(c, oldc);
    d = safeAdd(d, oldd);
  }
  return [a, b, c, d];
}

function str2binl(str: string): number[] {
  const bin: number[] = [];
  const mask = (1 << 8) - 1;
  for (let i = 0; i < str.length * 8; i += 8) {
    bin[i >> 5] |= (str.charCodeAt(i / 8) & mask) << i % 32;
  }
  return bin;
}

function binl2hex(binarray: number[]): string {
  const hexTab = "0123456789abcdef";
  let str = "";
  for (let i = 0; i < binarray.length * 4; i++) {
    str +=
      hexTab.charAt((binarray[i >> 2] >> ((i % 4) * 8 + 4)) & 0x0f) +
      hexTab.charAt((binarray[i >> 2] >> ((i % 4) * 8)) & 0x0f);
  }
  return str;
}

export function md5(str: string): string {
  return binl2hex(binlMD5(str2binl(str), str.length * 8));
}

// ==================== 常量与类型定义 ====================
export const PIKPAK_CLIENT_ID = "YNxT9w7GMdWvEOKa";
export const PIKPAK_CLIENT_VERSION = "1.0.0";
export const PIKPAK_PACKAGE_NAME = "mypikpak.com";
export const PIKPAK_API_HOST = "https://api-drive.mypikpak.com";
export const PIKPAK_USER_HOST = "https://user.mypikpak.com";

export interface PikPakParsedUrl {
  shareId: string;
  parentId?: string;
  passCode?: string;
  rawUrl: string;
}

export interface PikPakFileItem {
  id: string;
  name: string;
  kind: "drive#file" | "drive#folder";
  size: number;
  path: string; // 相对路径，如 "folder/sub/file.mp4"
  mime_type?: string;
  file_extension?: string;
  thumbnail_url?: string;
  created_time?: string;
  web_content_link?: string;
  medias?: Array<{
    media_name?: string;
    resolution_name?: string;
    link?: { url: string };
  }>;
}

export interface PikPakShareInfo {
  shareId: string;
  title: string;
  files: PikPakFileItem[];
  totalSize: number;
  fileCount: number;
  folderCount: number;
  passCodeRequired: boolean;
  passCodeToken?: string;
}

// ==================== 设备指纹管理 ====================
/**
 * 获取（或创建并固化）PikPak 设备指纹。
 * 创建任务时随 cloud_refresh 元数据一并提交给后端，
 * 直链过期自动刷新时复用同一指纹，降低风控概率。
 */
export function getOrCreateDeviceId(): string {
  const STORAGE_KEY = "maobu_pikpak_device_id";
  try {
    let id = localStorage.getItem(STORAGE_KEY);
    if (!id) {
      id = "mb_" + Math.random().toString(36).slice(2) + Date.now().toString(36);
      localStorage.setItem(STORAGE_KEY, id);
    }
    return id;
  } catch {
    return "mb_device_" + Math.random().toString(36).slice(2);
  }
}

// ==================== URL 匹配与解析 ====================
export function isPikPakShareUrl(url: string): boolean {
  if (!url || typeof url !== "string") return false;
  return /https?:\/\/(?:[a-zA-Z0-9-]+\.)?mypikpak\.(?:com|net)\/s\/([a-zA-Z0-9_-]+)/i.test(
    url.trim()
  );
}

export function parsePikPakShareUrl(rawText: string): PikPakParsedUrl | null {
  if (!rawText) return null;
  const text = rawText.trim();

  // 1. 匹配标准 URL 结构（包含可选的 query string）
  const match = text.match(
    /https?:\/\/(?:[a-zA-Z0-9-]+\.)?mypikpak\.(?:com|net)\/s\/([a-zA-Z0-9_-]+)(?:\/([a-zA-Z0-9_-]+))?(?:\?[^\s#]*)?/i
  );
  if (!match) return null;

  const shareId = match[1];
  const parentId = match[2] || undefined;

  // 2. 提取密码（支持 query param: ?pwd=xxx 或分享文本后缀: 提取码/密码 1234）
  let passCode: string | undefined = undefined;

  try {
    const urlObj = new URL(match[0]);
    const pwdParam =
      urlObj.searchParams.get("pwd") ||
      urlObj.searchParams.get("pass_code") ||
      urlObj.searchParams.get("code");
    if (pwdParam) passCode = pwdParam.trim();
  } catch {}

  if (!passCode) {
    const pwdMatch = text.match(/(?:提取码|密码|pwd|code)[:：\s]+([a-zA-Z0-9]{4,8})/i);
    if (pwdMatch) passCode = pwdMatch[1].trim();
  }

  return {
    shareId,
    parentId,
    passCode,
    rawUrl: match[0],
  };
}

// ==================== Captcha Token 初始化 ====================
let cachedCaptchaToken: { token: string; expireAt: number } | null = null;

async function getCaptchaToken(): Promise<string> {
  const now = Date.now();
  if (cachedCaptchaToken && cachedCaptchaToken.expireAt > now + 30_000) {
    return cachedCaptchaToken.token;
  }

  const deviceId = getOrCreateDeviceId();
  const timestamp = Math.floor(now / 1000).toString();

  // PikPak 签名算法：1. + MD5(client_id + client_version + package_name + device_id + timestamp + salt)
  const salt = "l-sark";
  const rawSign = `${PIKPAK_CLIENT_ID}${PIKPAK_CLIENT_VERSION}${PIKPAK_PACKAGE_NAME}${deviceId}${timestamp}${salt}`;
  const captchaSign = `1.${md5(rawSign)}`;

  const resp = await fetch(`${PIKPAK_USER_HOST}/v1/shield/captcha/init`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
    },
    body: JSON.stringify({
      client_id: PIKPAK_CLIENT_ID,
      device_id: deviceId,
      client_version: PIKPAK_CLIENT_VERSION,
      package_name: PIKPAK_PACKAGE_NAME,
      timestamp: Number(timestamp),
      captcha_sign: captchaSign,
      action: "POST:/share/v1/share/detail",
      meta: {
        phone_model: "Chrome/120.0.0.0",
      },
    }),
  });

  if (!resp.ok) {
    throw new Error(`初始化 PikPak 验证服务失败 (${resp.status}): ${resp.statusText}`);
  }

  const data = await resp.json();
  const token = data.captcha_token;
  if (!token) {
    throw new Error(data.error_description || "获取 PikPak 验证令牌失败");
  }

  cachedCaptchaToken = {
    token,
    expireAt: now + (data.expires_in || 3600) * 1000,
  };

  return token;
}

// ==================== 递归拉取目录树 ====================
async function fetchShareDirectoryLevel(
  shareId: string,
  parentId: string = "",
  passCode: string = "",
  passCodeToken: string = "",
  currentPath: string = "",
  visitedFolders: Set<string> = new Set()
): Promise<PikPakFileItem[]> {
  if (parentId && visitedFolders.has(parentId)) return [];
  if (parentId) visitedFolders.add(parentId);

  const captchaToken = await getCaptchaToken();
  const deviceId = getOrCreateDeviceId();
  let pageToken: string | undefined = undefined;
  const result: PikPakFileItem[] = [];

  do {
    const params = new URLSearchParams({
      share_id: shareId,
      limit: "100",
    });
    if (parentId) params.set("parent_id", parentId);
    if (pageToken) params.set("page_token", pageToken);
    if (passCodeToken) {
      params.set("pass_code_token", passCodeToken);
    } else if (passCode) {
      params.set("pass_code", passCode);
    }

    const endpoint = parentId ? `${PIKPAK_API_HOST}/drive/v1/share/detail` : `${PIKPAK_API_HOST}/drive/v1/share`;
    const resp = await fetch(
      `${endpoint}?${params.toString()}`,
      {
        headers: {
          "X-Client-Id": PIKPAK_CLIENT_ID,
          "X-Client-Version": PIKPAK_CLIENT_VERSION,
          "X-Device-Id": deviceId,
          "X-Captcha-Token": captchaToken,
          "Referer": "https://mypikpak.com/",
        },
      }
    );

    if (!resp.ok) {
      const errJson = await resp.json().catch(() => ({}));
      if (errJson.share_status === "PASS_CODE_EMPTY" || errJson.error === "need_pass_code" || resp.status === 403) {
        throw new Error("NEED_PASS_CODE");
      }
      if (errJson.share_status === "PASS_CODE_ERROR" || errJson.error === "invalid_pass_code") {
        throw new Error("提取码错误，请重新输入");
      }
      if (errJson.error_description) {
        throw new Error(errJson.error_description);
      }
      throw new Error(`拉取分享内容失败 (${resp.status})`);
    }

    const data = await resp.json();
    if (data.share_status === "PASS_CODE_EMPTY") {
      throw new Error("NEED_PASS_CODE");
    }
    if (data.share_status === "PASS_CODE_ERROR") {
      throw new Error("提取码错误，请重新输入");
    }

    const files = data.files || [];
    const nextToken = data.pass_code_token || passCodeToken;

    for (const item of files) {
      const itemPath = currentPath ? `${currentPath}/${item.name}` : item.name;
      const isFolder = item.kind === "drive#folder";

      const fileItem: PikPakFileItem = {
        id: item.id,
        name: item.name,
        kind: isFolder ? "drive#folder" : "drive#file",
        size: Number(item.size || 0),
        path: itemPath,
        mime_type: item.mime_type,
        file_extension: item.file_extension,
        thumbnail_url: item.thumbnail_link || item.icon_link,
        created_time: item.created_time,
        web_content_link: item.web_content_link,
        medias: item.medias,
      };

      if (!result.some((r) => r.id === fileItem.id)) {
        result.push(fileItem);
      }

      // 如果是子文件夹且未超出真实文件数量上限，递归拉取子项
      const currentFilesCount = result.filter((i) => i.kind === "drive#file").length;
      if (isFolder && !visitedFolders.has(item.id) && item.id !== parentId && currentFilesCount < 200) {
        const subItems = await fetchShareDirectoryLevel(
          shareId,
          item.id,
          passCode,
          nextToken,
          itemPath,
          visitedFolders
        );
        for (const sub of subItems) {
          if (!result.some((r) => r.id === sub.id)) {
            result.push(sub);
          }
        }
      }
      if (result.filter((i) => i.kind === "drive#file").length >= 200) break;
    }

    pageToken = data.next_page_token;
  } while (pageToken && result.filter((i) => i.kind === "drive#file").length < 200);

  return result;
}

// ==================== 主入口：解析 PikPak 分享 ====================
export async function inspectPikPakShare(
  rawUrl: string,
  providedPassCode?: string
): Promise<PikPakShareInfo> {
  const parsed = parsePikPakShareUrl(rawUrl);
  if (!parsed) {
    throw new Error("无效的 PikPak 分享链接，格式应为 https://mypikpak.com/s/xxxx");
  }

  // 桌面端优先走 Rust 原生网络栈（无 CORS 跨域限制，且遵循桌面代理配置）
  if (isDesktop()) {
    const deviceId = getOrCreateDeviceId();
    const res = await api.pikpakInspectShare({
      url: rawUrl,
      passCode: providedPassCode,
      deviceId,
    });
    return {
      shareId: res.shareId || (res as any).share_id || parsed.shareId,
      title: res.title || "PikPak 分享资源",
      files: (res.files || []).map((f: any) => ({
        id: f.id,
        name: f.name,
        kind: f.kind,
        size: Number(f.size || 0),
        path: f.path || f.name,
        mime_type: f.mime_type || f.mimeType,
        file_extension: f.file_extension || f.fileExtension,
        thumbnail_url: f.thumbnail_url || f.thumbnailUrl,
        web_content_link: f.web_content_link || f.webContentLink,
      })),
      totalSize: Number(res.totalSize ?? (res as any).total_size ?? 0),
      fileCount: Number(res.fileCount ?? (res as any).file_count ?? 0),
      folderCount: Number(res.folderCount ?? (res as any).folder_count ?? 0),
      passCodeRequired: Boolean(res.passCodeRequired ?? (res as any).pass_code_required),
      passCodeToken: res.passCodeToken || (res as any).pass_code_token,
    };
  }

  const effectivePassCode = providedPassCode || parsed.passCode || "";

  try {
    let allItems: PikPakFileItem[] = [];
    try {
      allItems = await fetchShareDirectoryLevel(
        parsed.shareId,
        parsed.parentId || "",
        effectivePassCode,
        "",
        ""
      );
    } catch (fetchErr: any) {
      if (fetchErr.message === "NEED_PASS_CODE") throw fetchErr;
      if (parsed.parentId) {
        allItems = await fetchShareDirectoryLevel(
          parsed.shareId,
          "",
          effectivePassCode,
          "",
          ""
        );
      } else {
        throw fetchErr;
      }
    }

    if (allItems.length === 0 && parsed.parentId) {
      allItems = await fetchShareDirectoryLevel(
        parsed.shareId,
        "",
        effectivePassCode,
        "",
        ""
      );
    }

    const onlyFiles = allItems.filter((i) => i.kind === "drive#file");
    const folderCount = allItems.filter((i) => i.kind === "drive#folder").length;
    const totalSize = onlyFiles.reduce((sum, f) => sum + f.size, 0);

    // 默认标题使用分享首项或首个文件夹名
    let title = onlyFiles[0]?.name || "PikPak 分享资源";
    if (allItems.length > 1) {
      const topFolder = allItems.find((i) => i.kind === "drive#folder" && !i.path.includes("/"));
      if (topFolder) title = topFolder.name;
    }

    return {
      shareId: parsed.shareId,
      title,
      files: allItems,
      fileCount: onlyFiles.length,
      folderCount,
      totalSize,
      passCodeRequired: false,
      passCodeToken: effectivePassCode,
    };
  } catch (err: any) {
    if (err.message === "NEED_PASS_CODE") {
      return {
        shareId: parsed.shareId,
        title: "加密分享链接",
        files: [],
        fileCount: 0,
        folderCount: 0,
        totalSize: 0,
        passCodeRequired: true,
      };
    }
    throw err;
  }
}

// ==================== 解析单文件直链 ====================
export async function resolvePikPakDirectUrl(
  shareId: string,
  fileId: string,
  passCodeToken?: string
): Promise<{ url: string; headers: Record<string, string> }> {
  // 桌面端优先走 Rust 原生网络栈
  if (isDesktop()) {
    const deviceId = getOrCreateDeviceId();
    return await api.pikpakResolveFile({
      shareId,
      fileId,
      passCodeToken,
      deviceId,
    });
  }

  const captchaToken = await getCaptchaToken();
  const deviceId = getOrCreateDeviceId();

  const params = new URLSearchParams({
    share_id: shareId,
    file_id: fileId,
  });
  if (passCodeToken) params.set("pass_code_token", passCodeToken);

  const resp = await fetch(
    `${PIKPAK_API_HOST}/drive/v1/share/file_info?${params.toString()}`,
    {
      headers: {
        "X-Client-Id": PIKPAK_CLIENT_ID,
        "X-Client-Version": PIKPAK_CLIENT_VERSION,
        "X-Device-Id": deviceId,
        "X-Captcha-Token": captchaToken,
        "Referer": "https://mypikpak.com/",
      },
    }
  );

  if (!resp.ok) {
    throw new Error(`获取文件直链失败 (${resp.status})`);
  }

  const data = await resp.json();
  const fileInfo = data.file_info || data.file;
  const directUrl =
    fileInfo?.medias?.[0]?.link?.url ||
    fileInfo?.web_content_link ||
    data.web_content_link;

  if (!directUrl) {
    throw new Error("PikPak 未返回该文件的有效下载直链");
  }

  return {
    url: directUrl,
    headers: {
      "User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
      "Referer": "https://mypikpak.com/",
    },
  };
}
