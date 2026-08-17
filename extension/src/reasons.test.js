import test from "node:test";
import assert from "node:assert/strict";
import { describeIgnoredReason, shortReasonLabel, SHORT_IGNORE_REASONS, IGNORE_REASONS } from "./reasons.js";

test("describeIgnoredReason: 常见原因映射为中文文案", () => {
  assert.match(describeIgnoredReason("size", 5), /5 MB/);
  assert.match(describeIgnoredReason("offline"), /桌面端离线/);
  assert.match(describeIgnoredReason("error:连接超时"), /连接超时/);
  assert.equal(describeIgnoredReason("unknown-reason"), "unknown-reason");
});

test("shortReasonLabel: error 前缀统一归为发送失败", () => {
  assert.equal(shortReasonLabel("error:ECONNREFUSED"), "发送失败");
  assert.equal(shortReasonLabel("error:任务参数无效"), "发送失败");
});

test("shortReasonLabel: 已知原因用短标签，未知原因原样返回", () => {
  assert.equal(shortReasonLabel("size"), "体积过小");
  assert.equal(shortReasonLabel("site-bypass"), "站点记忆放行");
  assert.equal(shortReasonLabel("other-extension"), "其它扩展发起");
  assert.equal(shortReasonLabel("some-new-reason"), "some-new-reason");
  assert.equal(shortReasonLabel(undefined), "未知原因");
});

test("SHORT_IGNORE_REASONS: 覆盖 IGNORE_REASONS 的全部已知原因", () => {
  // 新增 evaluateDownload reason 时必须同步两张表，这里强制保证不漏。
  for (const key of Object.keys(IGNORE_REASONS)) {
    assert.ok(key in SHORT_IGNORE_REASONS, `缺少短标签：${key}`);
  }
});
