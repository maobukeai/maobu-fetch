export type SettingsNavGroupId = "core" | "ext" | "ui" | "sys";

export type SettingsSectionId =
  | "general"
  | "download"
  | "network"
  | "bt"
  | "browser"
  | "media"
  | "rules"
  | "filename-cleanup"
  | "naming-template"
  | "presets"
  | "templates"
  | "tags"
  | "credentials"
  | "appearance"
  | "shortcuts"
  | "advanced"
  | "about";

export interface SettingsAnchorDef {
  id: string;
  labelKey: string;
}

export interface SettingsSectionMeta {
  group: SettingsNavGroupId;
  descKey: string;
  searchKeywords: string;
  bento: boolean;
  anchors: ReadonlyArray<SettingsAnchorDef>;
}

export const SETTINGS_NAV_GROUPS: ReadonlyArray<{
  id: SettingsNavGroupId;
  labelKey: string;
}> = [
  { id: "core", labelKey: "settings.navGroupCore" },
  { id: "ext", labelKey: "settings.navGroupExt" },
  { id: "ui", labelKey: "settings.navGroupUi" },
  { id: "sys", labelKey: "settings.navGroupSys" },
];

export const SETTINGS_SECTION_META: Record<SettingsSectionId, SettingsSectionMeta> = {
  general: {
    group: "core",
    descKey: "settings.descGeneral",
    searchKeywords:
      "语言 中文 english 开机 启动 最小化 托盘 剪切板 剪贴板 通知 完成提醒 提示音 悬浮窗 历史归档 language startup tray clipboard notification sound archive",
    bento: true,
    anchors: [
      { id: "general-language", labelKey: "settings.anchorLanguage" },
      { id: "general-behavior", labelKey: "settings.anchorBehavior" },
      { id: "general-notifications", labelKey: "settings.anchorNotifications" },
      { id: "general-progress", labelKey: "settings.anchorProgressWindow" },
      { id: "general-archive", labelKey: "settings.anchorArchive" },
    ],
  },
  download: {
    group: "core",
    descKey: "settings.descDownload",
    searchKeywords:
      "目录 文件夹 重名 碰撞 覆盖 跳过 完成动作 低内存 并发 同时下载 连接数 线程 限速 分时段 校验 sha256 directory collision concurrent connections speed limit verify",
    bento: true,
    anchors: [
      { id: "download-storage", labelKey: "settings.anchorStorage" },
      { id: "download-performance", labelKey: "settings.anchorPerformance" },
    ],
  },
  network: {
    group: "core",
    descKey: "settings.descNetwork",
    searchKeywords:
      "代理 proxy http socks 用户名 密码 连通性 重试 策略 流量 计费 metered network retry",
    bento: true,
    anchors: [
      { id: "network-proxy", labelKey: "settings.anchorProxy" },
      { id: "network-aware", labelKey: "settings.anchorMetered" },
      { id: "network-retry", labelKey: "settings.anchorRetry" },
    ],
  },
  bt: {
    group: "core",
    descKey: "settings.descBt",
    searchKeywords:
      "bt 磁力 magnet torrent librqbit 做种 seeding 分享率 上传 限速 tracker 订阅 节点 隐私 ip",
    bento: true,
    anchors: [
      { id: "bt-core", labelKey: "settings.anchorBtCore" },
      { id: "bt-seed", labelKey: "settings.anchorBtSeed" },
      { id: "bt-trackers", labelKey: "settings.anchorBtTrackers" },
    ],
  },
  browser: {
    group: "ext",
    descKey: "settings.descBrowser",
    searchKeywords:
      "浏览器 chrome edge 接管 最小文件 配对码 令牌 撤销 intercept browser pairing extension",
    bento: true,
    anchors: [
      { id: "browser-intercept", labelKey: "settings.anchorIntercept" },
      { id: "browser-pairing", labelKey: "settings.anchorPairing" },
    ],
  },
  media: {
    group: "ext",
    descKey: "settings.descMedia",
    searchKeywords:
      "媒体 yt-dlp ffmpeg ffprobe 组件 路径 播放器 看图器 文件关联 双击 打开方式 media player image association",
    bento: false,
    anchors: [
      { id: "media-tools", labelKey: "settings.anchorMediaTools" },
      { id: "media-assoc", labelKey: "settings.anchorFileAssoc" },
    ],
  },
  rules: {
    group: "ext",
    descKey: "settings.descRules",
    searchKeywords: "分类 规则 扩展名 域名 自动归档 目录 category rules extension domain",
    bento: false,
    anchors: [],
  },
  "filename-cleanup": {
    group: "ext",
    descKey: "settings.descFilenameCleanup",
    searchKeywords: "文件名 清理 广告 后缀 正则 净化 filename cleanup regex",
    bento: false,
    anchors: [],
  },
  "naming-template": {
    group: "ext",
    descKey: "settings.descNamingTemplate",
    searchKeywords: "平台 命名 模板 b站 bilibili 抖音 youtube 变量 naming template",
    bento: false,
    anchors: [],
  },
  presets: {
    group: "ext",
    descKey: "settings.descPresets",
    searchKeywords: "预设 画质 线程 快速参数 presets profile",
    bento: false,
    anchors: [],
  },
  templates: {
    group: "ext",
    descKey: "settings.descTemplates",
    searchKeywords: "模板 接管 站点 规则 task templates intercept",
    bento: false,
    anchors: [],
  },
  tags: {
    group: "ext",
    descKey: "settings.descTags",
    searchKeywords: "标签 颜色 分类 tags labels",
    bento: false,
    anchors: [],
  },
  credentials: {
    group: "ext",
    descKey: "settings.descCredentials",
    searchKeywords: "凭证 cookies 登录态 站点 认证 credentials cookies",
    bento: false,
    anchors: [],
  },
  appearance: {
    group: "ui",
    descKey: "settings.descAppearance",
    searchKeywords:
      "外观 主题 颜色方案 浅色 深色 跟随系统 行高 紧凑 标准 强调色 磨砂 玻璃 acrylic 窗口大小 分辨率 自适应 缩放 theme dark light compact accent frosted glass window scale",
    bento: true,
    anchors: [
      { id: "appearance-theme", labelKey: "settings.anchorTheme" },
      { id: "appearance-window", labelKey: "settings.anchorWindowSize" },
    ],
  },
  shortcuts: {
    group: "ui",
    descKey: "settings.descShortcuts",
    searchKeywords: "快捷键 热键 键盘 绑定 shortcuts hotkeys keybindings",
    bento: false,
    anchors: [],
  },
  advanced: {
    group: "sys",
    descKey: "settings.descAdvanced",
    searchKeywords:
      "高级 迁移 导出 导入 备份 恢复 日志 脱敏 清理 历史 缓存 advanced export import backup restore logs cache",
    bento: true,
    anchors: [
      { id: "advanced-migration", labelKey: "settings.anchorMigration" },
      { id: "advanced-backup", labelKey: "settings.anchorBackup" },
      { id: "advanced-logs", labelKey: "settings.anchorLogs" },
      { id: "advanced-maintenance", labelKey: "settings.anchorMaintenance" },
    ],
  },
  about: {
    group: "sys",
    descKey: "settings.descAbout",
    searchKeywords:
      "关于 版本 检查更新 github 扩展 兼容 赞助 联系 开发者 about version update compatibility",
    bento: false,
    anchors: [],
  },
};

export function filterSettingsSections<T extends { key: SettingsSectionId; label: string }>(
  items: readonly T[],
  query: string
): T[] {
  const normalized = query.trim().toLowerCase();
  if (!normalized) return [...items];
  return items.filter((item) => {
    const meta = SETTINGS_SECTION_META[item.key];
    return (
      item.label.toLowerCase().includes(normalized) ||
      item.key.toLowerCase().includes(normalized) ||
      meta.searchKeywords.toLowerCase().includes(normalized)
    );
  });
}

export function countDirtySettings(
  draft: Record<string, unknown>,
  saved: Record<string, unknown>
): number {
  const keys = new Set([...Object.keys(draft), ...Object.keys(saved)]);
  let count = 0;
  for (const key of keys) {
    // color_scheme 与 theme 同步写入，只计作一项外观变更
    if (key === "theme" && (keys.has("color_scheme") || "color_scheme" in draft)) {
      continue;
    }
    if (JSON.stringify(draft[key]) !== JSON.stringify(saved[key])) {
      count += 1;
    }
  }
  return count;
}

export function matchesSearchQuery(
  query: string,
  ...texts: Array<string | undefined>
): boolean {
  const normalized = query.trim().toLowerCase();
  if (!normalized) return false;
  return texts.some((text) => Boolean(text && text.toLowerCase().includes(normalized)));
}
