export {};

function assertEqual<T>(actual: T, expected: T, msg?: string) {
  if (actual !== expected) {
    throw new Error(`断言失败: 实际值 ${JSON.stringify(actual)} !== 期望值 ${JSON.stringify(expected)}. ${msg || ""}`);
  }
}

function isPan123Url(url: string): boolean {
  if (!url) return false;
  const lower = url.toLowerCase();
  return (
    lower.includes("123pan.com") ||
    lower.includes("123pan.cn") ||
    lower.includes("123684.com") ||
    lower.includes("123952.com")
  );
}

function parsePan123Url(raw: string) {
  try {
    const parsed = new URL(raw);
    const host = parsed.hostname.toLowerCase();
    if (!isPan123Url(raw)) return null;

    const match = /(?:\/s\/|\/123pan\/|\/)([a-zA-Z0-9_-]+)/.exec(parsed.pathname);
    const shareKey = match ? match[1].replace(/\.html$/i, "") : "";
    const passCode = parsed.searchParams.get("pwd") || parsed.searchParams.get("p") || parsed.searchParams.get("passcode") || parsed.searchParams.get("SharePwd") || undefined;

    return { host, shareKey, passCode };
  } catch {
    return null;
  }
}

console.log("▶ 测试 isPan123Url...");
assertEqual(isPan123Url("https://www.123pan.com/s/Abcd-Efgh.html"), true);
assertEqual(isPan123Url("https://1683912.share.123pan.cn/123pan/z3h9-rtFzh?notoken=1"), true);
assertEqual(isPan123Url("https://123684.com/s/XYZ123?pwd=8888"), true);
assertEqual(isPan123Url("https://example.com/s/123"), false);
console.log("✔ isPan123Url 测试通过");

console.log("▶ 测试 parsePan123Url...");
const pan1 = parsePan123Url("https://www.123pan.com/s/Abcd-Efgh.html");
assertEqual(pan1?.host, "www.123pan.com");
assertEqual(pan1?.shareKey, "Abcd-Efgh");
assertEqual(pan1?.passCode, undefined);

const pan2 = parsePan123Url("https://1683912.share.123pan.cn/123pan/z3h9-rtFzh?notoken=1");
assertEqual(pan2?.host, "1683912.share.123pan.cn");
assertEqual(pan2?.shareKey, "z3h9-rtFzh");
assertEqual(pan2?.passCode, undefined);
console.log("✔ parsePan123Url 测试通过");
