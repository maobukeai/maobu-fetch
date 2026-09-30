/**
 * 删除确认摘要纯函数单元测试（delete-summary.ts）。
 *
 * 覆盖：任务计数、本地文件计数与文件名样本上限、未完成任务计数。
 *
 * 与项目其他前端测试相同的极简断言运行器（AGENTS.md §8 不引入测试框架），
 * 通过 `npx tsx src/delete-summary.test.ts` 执行，挂载在 `pnpm run check`。
 */

declare const process: { exitCode: number; argv: string[] };

import {
  DELETE_CONFIRM_FILE_NAME_LIMIT,
  summarizeDeleteTargets,
  taskHasLocalFile,
  type DeleteSummaryTask,
} from "./delete-summary.js";

function assertEqual<T>(actual: T, expected: T, message: string): void {
  if (actual !== expected) {
    throw new Error(`${message}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
  }
}

function assertTrue(value: unknown, message: string): void {
  if (!value) {
    throw new Error(`${message}: expected truthy, got ${JSON.stringify(value)}`);
  }
}

type TestCase = { name: string; fn: () => void };
const tests: TestCase[] = [];
const test = (name: string, fn: () => void) => { tests.push({ name, fn }); };

/** 构造最小任务对象（仅摘要所需字段）。 */
function makeTask(overrides: Partial<DeleteSummaryTask> & { id: string }): DeleteSummaryTask {
  return {
    file_name: `${overrides.id}.bin`,
    status: "queued",
    downloaded_bytes: 0,
    ...overrides,
  };
}

// ===== taskHasLocalFile =====

test("已完成或已有分片数据的任务视为存在本地文件", () => {
  assertTrue(taskHasLocalFile(makeTask({ id: "a", status: "completed" })), "completed");
  assertTrue(taskHasLocalFile(makeTask({ id: "b", status: "paused", downloaded_bytes: 1024 })), "partial shards");
  assertTrue(!taskHasLocalFile(makeTask({ id: "c", status: "queued", downloaded_bytes: 0 })), "queued without data");
});

// ===== summarizeDeleteTargets =====

test("只统计任务集合命中的任务", () => {
  const tasks = [
    makeTask({ id: "a", status: "completed" }),
    makeTask({ id: "b", status: "completed" }),
  ];
  const summary = summarizeDeleteTargets(tasks, new Set(["a"]));
  assertEqual(summary.taskCount, 1, "taskCount");
  assertEqual(summary.fileCount, 1, "fileCount");
  assertEqual(summary.incompleteCount, 0, "incompleteCount");
  assertEqual(summary.fileNames.join(","), "a.bin", "fileNames");
});

test("fileCount 统计已完成与含分片数据的任务", () => {
  const tasks = [
    makeTask({ id: "a", status: "completed" }),
    makeTask({ id: "b", status: "paused", downloaded_bytes: 2048 }),
    makeTask({ id: "c", status: "downloading", downloaded_bytes: 0 }),
  ];
  const summary = summarizeDeleteTargets(tasks, new Set(["a", "b", "c"]));
  assertEqual(summary.fileCount, 2, "fileCount");
  assertEqual(summary.incompleteCount, 2, "incompleteCount (b + c)");
});

test("文件名样本不超过上限", () => {
  const ids = ["a", "b", "c", "d", "e"];
  const tasks = ids.map((id) => makeTask({ id, status: "completed" }));
  const summary = summarizeDeleteTargets(tasks, new Set(ids));
  assertEqual(summary.fileCount, 5, "fileCount");
  assertEqual(summary.fileNames.length, DELETE_CONFIRM_FILE_NAME_LIMIT, "fileNames capped");
  assertEqual(summary.fileNames.join(","), "a.bin,b.bin,c.bin", "first names kept");
});

test("空集合返回全零摘要", () => {
  const summary = summarizeDeleteTargets([], new Set());
  assertEqual(summary.taskCount, 0, "taskCount");
  assertEqual(summary.fileCount, 0, "fileCount");
  assertEqual(summary.fileNames.length, 0, "fileNames");
  assertEqual(summary.incompleteCount, 0, "incompleteCount");
});

function runAllTests(): void {
  let passed = 0;
  let failed = 0;
  const failures: string[] = [];
  for (const testCase of tests) {
    try {
      testCase.fn();
      passed += 1;
    } catch (error) {
      failed += 1;
      const message = error instanceof Error ? error.message : String(error);
      failures.push(`  ✗ ${testCase.name}: ${message}`);
    }
  }
  if (failed > 0) {
    console.error(`\nFailed ${failed} / ${tests.length} tests:`);
    for (const failure of failures) console.error(failure);
    process.exitCode = 1;
  } else {
    console.log(`\nPassed ${passed} / ${tests.length} tests.`);
  }
}

// 非 ASCII 路径下 import.meta.url 是百分号编码，必须 decodeURI 后再比较（见 url-sequence.test.ts）。
if (typeof process !== "undefined" && process.argv[1] && decodeURI(import.meta.url).endsWith(process.argv[1].replace(/\\/g, "/"))) {
  runAllTests();
}

export { runAllTests };
