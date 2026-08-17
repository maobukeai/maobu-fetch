import test from "node:test";
import assert from "node:assert/strict";
import { pushIgnoredEntry, IGNORED_HISTORY_MAX } from "./ignored-history.js";

test("pushIgnoredEntry: 追加并保持最新 max 条（环形缓冲）", () => {
  const max = 3;
  let list = [];
  for (let i = 1; i <= 5; i += 1) {
    list = pushIgnoredEntry(list, { filename: `file${i}` }, max);
  }
  assert.deepEqual(list.map((entry) => entry.filename), ["file3", "file4", "file5"]);
});

test("pushIgnoredEntry: 不修改入参数组", () => {
  const original = [{ filename: "a" }];
  const next = pushIgnoredEntry(original, { filename: "b" }, 5);
  assert.equal(original.length, 1);
  assert.equal(next.length, 2);
});

test("pushIgnoredEntry: 非数组入参按空列表处理", () => {
  const next = pushIgnoredEntry(undefined, { filename: "a" }, 5);
  assert.deepEqual(next, [{ filename: "a" }]);
});

test("pushIgnoredEntry: 默认容量与导出常量一致", () => {
  let list = [];
  for (let i = 0; i < IGNORED_HISTORY_MAX + 5; i += 1) {
    list = pushIgnoredEntry(list, { index: i });
  }
  assert.equal(list.length, IGNORED_HISTORY_MAX);
  assert.equal(list.at(-1).index, IGNORED_HISTORY_MAX + 4);
  assert.equal(list[0].index, 5);
});
