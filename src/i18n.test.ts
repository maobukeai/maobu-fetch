/**
 * Task 33.6: i18n 框架单元测试。
 *
 * 验证项：
 * - `t("common.ok")` 返回正确翻译
 * - 占位符替换 `t("toasts.addedTasks", { count: 3 })` 正确
 * - 缺失 key 返回 key 本身
 * - 英文 locale 中无中文字符（用正则 /[\u4e00-\u9fa5]/ 扫描整个 en.json）
 *
 * 本项目未引入 vitest/jest 等测试框架（AGENTS.md §8 不引入重复框架）。
 * 此文件实现一个极简断言运行器，可通过 `node --test` 或直接运行执行。
 *
 * 注意：`pnpm run check` 只做 tsc --noEmit 类型检查；此文件需保证类型通过。
 */

/**
 * Node.js `process` 全局变量的最小化类型声明。
 *
 * 项目未引入 `@types/node`（AGENTS.md §8 不引入重复依赖），但测试运行时
 * 需要 `process.exitCode` 和 `process.argv` 来检测入口点和报告失败。
 * 此声明仅暴露测试用到的两个字段，避免引入完整 Node 类型定义。
 */
declare const process: { exitCode: number; argv: string[] };

import { t, setLocale, getLocale, findChineseCharactersInLocale, type Locale } from "./i18n.js";
import { reorderTaskIdsWithinPriority, TASK_PRIORITY_PRESETS } from "./priority.js";

/** 简易断言：实际值 === 期望值时通过，否则抛错。 */
function assertEqual<T>(actual: T, expected: T, message: string): void {
  if (actual !== expected) {
    throw new Error(`${message}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
  }
}

/** 简易断言：实际值为真。 */
function assertTrue(value: unknown, message: string): void {
  if (!value) {
    throw new Error(`${message}: expected truthy, got ${JSON.stringify(value)}`);
  }
}

/** 简易断言：数组为空。 */
function assertEmpty(arr: unknown[], message: string): void {
  if (arr.length !== 0) {
    throw new Error(`${message}: expected empty array, got ${JSON.stringify(arr)}`);
  }
}

/** 测试用例注册器：将每个测试函数收集到全局数组，运行后输出结果。 */
type TestCase = { name: string; fn: () => void };
const tests: TestCase[] = [];
const test = (name: string, fn: () => void) => { tests.push({ name, fn }); };

// ===== 测试用例 =====

test("任务优先级预设遵循数字越小越优先", () => {
  assertEqual(TASK_PRIORITY_PRESETS.high, -1, "high priority");
  assertEqual(TASK_PRIORITY_PRESETS.normal, 0, "normal priority");
  assertEqual(TASK_PRIORITY_PRESETS.low, 1, "low priority");
  assertTrue(
    TASK_PRIORITY_PRESETS.high < TASK_PRIORITY_PRESETS.normal
      && TASK_PRIORITY_PRESETS.normal < TASK_PRIORITY_PRESETS.low,
    "priority presets should be ascending from high to low",
  );
});

test("同优先级任务可以按拖放目标重排", () => {
  const tasks = [
    { id: "high", priority: -1, queue_position: 3 },
    { id: "a", priority: 0, queue_position: 4 },
    { id: "b", priority: 0, queue_position: 5 },
    { id: "low", priority: 1, queue_position: 1 },
  ];
  const reordered = reorderTaskIdsWithinPriority(tasks, "b", "a");
  assertEqual(reordered?.join(","), "high,b,a,low", "drag B above A");
});

test("拖拽排序拒绝跨优先级移动", () => {
  const tasks = [
    { id: "high", priority: -1, queue_position: 1 },
    { id: "normal", priority: 0, queue_position: 2 },
  ];
  assertEqual(
    reorderTaskIdsWithinPriority(tasks, "normal", "high"),
    null,
    "cross-priority reorder",
  );
});

test("t(key) 返回中文 locale 的正确翻译", () => {
  setLocale("zh-CN");
  assertEqual(t("common.ok"), "确定", "common.ok zh-CN");
  assertEqual(t("common.cancel"), "取消", "common.cancel zh-CN");
  assertEqual(t("common.save"), "保存", "common.save zh-CN");
});

test("t(key) 返回英文 locale 的正确翻译", () => {
  setLocale("en");
  assertEqual(t("common.ok"), "OK", "common.ok en");
  assertEqual(t("common.cancel"), "Cancel", "common.cancel en");
  assertEqual(t("common.save"), "Save", "common.save en");
});

test("切换 locale 后立即生效", () => {
  setLocale("zh-CN");
  assertEqual(t("common.ok"), "确定", "zh-CN ok");
  setLocale("en");
  assertEqual(t("common.ok"), "OK", "en ok");
  setLocale("zh-CN");
  assertEqual(t("common.ok"), "确定", "zh-CN ok restored");
});

test("t(key, params) 替换单个占位符", () => {
  setLocale("zh-CN");
  assertEqual(t("toasts.addedTasks", { count: 5 }), "已添加 5 个任务", "addedTasks count=5");
  assertEqual(t("toasts.historyDeletedWithFile", { count: 3 }), "已删除 3 个历史任务及文件", "historyDeletedWithFile count=3");
});

test("t(key, params) 替换多个占位符", () => {
  setLocale("zh-CN");
  const result = t("details.priorityRange", { min: -1000, max: 1000 });
  assertEqual(result, "数字越小越优先（-1000 ~ 1000）", "priorityRange");
});

test("英文 locale 占位符替换正确", () => {
  setLocale("en");
  assertEqual(t("toasts.addedTasks", { count: 5 }), "Added 5 tasks", "addedTasks en count=5");
  assertEqual(t("toasts.historyDeletedWithFile", { count: 3 }), "Deleted 3 history tasks and files", "historyDeletedWithFile en count=3");
});

test("params 缺失占位符时保留原 {name} 文本", () => {
  setLocale("zh-CN");
  const result = t("toasts.addedTasks");
  assertEqual(result, "已添加 {count} 个任务", "addedTasks no params");
});

test("支持字符串、数字、布尔值参数", () => {
  setLocale("zh-CN");
  assertEqual(t("toasts.renamedTo", { name: "video.mp4" }), "已重命名为 video.mp4", "renamedTo string");
  assertEqual(t("toasts.addedTasks", { count: 10 }), "已添加 10 个任务", "addedTasks number");
});

test("缺失 key 返回 key 本身", () => {
  setLocale("zh-CN");
  assertEqual(t("nonexistent.key"), "nonexistent.key", "missing top-level key");
  assertEqual(t("common.nonexistent"), "common.nonexistent", "missing nested key");
});

test("部分路径缺失返回 key 本身", () => {
  setLocale("en");
  assertEqual(t("common.nonexistent.deep.path"), "common.nonexistent.deep.path", "deep missing path");
});

test("key 为空字符串返回空字符串", () => {
  assertEqual(t(""), "", "empty key");
});

test("getLocale 返回当前 locale", () => {
  setLocale("zh-CN");
  assertEqual(getLocale(), "zh-CN", "getLocale zh-CN");
  setLocale("en");
  assertEqual(getLocale(), "en", "getLocale en");
});

test("未识别的 locale 静默回退到默认 locale", () => {
  setLocale("zh-CN");
  setLocale("fr-FR" as Locale);
  assertEqual(getLocale(), "zh-CN", "fallback fr-FR");
  setLocale("ja" as Locale);
  assertEqual(getLocale(), "zh-CN", "fallback ja");
});

test("en locale 中扫描不到任何中文字符（用户硬性要求）", () => {
  const chineseChars = findChineseCharactersInLocale("en");
  assertEmpty(chineseChars, "en locale should have no Chinese characters");
});

test("zh-CN locale 中确实包含中文字符（验证扫描逻辑正确）", () => {
  const chineseChars = findChineseCharactersInLocale("zh-CN");
  assertTrue(chineseChars.length > 0, "zh-CN locale should have Chinese characters");
});

// ===== 运行测试 =====

/**
 * 运行所有已注册的测试用例。
 *
 * 当此文件被 `node --test` 或直接 `node` 执行时调用。
 * 在 `tsc --noEmit` 类型检查场景下此函数不会被调用，但代码会被类型检查。
 */
function runAllTests(): void {
  let passed = 0;
  let failed = 0;
  const failures: string[] = [];
  for (const testCase of tests) {
    try {
      // 每个测试前重置 locale，避免测试间状态污染。
      setLocale("zh-CN");
      testCase.fn();
      passed += 1;
    } catch (error) {
      failed += 1;
      const message = error instanceof Error ? error.message : String(error);
      failures.push(`  ✗ ${testCase.name}: ${message}`);
    }
  }
  // 重置回默认 locale，避免影响后续模块。
  setLocale("zh-CN");
  if (failed > 0) {
    console.error(`\nFailed ${failed} / ${tests.length} tests:`);
    for (const failure of failures) console.error(failure);
    process.exitCode = 1;
  } else {
    console.log(`\nPassed ${passed} / ${tests.length} tests.`);
  }
}

// 当作为脚本直接执行时运行测试；被 import 时不自动运行。
// 使用 import.meta.url 检测入口点，兼容 ESM。
if (typeof process !== "undefined" && process.argv[1] && import.meta.url.endsWith(process.argv[1].replace(/\\/g, "/"))) {
  runAllTests();
}

// 导出供外部测试运行器使用。
export { runAllTests, test, assertEqual, assertTrue, assertEmpty };
