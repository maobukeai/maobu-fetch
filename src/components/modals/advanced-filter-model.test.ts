import type { AdvancedFilter, Tag, TaskStatus } from "../../types";
import {
  EXTRA_FILTER_STATUSES,
  FILTER_STATUS_TONES,
  PRIMARY_FILTER_STATUSES,
  SIZE_PRESETS_MB,
  buildActiveFilterChips,
  countActiveFilterConditions,
  countExtraSelectedStatuses,
  isSizePresetActive,
  removeFilterChip,
  summarizeHistoryTasks,
  toggleSizePreset,
} from "./advanced-filter-model";

const assert = {
  equal(actual: unknown, expected: unknown, message?: string) {
    if (actual !== expected) {
      throw new Error(
        message ?? `Expected ${String(expected)}, got ${String(actual)}`
      );
    }
  },
  deepEqual(actual: unknown, expected: unknown, message?: string) {
    const a = JSON.stringify(actual);
    const b = JSON.stringify(expected);
    if (a !== b) {
      throw new Error(message ?? `Expected ${b}, got ${a}`);
    }
  },
  ok(cond: unknown, message?: string) {
    if (!cond) {
      throw new Error(message ?? "Expected truthy value");
    }
  },
};

const EMPTY_FILTER: AdvancedFilter = {
  statuses: [],
  sources: [],
  domain: "",
  dateFrom: null,
  dateTo: null,
  sizeMin: null,
  sizeMax: null,
  tagIds: [],
};

// 1. 13 个状态完整切分且全部拥有语义色
const combined = [...PRIMARY_FILTER_STATUSES, ...EXTRA_FILTER_STATUSES];
assert.equal(combined.length, 13);
assert.equal(new Set(combined).size, 13);
for (const status of combined) {
  assert.ok(FILTER_STATUS_TONES[status], `Missing tone for ${status}`);
}

// 2. 条件计数与非常规状态计数
const richFilter: AdvancedFilter = {
  statuses: ["completed", "interrupted", "paused-by-metered"],
  sources: ["extension"],
  domain: "github.com",
  dateFrom: 1_700_000_000_000,
  dateTo: null,
  sizeMin: 100 * 1024 * 1024,
  sizeMax: null,
  tagIds: ["t1"],
};
assert.equal(countActiveFilterConditions(EMPTY_FILTER), 0);
assert.equal(countActiveFilterConditions(richFilter), 8);
assert.equal(countExtraSelectedStatuses(richFilter), 2);

// 3. 摘要胶囊构建与单项移除
const tags: Tag[] = [
  { id: "t1", name: "4K 原画", color: "#2563eb" },
];
const chips = buildActiveFilterChips(richFilter, tags, {
  statusLabel: (s: TaskStatus) => s,
  sourceLabel: (s: string) => s,
  domainPrefix: "域名",
  datePrefix: "日期",
  sizePrefix: "大小",
  tagPrefix: "标签",
});
assert.equal(chips.length, 8);
assert.equal(chips.find((c) => c.kind === "tag")?.color, "#2563eb");

let next = removeFilterChip(richFilter, "status", "interrupted");
assert.deepEqual(next.statuses, ["completed", "paused-by-metered"]);
next = removeFilterChip(next, "source", "extension");
assert.deepEqual(next.sources, []);
next = removeFilterChip(next, "domain", "github.com");
assert.equal(next.domain, "");
next = removeFilterChip(next, "date", "date");
assert.equal(next.dateFrom, null);
next = removeFilterChip(next, "size", "size");
assert.equal(next.sizeMin, null);
next = removeFilterChip(next, "tag", "t1");
assert.deepEqual(next.tagIds, []);

// 4. 大小快捷预设切换
assert.equal(SIZE_PRESETS_MB.length, 2);
const presetApplied = toggleSizePreset(EMPTY_FILTER, 100);
assert.equal(isSizePresetActive(presetApplied, 100), true);
assert.equal(isSizePresetActive(presetApplied, 1024), false);
const presetToggledOff = toggleSizePreset(presetApplied, 100);
assert.equal(isSizePresetActive(presetToggledOff, 100), false);
assert.equal(presetToggledOff.sizeMin, null);

// 5. 历史任务统计汇总
const summary = summarizeHistoryTasks(
  [
    { total_bytes: 1024, downloaded_bytes: 512 },
    { total_bytes: 0, downloaded_bytes: 2048 },
    {},
  ],
  10
);
assert.deepEqual(summary, {
  matchedCount: 3,
  totalCount: 10,
  totalBytes: 3072,
});

console.log("Passed 5 / 5 advanced-filter-model tests.");
