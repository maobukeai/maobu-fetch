function equal(actual: string, expected: string): void { if (actual !== expected) throw new Error(`Expected ${expected}, got ${actual}`); }
function different(actual: string, expected: string): void { if (actual === expected) throw new Error(`Unexpected ${actual}`); }
import type { PlatformCompatibility } from "../../types";
import { compatibilityBadge } from "./platform-compatibility";

const item: PlatformCompatibility = { platform: "local-test", level: "verified", last_tested_at: "1970-01-01", verification: {
  app_version: "test-app-1", tool_version: "test-tool-1", sample: "local-fixture/1", scope: "download", result: "passed",
} };
equal(compatibilityBadge({ ...item, verification: null }, "test-app-1").label, "未验证");
equal(compatibilityBadge(item, "test-app-1").label, "下载样本已验证");
equal(compatibilityBadge(item, "test-app-2").label, "历史样本通过");
different(compatibilityBadge(item).label, "下载样本已验证");
equal(compatibilityBadge({ ...item, last_tested_at: "2025-02-29" }, "test-app-1").label, "证据不完整");
equal(compatibilityBadge({ ...item, verification: { ...item.verification!, sample: "" } }, "test-app-1").label, "证据不完整");
equal(compatibilityBadge({ ...item, verification: { ...item.verification!, scope: "metadata" } }, "test-app-1").label, "仅元数据解析通过");
equal(compatibilityBadge({ ...item, verification: { ...item.verification!, result: "failed" } }, "test-app-1").label, "验证未通过");
console.log("platform compatibility evidence display checks passed");
