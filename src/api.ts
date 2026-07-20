import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AppInfo, AppSettings, CategoryRule, CategoryRuleTestResult, CompletionAction, DeepLinkReceivedPayload, DetectedMediaTools, DownloadPreset, DownloadTask, DuplicateCheckResult, ErrorDiagnosis, ExtensionCompatibilityResult, FilenameCleanupRule, MediaCredential, MediaPlatform, MediaProbeResult, MeteredNetworkDetectedPayload, NewTaskRequest, PairingInfo, PlatformCompatibility, PlatformNamingTemplate, PowerAction, PowerActionState, PrecheckRequest, PrecheckResult, ProxyAuth, ProxyTestResult, RestorePreview, RestoreStats, RetryPolicy, SelfcheckReport, Tag, TaskEvent, TaskNotificationPayload, TaskTagsMap, TaskTemplate, TaskTemplateTestResult, ToolComponent, ToolStatus, UpdateCheckResult, UrlHistoryEntry, WaitReason } from "./types";

export const isDesktop = () => "__TAURI_INTERNALS__" in window;
const call = <T>(command: string, args?: Record<string, unknown>): Promise<T> => isDesktop() ? invoke<T>(command, args) : Promise.reject(new Error("请运行猫步下载器桌面应用"));

export const api = {
  list: () => isDesktop() ? call<DownloadTask[]>("tasks_list") : Promise.resolve([]),
  add: (request: NewTaskRequest) => call<DownloadTask>("task_add", { request }),
  addBatch: (urls: string[], template: Omit<NewTaskRequest, "url">) => call<DownloadTask[]>("tasks_add_batch", { request: { urls, destination: template.destination, headers: template.headers, scheduled_at: template.scheduled_at, priority: template.priority, per_task_speed_limit: template.per_task_speed_limit, collision_policy: template.collision_policy, completion_action: template.completion_action, connection_count: template.connection_count } }),
  exportTasks: (path: string) => call<number>("tasks_export", { path }),
  importTasks: (path: string, destination: string) => call<DownloadTask[]>("tasks_import", { path, destination }),
  action: (id: string, action: string) => call<void>("task_action", { id, action }),
  updateTaskOptions: (id: string, options: { priority?: number; perTaskSpeedLimit?: number; completionAction?: CompletionAction }) => call<DownloadTask>("task_update_options", { id, priority: options.priority, perTaskSpeedLimit: options.perTaskSpeedLimit, completionAction: options.completionAction }),
  /** 更新任务级重试策略覆盖（Task 14）。`null` 表示清除覆盖、回退到全局默认。 */
  updateRetryPolicy: (id: string, policy: RetryPolicy | null) => call<DownloadTask>("task_update_retry_policy", { id, policy }),
  /**
   * Task 31：更新任务级代理覆盖与代理认证。
   * - `proxyOverride = null`：清除覆盖、回退到全局。
   * - `proxyOverride = ""`：显式禁用代理。
   * - `proxyOverride = "http://..."`：使用指定代理 URL。
   * - `proxyAuth`：可选认证；密码非空时由后端 DPAPI 加密后落库。
   *   返回的 task.proxy_auth.password 是 DPAPI 密文，前端不应展示。
   */
  updateTaskProxy: (id: string, proxyOverride: string | null, proxyAuth: ProxyAuth | null) =>
    call<DownloadTask>("task_update_proxy", { id, proxyOverride, proxyAuth }),
  /**
   * Task 31：测试代理连通性与实际出口 IP。
   * 通过指定代理 URL 请求 ipify，返回出口 IP 与延迟。
   * `auth.password` 应为明文（前端输入），此命令不读取数据库。
   * 失败时 `success = false`，`error` 为脱敏后的中文说明。
   */
  proxyTest: (proxyUrl: string, auth: ProxyAuth | null) =>
    call<ProxyTestResult>("proxy_test", { proxyUrl, auth }),
  bulkAction: (ids: string[], action: string) => call<void>("tasks_bulk_action", { ids, action }),
  remove: (id: string, deleteFile: boolean) => call<void>("task_remove", { id, deleteFile }),
  /** Task 21.2：重命名任务文件名。仅 Queued 状态可用；后端会校验合法性和重名。 */
  rename: (id: string, newFilename: string) => call<DownloadTask>("task_rename", { id, newFilename }),
  reorder: (ids: string[]) => call<void>("queue_reorder", { ids }),
  settings: () => call<AppSettings>("settings_get"),
  saveSettings: (settings: AppSettings) => call<void>("settings_save", { settings }),
  powerActionState: () => isDesktop() ? call<PowerActionState>("power_action_get") : Promise.resolve({ action: "none", phase: "idle", remaining_seconds: 0, target_count: 0 } as PowerActionState),
  armPowerAction: (action: PowerAction) => call<PowerActionState>("power_action_arm", { action }),
  cancelPowerAction: () => call<PowerActionState>("power_action_cancel"),
  openFile: (id: string) => call<void>("task_open_file", { id }),
  openFolder: (id: string) => call<void>("task_open_folder", { id }),
  verify: (id: string) => call<string>("task_verify", { id }),
  logToBackend: (message: string) => call<void>("log_to_backend", { message }),
  diagnose: (id: string) => call<ErrorDiagnosis | null>("task_diagnose", { id }),
  getWaitReason: (id: string) => call<WaitReason>("task_wait_reason", { id }),
  precheck: (request: PrecheckRequest) => call<PrecheckResult>("task_precheck", { request }),
  /** 检测新任务是否与已有任务重复（Task 10）。`fileSize` 和 `sha256` 可选，用于 SameChecksum 检测。 */
  duplicateCheck: (url: string, targetPath: string, options?: { fileSize?: number; sha256?: string }) => call<DuplicateCheckResult>("duplicate_check", { url, targetPath, fileSize: options?.fileSize, sha256: options?.sha256 }),
  clearHistory: (includeCompleted: boolean) => call<void>("history_clear", { includeCompleted }),
  pairing: () => call<PairingInfo>("pairing_info"),
  rotatePairing: () => call<PairingInfo>("pairing_rotate"),
  revokePairing: () => call<void>("pairing_revoke"),
  probeMedia: (url: string, options?: { cookie?: string; referer?: string; user_agent?: string }) => call<MediaProbeResult>("media_probe", { url, cookie: options?.cookie, referer: options?.referer, user_agent: options?.user_agent }),
  /**
   * Task 37.1：识别 URL 所属媒体平台。
   *
   * 返回平台名字符串（`"douyin"` / `"tiktok"` / `"twitter"` / `"youtube"` /
   * `"bilibili"` / `"weibo"` / `"unknown"`），前端用于在新建任务对话框展示
   * "检测到：抖音" 等提示，帮助用户预期下载行为。
   *
   * 仅基于 URL host 模式匹配，不发起新的网络请求。`"unknown"` 不阻止 yt-dlp 通用流程。
   */
  mediaDetectPlatform: (url: string) => call<MediaPlatform>("media_detect_platform", { url }),
  /**
   * Task 41：规范化用户输入的 URL（分享文本提取 + 短链跟随 + 跟踪参数剥离）。
   *
   * 调用顺序与 `probeMedia` 一致：
   * 1. 从分享文本（如"xxx https://v.douyin.com/yyy 复制此链接..."）中提取首个 URL。
   * 2. 若是已知短链域名，跟随 HTTP 302 到最终地址；失败时回退到提取后的 URL。
   * 3. 剥离 utm_* / fbclid / gclid 等跟踪参数。
   *
   * 前端在新建任务对话框中调用此 API，展示"原文本 → 规范化 URL"预览，
   * 帮助用户确认分享文本已被正确解析（如抖音分享 → 抖音长链）。
   * 失败时返回中文错误（如"未识别到有效链接"）。
   */
  mediaNormalizeUrl: (input: string) => call<string>("media_normalize_url", { input }),
  detectSystemMediaTools: () => call<DetectedMediaTools>("media_tools_detect_system"),
  toolStatus: () => isDesktop() ? call<ToolStatus>("media_tool_status") : Promise.resolve({ state: "missing", version: "yt-dlp 2026.06.09 · FFmpeg 8.1.2", downloaded_bytes: 0, total_bytes: 0, installed_bytes: 0, yt_dlp_available: false, ffmpeg_available: false, yt_dlp_version: "2026.06.09", ffmpeg_version: "8.1.2 essentials", yt_dlp_download_bytes: 18_202_192, ffmpeg_download_bytes: 109_728_040, yt_dlp_installed_bytes: 0, ffmpeg_installed_bytes: 0, yt_dlp_source: "missing", ffmpeg_source: "missing" } as ToolStatus),
  installMediaTool: (component: ToolComponent) => call<void>("media_tool_install", { component }),
  installMediaTools: () => call<void>("media_tools_install"),
  cancelMediaTools: () => call<void>("media_tools_cancel"),
  removeMediaTools: () => call<void>("media_tools_remove"),
  removeMediaTool: (component: ToolComponent) => call<void>("media_tool_remove", { component }),
  checkMediaToolsUpdate: () => call<ToolStatus>("media_tools_check_update"),
  /** Task 26.2 / 26.5：检查猫步下载器应用更新。只检查不自动下载，结果仅用于提醒用户。 */
  appCheckUpdate: () => call<UpdateCheckResult>("app_check_update"),
  /** Task 26.3 / 26.6：检查浏览器扩展版本与桌面端兼容性。`extVersion` 为扩展 manifest 中的 version。 */
  extensionCheckCompatibility: (extVersion: string) => call<ExtensionCompatibilityResult>("extension_check_compatibility", { extVersion }),
  categoryRuleList: () => isDesktop() ? call<CategoryRule[]>("category_rule_list") : Promise.resolve([]),
  categoryRuleAdd: (rule: CategoryRule) => call<CategoryRule>("category_rule_add", { rule }),
  categoryRuleUpdate: (rule: CategoryRule) => call<void>("category_rule_update", { rule }),
  categoryRuleDelete: (id: string) => call<void>("category_rule_delete", { id }),
  categoryRuleTest: (rule: CategoryRule, url: string, fileName: string, contentType?: string) => call<CategoryRuleTestResult>("category_rule_test", { rule, url, fileName, contentType: contentType ?? null }),
  categoryRuleApply: (url: string, fileName: string, contentType?: string) => call<string | null>("category_rule_apply", { url, fileName, contentType: contentType ?? null }),
  // ===== Task 36: 任务模板 CRUD 与匹配测试 =====
  /** 列出全部任务模板，按 priority 升序、name 升序返回（Task 36）。 */
  taskTemplateList: () => isDesktop() ? call<TaskTemplate[]>("task_template_list") : Promise.resolve([] as TaskTemplate[]),
  /** 新增任务模板（Task 36）。前端应生成 UUID 作为 id。 */
  taskTemplateAdd: (template: TaskTemplate) => call<TaskTemplate>("task_template_add", { template }),
  /** 更新任务模板字段（Task 36）。所有字段都会被覆盖。 */
  taskTemplateUpdate: (template: TaskTemplate) => call<void>("task_template_update", { template }),
  /** 删除任务模板（Task 36）。 */
  taskTemplateDelete: (id: string) => call<void>("task_template_delete", { id }),
  /** 测试给定 URL 是否命中任意模板（Task 36）。供新建任务对话框展示"已匹配模板"提示。 */
  taskTemplateTest: (url: string) => call<TaskTemplateTestResult>("task_template_test", { url }),
  // ===== Task 46: 媒体凭证 CRUD =====
  /** 列出全部已存储的媒体凭证，按 domain 升序返回。任一行解密失败时该行被跳过。 */
  mediaCredentialList: () => isDesktop() ? call<MediaCredential[]>("media_credential_list") : Promise.resolve([] as MediaCredential[]),
  /** 保存（新增或更新）一条媒体凭证。`cookie` 为明文，后端用 DPAPI 加密落库。 */
  mediaCredentialSave: (credential: MediaCredential) => call<MediaCredential>("media_credential_save", { credential }),
  /** 按 domain 查询单条凭证。解密失败时后端返回中文错误；不存在返回 null。 */
  mediaCredentialGet: (domain: string) => call<MediaCredential | null>("media_credential_get", { domain }),
  /** 按 domain 删除单条凭证。不存在不算错误（幂等）。 */
  mediaCredentialDelete: (domain: string) => call<void>("media_credential_delete", { domain }),
  // ===== Task 44: 平台兼容性矩阵（只读） =====
  /**
   * 列出全部平台兼容性记录，按 platform 升序返回。
   *
   * 内置 6 条默认记录（YouTube/哔哩哔哩=Verified，
   * 抖音/TikTok/Twitter/微博=Experimental）由后端 `Store::open` 自动 seed，
   * 用户修改不会被覆盖。用于设置页"关于 > 平台兼容性"子区域展示矩阵。
   */
  platformCompatibilityList: () => isDesktop() ? call<PlatformCompatibility[]>("platform_compatibility_list") : Promise.resolve([] as PlatformCompatibility[]),
  /**
   * 按 platform 查询单条兼容性记录。不存在返回 null。
   *
   * `platform` 应为 `MediaPlatform` 序列化值（`"douyin"` / `"tiktok"` /
   * `"twitter"` / `"youtube"` / `"bilibili"` / `"weibo"` / `"unknown"`）。
   * 新建任务对话框检测到平台后调用此命令展示徽章（已验证/实验性/不支持）。
   */
  platformCompatibilityGet: (platform: string) => call<PlatformCompatibility | null>("platform_compatibility_get", { platform }),
  /** 列出全部文件名清理规则（内置 + 自定义），Task 20。 */
  filenameCleanupRuleList: () => isDesktop() ? call<FilenameCleanupRule[]>("filename_cleanup_rule_list") : Promise.resolve([]),
  /** 新增文件名清理规则，Task 20。后端会校验正则可编译。 */
  filenameCleanupRuleAdd: (rule: FilenameCleanupRule) => call<FilenameCleanupRule>("filename_cleanup_rule_add", { rule }),
  /** 更新文件名清理规则。内置规则可编辑字段，Task 20。 */
  filenameCleanupRuleUpdate: (rule: FilenameCleanupRule) => call<void>("filename_cleanup_rule_update", { rule }),
  /** 删除文件名清理规则，Task 20。 */
  filenameCleanupRuleDelete: (id: string) => call<void>("filename_cleanup_rule_delete", { id }),
  /** 预览文件名清理结果。`rules` 省略时使用数据库中启用的规则，Task 20。 */
  filenameCleanupPreview: (fileName: string, rules?: FilenameCleanupRule[]) => call<string>("filename_cleanup_preview", { fileName, rules: rules ?? null }),
  // ===== Task 43: 平台命名模板 CRUD =====
  /**
   * 列出全部平台命名模板（内置 + 自定义）。
   *
   * 排序：platform 升序、enabled 降序（启用的在前）、id 升序。
   * 设置页与下载流程均使用此方法读取模板列表。
   */
  platformNamingTemplateList: () => isDesktop() ? call<PlatformNamingTemplate[]>("platform_naming_template_list") : Promise.resolve([] as PlatformNamingTemplate[]),
  /**
   * 新增一条平台命名模板。
   *
   * 校验：`id` / `platform` / `template` 不能为空（trim 后）。
   * `platform` 会被后端转为小写存储。`is_builtin` 应为 false（自定义模板）。
   */
  platformNamingTemplateAdd: (template: PlatformNamingTemplate) => call<PlatformNamingTemplate>("platform_naming_template_add", { template }),
  /**
   * 更新一条平台命名模板。
   *
   * `is_builtin` 字段由数据库既有值决定，前端不应修改。
   * 内置模板可编辑、可禁用，但删除按钮在前端隐藏。
   */
  platformNamingTemplateUpdate: (template: PlatformNamingTemplate) => call<void>("platform_naming_template_update", { template }),
  /** 按 id 删除一条平台命名模板。内置模板的删除按钮在前端隐藏。 */
  platformNamingTemplateDelete: (id: string) => call<void>("platform_naming_template_delete", { id }),
  /** 列出全部下载预设（内置 + 自定义），Task 12。 */
  presetList: () => isDesktop() ? call<DownloadPreset[]>("preset_list") : Promise.resolve([]),
  /** 新增自定义下载预设（`is_builtin` 强制为 false），Task 12。 */
  presetAdd: (preset: DownloadPreset) => call<DownloadPreset>("preset_add", { preset }),
  /** 更新预设字段。内置预设可编辑字段，但 `is_builtin` 标志不可修改，Task 12。 */
  presetUpdate: (preset: DownloadPreset) => call<void>("preset_update", { preset }),
  /** 删除预设。内置预设不可删除（后端返回中文错误），Task 12。 */
  presetDelete: (id: string) => call<void>("preset_delete", { id }),
  /** 把预设套用到指定任务：覆盖连接数/限速/完成动作/校验/计划时间。仅 Queued/Paused/Scheduled/Failed/Cancelled 状态可用，Task 12。 */
  presetApplyToTask: (taskId: string, presetId: string) => call<DownloadTask>("preset_apply_to_task", { taskId, presetId }),
  /** 打开日志目录（Task 23.3）。路径不返回前端，仅在系统文件管理器中打开。 */
  openLogsDir: () => call<void>("open_logs_dir"),
  /** 导出最近 24 小时日志到指定路径（Task 23.4）。返回包含的日志文件数。 */
  exportRecentLogs: (outputPath: string) => call<number>("export_recent_logs", { outputPath }),
  /** 新增或更新一条 URL 历史（Task 19）。同一 URL 重复添加只更新 last_used（LRU）。 */
  urlHistoryAdd: (url: string) => isDesktop() ? call<void>("url_history_add", { url }) : Promise.resolve(),
  /** 列出最近 20 条 URL 历史，按 last_used 降序返回（Task 19）。 */
  urlHistoryList: () => isDesktop() ? call<UrlHistoryEntry[]>("url_history_list") : Promise.resolve([] as UrlHistoryEntry[]),
  /** 清空全部 URL 历史（Task 19）。 */
  urlHistoryClear: () => call<void>("url_history_clear"),
  // ===== Task 25: 标签 CRUD 与任务-标签关联 =====
  /** 列出全部标签，按 name 升序排列。 */
  tagList: () => isDesktop() ? call<Tag[]>("tag_list") : Promise.resolve([] as Tag[]),
  /** 新增用户标签。name 重复时后端返回中文错误。 */
  tagAdd: (tag: Tag) => call<Tag>("tag_add", { tag }),
  /** 更新标签字段。 */
  tagUpdate: (tag: Tag) => call<void>("tag_update", { tag }),
  /** 删除标签。关联的 task_tags 由外键级联清理。 */
  tagDelete: (id: string) => call<void>("tag_delete", { id }),
  /** 替换任务的全部标签关联。传入空数组清空该任务的标签。 */
  taskTagsSet: (taskId: string, tagIds: string[]) => call<void>("task_tags_set", { taskId, tagIds }),
  /** 获取单个任务的标签列表。 */
  taskTagsGet: (taskId: string) => isDesktop() ? call<Tag[]>("task_tags_get", { taskId }) : Promise.resolve([] as Tag[]),
  /** 列出全部任务-标签关联，按 task_id 分组返回。 */
  taskTagsListAll: () => isDesktop() ? call<TaskTagsMap>("task_tags_list_all") : Promise.resolve({} as TaskTagsMap),
  /** Task 27.2：导出完整备份到 JSON 文件。
   *  `includeAuth = true` 时必须提供 `password`，备份文件会被 AES-256-GCM 加密；
   *  `includeAuth = false` 时认证字段（Cookie/Authorization/代理密码）会被清空。 */
  backupExport: (path: string, includeAuth: boolean, password: string | null) => call<void>("backup_export", { path, includeAuth, password }),
  /** Task 27.3：读取备份文件并计算恢复预览，不修改任何状态。
   *  加密文件必须提供密码。返回新增/覆盖/跳过的条数。 */
  backupPreview: (path: string, password: string | null) => call<RestorePreview>("backup_preview", { path, password }),
  /** Task 27.4：应用备份恢复。设置覆盖；规则/预设按 ID upsert；
   *  URL 历史去重；任务按 ID 去重（已存在的跳过，不覆盖用户进度）。 */
  backupRestore: (path: string, password: string | null) => call<RestoreStats>("backup_restore", { path, password }),
  subscribeMediaTools: async (handler: (status: ToolStatus) => void): Promise<UnlistenFn | undefined> => isDesktop() ? listen<ToolStatus>("media-tools-progress", event => handler(event.payload)) : undefined,
  subscribeSettings: async (handler: (settings: AppSettings) => void): Promise<UnlistenFn | undefined> => isDesktop() ? listen<AppSettings>("settings-changed", event => handler(event.payload)) : undefined,
  subscribePowerAction: async (handler: (state: PowerActionState) => void): Promise<UnlistenFn | undefined> => isDesktop() ? listen<PowerActionState>("power-action-state", event => handler(event.payload)) : undefined,
  subscribeNotificationErrors: async (handler: (message: string) => void): Promise<UnlistenFn | undefined> => isDesktop() ? listen<string>("notification-error", event => handler(event.payload)) : undefined,
  subscribeStartupSelfcheck: async (handler: (report: SelfcheckReport) => void): Promise<UnlistenFn | undefined> => isDesktop() ? listen<SelfcheckReport>("startup-selfcheck", event => handler(event.payload)) : undefined,
  /** Task 29：监听 maobu:// 深链错误（URL 无效、任务不存在等）。 */
  subscribeDeepLinkErrors: async (handler: (message: string) => void): Promise<UnlistenFn | undefined> => isDesktop() ? listen<string>("deep-link-error", event => handler(event.payload)) : undefined,
  /** Task 29：监听 maobu:// 深链与 .maobu-task 文件导入事件。 */
  subscribeDeepLinkReceived: async (handler: (payload: DeepLinkReceivedPayload) => void): Promise<UnlistenFn | undefined> => isDesktop() ? listen<DeepLinkReceivedPayload>("deep-link-received", event => handler(event.payload)) : undefined,
  /** Task 30：监听任务完成/失败通知事件。前端据此播放提示音并展示带"一键重试"按钮的 toast。 */
  subscribeTaskNotification: async (handler: (payload: TaskNotificationPayload) => void): Promise<UnlistenFn | undefined> => isDesktop() ? listen<TaskNotificationPayload>("task-notification", event => handler(event.payload)) : undefined,
  /** Task 32：立即检测当前网络是否为计量网络。失败时返回 false（安全回退）。 */
  networkCheckMetered: () => call<boolean>("network_check_metered"),
  /** Task 32：监听计量网络自动暂停事件。后端在暂停 ≥1 个任务时 emit，前端展示 toast。 */
  subscribeMeteredNetwork: async (handler: (payload: MeteredNetworkDetectedPayload) => void): Promise<UnlistenFn | undefined> => isDesktop() ? listen<MeteredNetworkDetectedPayload>("metered-network-detected", event => handler(event.payload)) : undefined,
  /**
   * Task 34.3：获取应用信息（版本、便携模式、数据目录）。
   * 前端在设置页"关于"分组调用此命令，便携模式启用时显示醒目提示。
   */
  appGetInfo: () => isDesktop() ? call<AppInfo>("app_get_info") : Promise.resolve({ version: "0.0.0", portable_mode: false, data_dir: "" } as AppInfo),
  subscribe: async (handler: (event: TaskEvent | { removed: string }) => void): Promise<UnlistenFn[]> => {
    if (!isDesktop()) return [];
    return Promise.all([
      listen<TaskEvent>("task-created", event => handler(event.payload)),
      listen<TaskEvent>("task-updated", event => handler(event.payload)),
      listen<string>("task-removed", event => handler({ removed: event.payload }))
    ]);
  }
};
