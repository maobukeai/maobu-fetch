export {};

function assertEqual<T>(actual: T, expected: T, msg?: string) {
  if (actual !== expected) {
    throw new Error(`断言失败: 实际值 ${JSON.stringify(actual)} !== 期望值 ${JSON.stringify(expected)}. ${msg || ""}`);
  }
}

function isLanzouUrl(url: string): boolean {
  if (!url) return false;
  const lower = url.toLowerCase();
  return lower.includes("lanzou") || lower.includes("lanzo") || lower.includes("baidupan.com.lanzou");
}

function parseLanzouUrl(raw: string) {
  try {
    const parsed = new URL(raw);
    const host = parsed.hostname.toLowerCase();
    if (!isLanzouUrl(host)) return null;

    const parts = parsed.pathname.split("/").filter(Boolean);
    const shareId = parts[parts.length - 1] || "";
    const passCode = parsed.searchParams.get("pwd") || parsed.searchParams.get("p") || parsed.searchParams.get("passcode") || undefined;

    return { host, shareId, passCode };
  } catch {
    return null;
  }
}

console.log("▶ 测试 isLanzouUrl...");
assertEqual(isLanzouUrl("https://wwx.lanzoux.com/i123456"), true);
assertEqual(isLanzouUrl("https://www.lanzoui.com/u/yoyodadada"), true);
assertEqual(isLanzouUrl("https://wwe.lanzouy.com/b0xxxxxx?pwd=abcd"), true);
assertEqual(isLanzouUrl("https://example.com/file.zip"), false);
console.log("✔ isLanzouUrl 测试通过");

console.log("▶ 测试 parseLanzouUrl...");
const lanzou1 = parseLanzouUrl("https://wwx.lanzoux.com/i123456");
assertEqual(lanzou1?.host, "wwx.lanzoux.com");
assertEqual(lanzou1?.shareId, "i123456");
assertEqual(lanzou1?.passCode, undefined);

const lanzou2 = parseLanzouUrl("https://www.lanzoui.com/u/yoyodadada");
assertEqual(lanzou2?.host, "www.lanzoui.com");
assertEqual(lanzou2?.shareId, "yoyodadada");
assertEqual(lanzou2?.passCode, undefined);
console.log("✔ parseLanzouUrl 测试通过");
