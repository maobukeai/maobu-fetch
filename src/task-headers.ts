/**
 * 任务请求头中的临时认证辅助（安全相关纯函数）。
 *
 * - `hasTemporaryAuthHeaders`：判断任务是否携带临时登录态请求头（详情页提示徽标）。
 * - `stripCredentialHeaders`：剔除请求头中的 Cookie / Authorization 等认证头（保存任务模板时使用）。
 *
 * 所有判定统一"去首尾空白 + 转小写"后比较，避免 `Cookie` / `COOKIE` / ` authorization` 等
 * 变体漏判。依据 AGENTS.md §3：认证信息不得写入日志或持久化到模板等用户可见配置。
 */

/** 视为"临时登录态"的请求头（小写）。详情页据此显示"包含临时登录态"徽标。 */
const TEMP_AUTH_HEADER_NAMES: ReadonlySet<string> = new Set([
  "cookie",
  "authorization",
  "referer",
  "referrer",
  "user-agent",
]);

/** 视为"认证凭据"的请求头（小写）。保存模板等持久化场景必须剔除。 */
const CREDENTIAL_HEADER_NAMES: ReadonlySet<string> = new Set([
  "cookie",
  "authorization",
  "proxy-authorization",
]);

/** 请求头名称标准化：去首尾空白并转小写。 */
function normalizeHeaderName(name: string): string {
  return name.trim().toLowerCase();
}

/** 判断请求头名称是否为认证凭据头（大小写不敏感）。 */
export function isCredentialHeaderName(name: string): boolean {
  return CREDENTIAL_HEADER_NAMES.has(normalizeHeaderName(name));
}

/**
 * 判断任务是否携带临时登录态请求头
 * （Cookie / Authorization / Referer / User-Agent 等，大小写不敏感）。
 */
export function hasTemporaryAuthHeaders(
  headers?: Record<string, string> | null
): boolean {
  if (!headers) return false;
  return Object.keys(headers).some((name) =>
    TEMP_AUTH_HEADER_NAMES.has(normalizeHeaderName(name))
  );
}

/**
 * 剔除请求头中的认证凭据头（Cookie / Authorization / Proxy-Authorization，
 * 大小写不敏感），其余请求头原样保留。
 *
 * 输入为空或全部请求头都被剔除时返回 null，与 TaskTemplate.headers 的可空结构一致。
 * 不修改传入对象，返回新对象。
 */
export function stripCredentialHeaders(
  headers?: Record<string, string> | null
): Record<string, string> | null {
  if (!headers) return null;
  const result: Record<string, string> = {};
  for (const [name, value] of Object.entries(headers)) {
    if (isCredentialHeaderName(name)) continue;
    result[name] = value;
  }
  return Object.keys(result).length > 0 ? result : null;
}
