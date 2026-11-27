import {
  SETTINGS_NAV_GROUPS,
  SETTINGS_SECTION_META,
  countDirtySettings,
  filterSettingsSections,
  matchesSearchQuery,
  type SettingsSectionId,
} from "./settings-fusion";

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

const ALL_SECTIONS: Array<{ key: SettingsSectionId; label: string }> = [
  { key: "general", label: "常规" },
  { key: "download", label: "下载" },
  { key: "network", label: "网络" },
  { key: "bt", label: "BT / 磁力" },
  { key: "browser", label: "浏览器" },
  { key: "media", label: "媒体" },
  { key: "rules", label: "分类规则" },
  { key: "filename-cleanup", label: "文件名清理" },
  { key: "naming-template", label: "平台命名模板" },
  { key: "presets", label: "下载预设" },
  { key: "templates", label: "模板与接管" },
  { key: "tags", label: "标签管理" },
  { key: "credentials", label: "媒体凭证" },
  { key: "appearance", label: "外观" },
  { key: "shortcuts", label: "快捷键" },
  { key: "advanced", label: "高级" },
  { key: "about", label: "关于" },
];

// 1. 17 个分区的元数据与分组合法性校验
const validGroups = new Set(SETTINGS_NAV_GROUPS.map((g) => g.id));
assert.equal(Object.keys(SETTINGS_SECTION_META).length, 17);
for (const item of ALL_SECTIONS) {
  const meta = SETTINGS_SECTION_META[item.key];
  assert.ok(meta, `Missing meta for ${item.key}`);
  assert.ok(validGroups.has(meta.group), `Invalid group for ${item.key}`);
  assert.ok(meta.descKey.startsWith("settings."));
}

// 2. 中英文设置项实时检索与自动定位
assert.equal(filterSettingsSections(ALL_SECTIONS, "").length, 17);
assert.deepEqual(
  filterSettingsSections(ALL_SECTIONS, "做种").map((x) => x.key),
  ["bt"]
);
assert.deepEqual(
  filterSettingsSections(ALL_SECTIONS, "磨砂").map((x) => x.key),
  ["appearance"]
);
assert.deepEqual(
  filterSettingsSections(ALL_SECTIONS, "ACRYLIC").map((x) => x.key),
  ["appearance"]
);
assert.deepEqual(
  filterSettingsSections(ALL_SECTIONS, "tracker").map((x) => x.key),
  ["bt"]
);
assert.deepEqual(
  filterSettingsSections(ALL_SECTIONS, "proxy").map((x) => x.key),
  ["network"]
);
assert.ok(
  filterSettingsSections(ALL_SECTIONS, "限速").some((x) => x.key === "download")
);
assert.equal(
  filterSettingsSections(ALL_SECTIONS, "不存在的关键词xyz").length,
  0
);

// 3. 脏状态计数（color_scheme 与 theme 去重、嵌套对象深对比）
const base = {
  language: "zh-CN",
  color_scheme: "system",
  theme: "system",
  frosted_glass: true,
  scheduled_limit: {
    enabled: false,
    start_minutes: 540,
    end_minutes: 1080,
    limit_kbps: 2048,
  },
};
assert.equal(countDirtySettings({ ...base }, { ...base }), 0);
assert.equal(
  countDirtySettings({ ...base, color_scheme: "dark", theme: "dark" }, base),
  1
);
assert.equal(
  countDirtySettings(
    {
      ...base,
      frosted_glass: false,
      scheduled_limit: {
        enabled: true,
        start_minutes: 540,
        end_minutes: 1080,
        limit_kbps: 1024,
      },
    },
    base
  ),
  2
);

// 4. 设置行高亮匹配判定
assert.equal(matchesSearchQuery("", "磨砂玻璃"), false);
assert.equal(matchesSearchQuery("   ", "磨砂玻璃"), false);
assert.equal(matchesSearchQuery("磨砂", "磨砂玻璃", "Acrylic"), true);
assert.equal(matchesSearchQuery("acrylic", "磨砂玻璃", "Windows Acrylic"), true);
assert.equal(matchesSearchQuery("代理", "磨砂玻璃"), false);

console.log("Passed 4 / 4 settings-fusion tests.");
