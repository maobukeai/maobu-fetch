import test from "node:test";
import assert from "node:assert/strict";
import { performBridgeTaskAction } from "./task-action.js";

test("Cancel uses native support; old desktop gets truthful pause fallback", async () => {
  const calls = [];
  const invoke = async (_path, body) => {
    calls.push(body.action);
    return { ok: true, json: async () => body.action === "cancel" ? { success: false, error: "不支持的操作: cancel" } : { success: true } };
  };
  assert.deepEqual(await performBridgeTaskAction("task", "cancel", invoke), { success: true, paused_instead: true });
  assert.deepEqual(calls, ["cancel", "pause"]);
  const supported = await performBridgeTaskAction("task", "cancel", async () => ({ ok: true, json: async () => ({ success: true }) }));
  assert.equal(supported.paused_instead, undefined);
});

test("Cancel must not hide an actual task error by reporting pause success", async () => {
  let calls = 0;
  const result = await performBridgeTaskAction("task", "cancel", async () => {
    calls += 1;
    return { ok: true, json: async () => ({ success: false, error: "任务已完成，不能取消" }) };
  });
  assert.equal(calls, 1);
  assert.equal(result.success, false);
});
