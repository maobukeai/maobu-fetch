/** 只识别抖音的具体作品页与分享短链；CDN 文件和其他域名继续普通下载。 */
export function isDouyinMediaPage(input: string): boolean {
  try {
    const url = new URL(input);
    if (url.protocol !== "https:" && url.protocol !== "http:") return false;
    const host = url.hostname.toLowerCase();
    if (host === "v.douyin.com") return true;
    if (["douyin.com", "www.douyin.com", "m.douyin.com"].includes(host)) {
      return /^\/(?:video|note)\/\d+(?:\/|$)/.test(url.pathname)
        || /^\d+$/.test(url.searchParams.get("modal_id") ?? "");
    }
    return ["iesdouyin.com", "www.iesdouyin.com"].includes(host)
      && /^\/share\/(?:video|note)\/\d+(?:\/|$)/.test(url.pathname);
  } catch {
    return false;
  }
}
