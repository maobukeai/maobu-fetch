/**
 * 夸克网盘 (Quark Pan) 分享解析与直链获取服务。
 *
 * 遵循猫步下载器本地优先、无外部重依赖与零隐私收集规范：
 * 1. 优先调用 Rust 后端原生网络请求（零 CORS 跨域限制，支持桌面代理）；
 * 2. 纯客户端直连官方 API，解析公开/加密分享的文件树与 HTTP Range 下载直链；
 * 3. 严格错误处理，返回可读中文提示。
 */

import { api, isDesktop } from "../api";

export interface QuarkFileItem {
  id: string;
  name: string;
  kind: "drive#file" | "drive#folder" | string;
  size: number;
  path: string;
  share_fid_token?: string;
  mime_type?: string;
  file_extension?: string;
  thumbnail_url?: string;
  format_type?: string;
}

export interface QuarkShareInfo {
  pwd_id: string;
  title: string;
  files: QuarkFileItem[];
  total_size: number;
  file_count: number;
  folder_count: number;
  pass_code_required: boolean;
  stoken?: string;
}

export interface QuarkDirectUrlResult {
  url: string;
  headers: Record<string, string>;
}

export const QUARK_API_HOST = "https://drive.quark.cn";
export const QUARK_USER_AGENT =
  "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

/**
 * 判断文本是否包含夸克分享链接
 */
export function isQuarkUrl(url: string): boolean {
  if (!url || typeof url !== "string") return false;
  const trimmed = url.trim();
  return (
    /https?:\/\/(?:[a-zA-Z0-9-]+\.)?quark\.cn\/s\/[a-zA-Z0-9_-]+/i.test(trimmed)
  );
}

/**
 * 从 URL/文本中解析夸克分享 ID 和提取码
 */
export function parseQuarkUrl(raw: string): {
  pwdId: string;
  pdirFid?: string;
  passCode?: string;
} | null {
  const text = raw.trim();
  const match = text.match(
    /https?:\/\/(?:[a-zA-Z0-9-]+\.)?quark\.cn\/s\/([a-zA-Z0-9_-]+)(?:\/([a-zA-Z0-9_-]+))?(?:\?[^\s#]*)?/i
  );
  if (!match) return null;

  const pwdId = match[1];
  const pdirFid = match[2] || undefined;
  let passCode: string | undefined = undefined;

  // 1. 从 URL query 中获取
  try {
    const parsed = new URL(match[0]);
    passCode =
      parsed.searchParams.get("pwd") ||
      parsed.searchParams.get("pass_code") ||
      parsed.searchParams.get("code") ||
      parsed.searchParams.get("passcode") ||
      undefined;
  } catch {
    // 忽略
  }

  // 2. 从文本中抓取提取码
  if (!passCode) {
    const codeMatch = text.match(
      /(?:提取码|密码|pwd|code|passcode)[:：\s]+([a-zA-Z0-9]{4,8})/i
    );
    if (codeMatch) {
      passCode = codeMatch[1].trim();
    }
  }

  return { pwdId, pdirFid, passCode };
}

/**
 * 解析夸克分享（主入口）
 */
export async function inspectQuarkShare(
  url: string,
  passCode?: string,
  cookie?: string
): Promise<QuarkShareInfo> {
  const parsed = parseQuarkUrl(url);
  if (!parsed) {
    throw new Error("无效的夸克分享链接，格式应为 https://pan.quark.cn/s/xxxx");
  }

  const effectivePassCode = passCode || parsed.passCode;

  // 优先使用 Rust 后端
  if (isDesktop() && api.quarkInspectShare) {
    return await api.quarkInspectShare({
      url,
      pass_code: effectivePassCode,
      cookie,
    });
  }

  // 浏览器降级模式
  return await inspectQuarkShareBrowser(url, effectivePassCode, cookie);
}

/**
 * 获取单文件下载直链
 */
/**
 * 自动转存并获取转存后的 fid
 */
async function saveShareFileToDriveBrowser(
  pwdId: string,
  fid: string,
  shareFidToken: string | undefined,
  stoken: string,
  cookie: string
): Promise<string> {
  const payload: any = {
    fid_list: [fid],
    pwd_id: pwdId,
    stoken: stoken,
    to_pdir_fid: "0",
  };
  if (shareFidToken) {
    payload.fid_token_list = [shareFidToken];
  }

  const saveRes = await fetch(`${QUARK_API_HOST}/1/clouddrive/share/sharepage/save?pr=ucpro&fr=pc`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Cookie: cookie,
      Referer: "https://pan.quark.cn/",
      Origin: "https://pan.quark.cn",
      "User-Agent": QUARK_USER_AGENT,
    },
    body: JSON.stringify(payload),
  });
  const saveJson = await saveRes.json().catch(() => ({}));
  const taskId = saveJson?.data?.task_id || saveJson?.data?.task_id_str;

  if (taskId) {
    for (let retry = 0; retry < 12; retry++) {
      await new Promise((r) => setTimeout(r, 500));
      const pollRes = await fetch(
        `${QUARK_API_HOST}/1/clouddrive/task?task_id=${taskId}&retry_index=${retry}&pr=ucpro&fr=pc`,
        {
          headers: {
            Cookie: cookie,
            Referer: "https://pan.quark.cn/",
            "User-Agent": QUARK_USER_AGENT,
          },
        }
      );
      const pollJson = await pollRes.json().catch(() => ({}));
      const data = pollJson?.data;
      if (data) {
        for (const k of ["save_as_top_fids", "save_as_fids", "fids", "target_fids"]) {
          if (Array.isArray(data[k]) && data[k][0]) {
            return data[k][0];
          }
        }
        if (data.fid || data.file_id) return data.fid || data.file_id;
        if (Array.isArray(data.list) && data.list[0]?.fid) return data.list[0].fid;

        if (data.status === 2) {
          const sortRes = await fetch(
            `${QUARK_API_HOST}/1/clouddrive/file/sort?pdir_fid=0&_sort=created_at:desc&_page=1&_size=10&pr=ucpro&fr=pc`,
            {
              headers: {
                Cookie: cookie,
                Referer: "https://pan.quark.cn/",
                "User-Agent": QUARK_USER_AGENT,
              },
            }
          );
          const sortJson = await sortRes.json().catch(() => ({}));
          const list = sortJson?.data?.list;
          if (Array.isArray(list) && list[0]?.fid) {
            return list[0].fid;
          }
        }
      }
    }
  }

  throw new Error(saveJson?.message || "转存未能在网盘中检索到文件记录");
}

export async function resolveQuarkFile(
  pwdId: string,
  fid: string,
  shareFidToken?: string,
  stoken?: string,
  cookie?: string
): Promise<QuarkDirectUrlResult> {
  if (isDesktop() && api.quarkResolveFile) {
    return await api.quarkResolveFile({
      pwd_id: pwdId,
      fid,
      share_fid_token: shareFidToken,
      stoken,
      cookie,
    });
  }

  if (!cookie || !cookie.trim()) {
    throw new Error("下载夸克文件需要提供登录 Cookie，请在凭证库中保存或提供 Cookie");
  }

  const res = await fetch(`${QUARK_API_HOST}/1/clouddrive/file/download?pr=ucpro&fr=pc`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Cookie: cookie,
      Referer: "https://pan.quark.cn/",
      Origin: "https://pan.quark.cn",
      "User-Agent": QUARK_USER_AGENT,
    },
    body: JSON.stringify({
      fids: [fid],
      pwd_id: pwdId,
      stoken: stoken || "",
    }),
  });

  const json = await res.json().catch(() => ({}));
  const directUrl =
    json?.data?.download_url ||
    json?.data?.url ||
    (Array.isArray(json?.data) &&
      (typeof json?.data[0] === "string"
        ? json?.data[0]
        : json?.data[0]?.download_url || json?.data[0]?.url));

  if (!directUrl) {
    const code = json?.code;
    const errorMsg = json?.message || json?.msg || json?.error || "";

    if ((code === 23018 || errorMsg.includes("size limit")) && stoken) {
      try {
        const savedFid = await saveShareFileToDriveBrowser(
          pwdId,
          fid,
          shareFidToken,
          stoken,
          cookie
        );
        const saveDownRes = await fetch(
          `${QUARK_API_HOST}/1/clouddrive/file/download?pr=ucpro&fr=pc`,
          {
            method: "POST",
            headers: {
              "Content-Type": "application/json",
              Cookie: cookie,
              Referer: "https://pan.quark.cn/",
              Origin: "https://pan.quark.cn",
              "User-Agent": QUARK_USER_AGENT,
            },
            body: JSON.stringify({
              fids: [savedFid],
            }),
          }
        );
        const sJson = await saveDownRes.json().catch(() => ({}));
        const sDirectUrl =
          sJson?.data?.download_url ||
          sJson?.data?.url ||
          (Array.isArray(sJson?.data) &&
            (typeof sJson?.data[0] === "string"
              ? sJson?.data[0]
              : sJson?.data[0]?.download_url || sJson?.data[0]?.url));
        if (sDirectUrl) {
          return {
            url: sDirectUrl,
            headers: {
              "User-Agent": QUARK_USER_AGENT,
              Referer: "https://pan.quark.cn/",
              Cookie: cookie,
            },
          };
        }
      } catch (err: any) {
        throw new Error(`大文件免转存受限且自动转存失败: ${err.message || err}`);
      }
    }

    if (code === 31001 || errorMsg.includes("require login")) {
      throw new Error("夸克网盘凭证已过期或需要登录，请在扩展中同步 Cookie");
    } else if (code === 14001 || errorMsg.includes("非法token")) {
      throw new Error("夸克分享访问令牌已失效，请重新解析分享链接");
    }
    throw new Error(errorMsg ? `夸克直链获取失败: ${errorMsg}` : "夸克服务端未返回有效的下载地址");
  }

  return {
    url: directUrl,
    headers: {
      "User-Agent": QUARK_USER_AGENT,
      Referer: "https://pan.quark.cn/",
      Cookie: cookie,
    },
  };
}

/**
 * 浏览器端降级：解析夸克分享
 */
async function inspectQuarkShareBrowser(
  url: string,
  passCode?: string,
  cookie?: string
): Promise<QuarkShareInfo> {
  const parsed = parseQuarkUrl(url);
  if (!parsed) {
    throw new Error("无效的夸克分享链接");
  }

  // 1. 获取 stoken
  const tokenRes = await fetch(`${QUARK_API_HOST}/1/clouddrive/share/sharepage/token`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Referer: "https://pan.quark.cn/",
      ...(cookie ? { Cookie: cookie } : {}),
    },
    body: JSON.stringify({
      pwd_id: parsed.pwdId,
      ...(passCode ? { passcode: passCode } : {}),
    }),
  });

  const tokenJson = await tokenRes.json();
  const code = tokenJson?.code || tokenJson?.status || 0;

  if (code === 40010 || code === 40011 || tokenJson?.message?.includes("提取码错误")) {
    throw new Error("提取码错误，请重新输入");
  }
  if (code === 40008 || code === 40009 || tokenJson?.message?.includes("分享已失效")) {
    throw new Error("该夸克分享已失效或不存在");
  }

  const stoken = tokenJson?.data?.stoken;
  if (!stoken) {
    if (tokenJson?.message?.includes("密码") || tokenJson?.message?.includes("提取码")) {
      return {
        pwd_id: parsed.pwdId,
        title: "夸克加密分享",
        files: [],
        total_size: 0,
        file_count: 0,
        folder_count: 0,
        pass_code_required: true,
      };
    }
    throw new Error(tokenJson?.message || "未能获取夸克分享访问令牌");
  }

  // 2. 遍历目录
  const allItems: QuarkFileItem[] = [];
  const queue: Array<{ fid: string; path: string }> = [{ fid: "0", path: "" }];

  let folderVisits = 0;
  while (queue.length > 0) {
    const current = queue.shift()!;
    folderVisits++;
    if (folderVisits > 30 && allItems.filter((i) => i.kind === "drive#file").length > 0) {
      break;
    }

    let page = 1;
    while (true) {
      const searchParams = new URLSearchParams({
        pwd_id: parsed.pwdId,
        stoken: stoken,
        pdir_fid: current.fid,
        _page: String(page),
        _size: "100",
      });
      const detailRes = await fetch(
        `${QUARK_API_HOST}/1/clouddrive/share/sharepage/detail?${searchParams.toString()}`,
        {
          headers: {
            Referer: "https://pan.quark.cn/",
            ...(cookie ? { Cookie: cookie } : {}),
          },
        }
      );
      const detailJson = await detailRes.json();
      const list = detailJson?.data?.list || [];
      if (list.length === 0) break;

      for (const item of list) {
        const isDir =
          item.dir === true ||
          item.file === false ||
          item.is_dir ||
          item.obj_type === "dir" ||
          item.format_type === "dir";
        const itemPath = current.path ? `${current.path}/${item.file_name}` : item.file_name;
        const fileItem: QuarkFileItem = {
          id: item.fid,
          name: item.file_name,
          kind: isDir ? "drive#folder" : "drive#file",
          size: typeof item.size === "string" ? parseInt(item.size, 10) || 0 : item.size || 0,
          path: itemPath,
          share_fid_token: item.share_fid_token || item.fid_token,
          format_type: item.format_type,
          thumbnail_url: item.thumbnail || item.icon,
        };
        allItems.push(fileItem);

        if (isDir && allItems.filter((i) => i.kind === "drive#file").length < 200) {
          queue.unshift({ fid: item.fid, path: itemPath });
        }
      }

      const total = detailJson?.data?.total || 0;
      if (page * 100 >= total || allItems.filter((i) => i.kind === "drive#file").length >= 200) {
        break;
      }
      page++;
    }

    if (allItems.filter((i) => i.kind === "drive#file").length >= 200) {
      break;
    }
  }

  const filesCount = allItems.filter((i) => i.kind === "drive#file").length;
  const folderCount = allItems.filter((i) => i.kind === "drive#folder").length;
  const totalSize = allItems
    .filter((i) => i.kind === "drive#file")
    .reduce((sum, item) => sum + (item.size || 0), 0);

  return {
    pwd_id: parsed.pwdId,
    title: allItems[0]?.name || "夸克分享资源",
    files: allItems,
    total_size: totalSize,
    file_count: filesCount,
    folder_count: folderCount,
    pass_code_required: false,
    stoken,
  };
}
