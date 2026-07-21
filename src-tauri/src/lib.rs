mod bridge;
pub mod cli;
mod deep_link;
mod logging;
mod manager;
mod media;
mod media_cookies;
// Task 37 / 39：媒体平台识别与适配（抖音 / TikTok / Twitter/X 等）。
// 提供 detect_platform / expand_short_url / classify_platform_error /
// is_twitter_space / format_twitter_filename 等函数。
mod media_platforms;
mod media_tools;
mod models;
mod network_awareness;
// Task 31：代理配置精细化（resolve_proxy / test_proxy）与 DPAPI 安全存储。
mod proxy;
// Task 34：便携版模式。检测 EXE 同目录 maobu.portable 标记文件，
// 存在时所有数据写入 EXE_DIR/data/。
mod portable;
mod secure_storage;
mod store;
mod task_transfer;
mod tray_icon;
mod updater;

use bridge::PairingService;
use cli::CliCommand;
use deep_link::{parse_deep_link, DeepLinkAction};
pub use manager::RateLimiter;
use manager::{
    apply_category_rules, apply_filename_cleanup, normalize_directory, test_category_rule,
    test_task_template,
};
use manager::{DownloadManager, ErrorContext, SharedManager};
use media_tools::MediaTools;
use models::{
    AppSettings, BatchTaskRequest, CategoryRule, CategoryRuleTestResult, CollisionPolicy,
    CompletionAction, DetectedMediaTools, DownloadPreset, DownloadTask, DuplicateCheckResult,
    ErrorDiagnosis, ExtensionCompatibilityResult, FilenameCleanupRule, MediaCredential,
    MediaProbeResult, NewTaskRequest, PairingInfo, PlatformCompatibility, PlatformNamingTemplate,
    PowerAction, PowerActionState, PrecheckRequest, PrecheckResult, ProxyAuth, ProxyTestResult,
    RestorePreview, RestoreStats, RetryPolicy, Tag, TaskTemplate, TaskTemplateTestResult,
    ToolComponent, ToolStatus, UpdateCheckResult, UrlHistoryEntry, WaitReason,
};
use std::{path::PathBuf, sync::Arc};
use store::Store;
use tauri::menu::{CheckMenuItem, Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Listener, Manager, State};

struct TrayMenuItems {
    clipboard: CheckMenuItem<tauri::Wry>,
    low_memory: CheckMenuItem<tauri::Wry>,
    frosted_glass: CheckMenuItem<tauri::Wry>,
}

#[tauri::command]
async fn tasks_list(manager: State<'_, SharedManager>) -> Result<Vec<DownloadTask>, String> {
    manager.list().await
}

#[tauri::command]
async fn task_add(
    request: NewTaskRequest,
    manager: State<'_, SharedManager>,
) -> Result<DownloadTask, String> {
    manager.inner().add(request).await
}

#[tauri::command]
async fn tasks_add_batch(
    request: BatchTaskRequest,
    manager: State<'_, SharedManager>,
) -> Result<Vec<DownloadTask>, String> {
    manager.inner().add_batch(request).await
}

#[tauri::command]
async fn tasks_export(path: String, manager: State<'_, SharedManager>) -> Result<usize, String> {
    manager.export_tasks(&path).await
}

#[tauri::command]
async fn tasks_import(
    path: String,
    destination: String,
    manager: State<'_, SharedManager>,
) -> Result<Vec<DownloadTask>, String> {
    manager.inner().import_tasks(&path, &destination).await
}

/// Task 27.2：导出完整备份。
///
/// `include_auth = true` 时必须提供 `password`，备份文件会以 AES-256-GCM 加密；
/// `include_auth = false` 时备份为明文 JSON，认证字段（Cookie/Authorization/代理密码）会被清空。
/// 路径必须是绝对路径且以 `.json` 结尾。
#[tauri::command]
async fn backup_export(
    path: String,
    include_auth: bool,
    password: Option<String>,
    manager: State<'_, SharedManager>,
) -> Result<(), String> {
    manager
        .backup_export(&path, include_auth, password.as_deref())
        .await
}

/// Task 27.3：读取备份文件并计算恢复预览，不修改任何状态。
///
/// 加密文件必须提供密码。返回 [`RestorePreview`] 列出本次恢复将新增、覆盖、跳过的条数。
#[tauri::command]
async fn backup_preview(
    path: String,
    password: Option<String>,
    manager: State<'_, SharedManager>,
) -> Result<RestorePreview, String> {
    manager.backup_preview(&path, password.as_deref()).await
}

/// Task 27.4：应用备份恢复。
///
/// 设置覆盖；规则/预设按 ID upsert；URL 历史去重；任务按 ID 去重（已存在的跳过，
/// 不覆盖用户进度）。返回 [`RestoreStats`] 反映实际写入数据库的条目数。
#[tauri::command]
async fn backup_restore(
    path: String,
    password: Option<String>,
    manager: State<'_, SharedManager>,
) -> Result<RestoreStats, String> {
    manager.backup_restore(&path, password.as_deref()).await
}

#[tauri::command]
async fn task_action(
    id: String,
    action: String,
    manager: State<'_, SharedManager>,
) -> Result<(), String> {
    manager.inner().action(&id, &action).await
}

#[tauri::command]
async fn task_update_options(
    id: String,
    priority: Option<i32>,
    per_task_speed_limit: Option<u64>,
    completion_action: Option<CompletionAction>,
    manager: State<'_, SharedManager>,
) -> Result<DownloadTask, String> {
    manager
        .update_task_options(&id, priority, per_task_speed_limit, completion_action)
        .await
}

/// 更新任务级重试策略覆盖（Task 14）。
/// `policy = null` 表示清除覆盖，回退到全局默认。
#[tauri::command]
async fn task_update_retry_policy(
    id: String,
    policy: Option<RetryPolicy>,
    manager: State<'_, SharedManager>,
) -> Result<DownloadTask, String> {
    manager.update_retry_policy(&id, policy).await
}

/// Task 31.5：更新任务级代理覆盖与代理认证。
///
/// - `proxy_override = null`：清除覆盖，回退到全局 `AppSettings.proxy_mode`/`proxy_url`。
/// - `proxy_override = Some("")`：显式禁用代理（即使全局是 manual）。
/// - `proxy_override = Some(url)`：使用指定代理 URL。
/// - `proxy_auth`：可选认证；密码会在保存前经 DPAPI 加密后写入数据库，
///   不在响应、日志或事件中暴露明文。
///
/// 校验：
/// - URL 非空时必须是合法 http/https/socks5/socks5h 前缀（`validate_proxy_url`）。
/// - 任务必须存在；任务不存在返回中文错误。
/// - 密码为空字符串时视为"无认证"，等价于 `proxy_auth = null`。
#[tauri::command]
async fn task_update_proxy(
    id: String,
    proxy_override: Option<String>,
    proxy_auth: Option<ProxyAuth>,
    manager: State<'_, SharedManager>,
) -> Result<DownloadTask, String> {
    manager.update_proxy(&id, proxy_override, proxy_auth).await
}

/// Task 31.4：测试代理连通性与实际出口 IP。
///
/// 通过指定代理 URL 请求 `https://api.ipify.org/format=json`，
/// 测量延迟并返回出口 IP。`auth.password` 期望为前端输入的明文（此命令不读取数据库）。
///
/// 失败时 `success = false`，`error` 为脱敏后的中文说明
/// （URL 中的 `userinfo` 段会被替换为 `***`，不暴露认证字段）。
#[tauri::command]
async fn proxy_test(proxy_url: String, auth: Option<ProxyAuth>) -> Result<ProxyTestResult, String> {
    Ok(proxy::test_proxy(&proxy_url, auth.as_ref()).await)
}

#[tauri::command]
async fn tasks_bulk_action(
    ids: Vec<String>,
    action: String,
    manager: State<'_, SharedManager>,
) -> Result<(), String> {
    manager.inner().bulk_action(&ids, &action).await
}

#[tauri::command]
async fn task_remove(
    id: String,
    delete_file: bool,
    manager: State<'_, SharedManager>,
) -> Result<(), String> {
    manager.inner().remove(&id, delete_file).await
}

/// Task 21.2：重命名任务文件名。
///
/// 仅允许 `Queued`（等待中，即未开始）状态的任务重命名，避免与活动分片、
/// 已合并文件或外部句柄产生冲突。重命名前会校验文件名合法性和目标目录重名，
/// 失败时返回可操作的中文错误，不修改任何状态。
#[tauri::command]
async fn task_rename(
    id: String,
    new_filename: String,
    manager: State<'_, SharedManager>,
) -> Result<DownloadTask, String> {
    manager.rename(&id, &new_filename).await
}

#[tauri::command]
async fn queue_reorder(ids: Vec<String>, manager: State<'_, SharedManager>) -> Result<(), String> {
    manager.reorder(&ids).await
}

#[tauri::command]
async fn settings_get(manager: State<'_, SharedManager>) -> Result<AppSettings, String> {
    Ok(manager.settings().await)
}

#[tauri::command]
async fn settings_save(
    settings: AppSettings,
    manager: State<'_, SharedManager>,
    tray_items: State<'_, TrayMenuItems>,
) -> Result<(), String> {
    manager.save_settings(settings.clone()).await?;
    let _ = tray_items.clipboard.set_checked(settings.clipboard_monitor);
    let _ = tray_items.low_memory.set_checked(settings.low_memory_mode);
    let _ = tray_items.frosted_glass.set_checked(settings.frosted_glass);
    Ok(())
}

#[tauri::command]
async fn power_action_get(manager: State<'_, SharedManager>) -> Result<PowerActionState, String> {
    Ok(manager.power_action_state().await)
}

#[tauri::command]
async fn power_action_arm(
    action: PowerAction,
    manager: State<'_, SharedManager>,
) -> Result<PowerActionState, String> {
    manager.arm_power_action(action).await
}

#[tauri::command]
async fn power_action_cancel(
    manager: State<'_, SharedManager>,
) -> Result<PowerActionState, String> {
    manager.cancel_power_action().await
}

#[tauri::command]
fn log_to_backend(message: String) {
    println!("[FRONTEND LOG] {message}");
}

#[tauri::command]
async fn task_verify(id: String, manager: State<'_, SharedManager>) -> Result<String, String> {
    manager.verify_checksum(&id).await
}

#[tauri::command]
async fn task_precheck(
    request: PrecheckRequest,
    manager: State<'_, SharedManager>,
) -> Result<PrecheckResult, String> {
    manager.precheck(request).await
}

/// 检测新任务是否与已有任务重复（Task 10）。
///
/// 比对四类冲突：`SameUrl`、`SameFinalUrl`、`SameTargetPath`、`SameChecksum`。
/// URL 比对前会先剥离跟踪参数（utm_*、fbclid、gclid 等白名单）。
/// `file_size` 和 `sha256` 来自预检结果（可选），用于 `SameChecksum` 检测。
#[tauri::command]
async fn duplicate_check(
    url: String,
    target_path: String,
    file_size: Option<u64>,
    sha256: Option<String>,
    manager: State<'_, SharedManager>,
) -> Result<DuplicateCheckResult, String> {
    manager
        .check_duplicate(&url, &target_path, file_size, sha256.as_deref())
        .await
}

/// 诊断指定任务的最近一次错误。
///
/// 从 `task.error` 读取原始错误字符串，结合 `task.response_status` 和
/// 任务上下文（URL、ETag、代理、校验值）调用 `classify_error`，
/// 返回包含分类、中文说明、建议操作和脱敏原始错误的 `ErrorDiagnosis`。
///
/// 若任务不存在或无错误记录，返回 `Ok(None)`。
#[tauri::command]
async fn task_diagnose(
    id: String,
    manager: State<'_, SharedManager>,
) -> Result<Option<ErrorDiagnosis>, String> {
    let task = manager
        .store
        .get_task(&id)
        .await?
        .ok_or_else(|| "任务不存在".to_string())?;

    let Some(error) = task.error.as_deref() else {
        return Ok(None);
    };

    let settings = manager.settings().await;
    let is_proxy_used = settings.proxy_mode != "none";

    let context = ErrorContext::from_task_fields(
        task.final_url.clone().unwrap_or_else(|| task.url.clone()),
        task.etag.as_deref(),
        is_proxy_used,
        task.expected_checksum.as_deref(),
    );

    let diagnosis = manager::classify_error(error, task.response_status, &context);
    Ok(Some(diagnosis))
}

/// 队列调度可观察性（Task 15）：查询指定任务的等待原因。
///
/// 只读操作，不修改任何状态。前端在任务详情面板调用此命令，
/// 展示"为什么这个任务还没开始"。仅在任务处于 Queued/Scheduled 时主动调用，
/// 并通过 task-updated 事件驱动刷新，不引入轮询。
#[tauri::command]
async fn task_wait_reason(
    id: String,
    manager: State<'_, SharedManager>,
) -> Result<WaitReason, String> {
    manager.explain_wait_reason(&id).await
}

#[tauri::command]
async fn task_open_file(id: String, manager: State<'_, SharedManager>) -> Result<(), String> {
    let task = manager.store.get_task(&id).await?.ok_or("任务不存在")?;
    open::that(PathBuf::from(task.destination).join(task.file_name)).map_err(|e| e.to_string())
}

#[tauri::command]
async fn task_open_folder(id: String, manager: State<'_, SharedManager>) -> Result<(), String> {
    let task = manager.store.get_task(&id).await?.ok_or("任务不存在")?;
    open::that(task.destination).map_err(|e| e.to_string())
}

#[tauri::command]
async fn history_clear(
    include_completed: bool,
    manager: State<'_, SharedManager>,
) -> Result<(), String> {
    manager.store.clear_history(include_completed).await
}

#[tauri::command]
async fn pairing_info(pairing: State<'_, PairingService>) -> Result<PairingInfo, String> {
    pairing.info().await
}

#[tauri::command]
async fn pairing_rotate(pairing: State<'_, PairingService>) -> Result<PairingInfo, String> {
    Ok(pairing.rotate().await)
}

#[tauri::command]
async fn pairing_revoke(manager: State<'_, SharedManager>) -> Result<(), String> {
    manager.store.clear_pairing().await
}

/// Task 46：合并前端传入的凭证与数据库按域名存储的凭证。
///
/// 优先级：前端显式传入的非空值 > 数据库存储值 > None。
/// 任一字段在前端为 `None` 时，都会尝试从数据库按域名回填。
/// 解密失败（换机器/密文损坏）时安全降级为"无凭证"，不阻塞 probe/download；
/// 调用方不需要感知错误。
///
/// 不写日志，避免泄露 Cookie/Referer/UA 内容（AGENTS.md §3）。
async fn resolve_media_credentials(
    url: &str,
    cookie: Option<String>,
    referer: Option<String>,
    user_agent: Option<String>,
    store: &Arc<Store>,
) -> (Option<String>, Option<String>, Option<String>) {
    let Some(domain) = crate::media_cookies::extract_domain(url) else {
        return (cookie, referer, user_agent);
    };
    let stored = match store.media_credential_get_matching(&domain).await {
        Ok(Some(credential)) => credential,
        _ => return (cookie, referer, user_agent),
    };
    let merged_cookie = cookie.or_else(|| {
        if stored.cookie.is_empty() {
            None
        } else {
            Some(stored.cookie.clone())
        }
    });
    let merged_referer = referer.or(stored.referer);
    let merged_user_agent = user_agent.or(stored.user_agent);
    (merged_cookie, merged_referer, merged_user_agent)
}

#[tauri::command]
async fn media_probe(
    url: String,
    cookie: Option<String>,
    referer: Option<String>,
    user_agent: Option<String>,
    app: tauri::AppHandle,
    manager: State<'_, SharedManager>,
) -> Result<MediaProbeResult, String> {
    // Task 46：前端显式传入的凭证优先；缺失时回退到数据库按域名存储的凭证。
    // 解密失败（换机器/密文损坏）时安全降级为"无凭证"，不阻塞 probe。
    let (cookie, referer, user_agent) =
        resolve_media_credentials(&url, cookie, referer, user_agent, &manager.store).await;
    media::probe(
        &app,
        &manager.settings().await,
        &url,
        cookie.as_deref(),
        referer.as_deref(),
        user_agent.as_deref(),
    )
    .await
}

/// Task 37.1：识别 URL 所属媒体平台。
///
/// 返回平台名字符串（`"douyin"` / `"tiktok"` / `"twitter"` / `"youtube"` /
/// `"bilibili"` / `"weibo"` / `"unknown"`），前端用于在新建任务对话框展示
/// "检测到：抖音" 等提示，帮助用户预期下载行为。
///
/// 仅基于 URL host 模式匹配，不发起新的网络请求。`unknown` 不阻止 yt-dlp 通用流程。
#[tauri::command]
fn media_detect_platform(url: String) -> Result<String, String> {
    let platform = media_platforms::detect_platform(&url);
    Ok(platform.as_str().to_string())
}

/// Task 41：规范化用户输入的 URL（分享文本提取 + 短链跟随 + 跟踪参数剥离）。
///
/// 调用顺序与 [`media::probe`] 一致：
/// 1. [`media_platforms::extract_url_from_share_text`]：从分享文本中提取首个 URL。
///    纯 URL 输入原样返回；无 URL 返回中文错误。
/// 2. [`media_platforms::expand_short_url`]：若是已知短链域名，跟随 HTTP 302 到最终地址。
///    失败时回退到提取后的 URL，不阻断（前端预览仍可用）。
/// 3. [`media_platforms::strip_tracking_params`]：剥离 utm_* / fbclid / gclid 等跟踪参数。
///
/// 前端在新建任务对话框中调用此命令，展示"原文本 → 规范化 URL"预览，
/// 帮助用户确认分享文本已被正确解析（如抖音分享 → 抖音长链）。
///
/// 失败返回中文错误（如"未识别到有效链接"），不暴露内部异常细节（AGENTS.md §7）。
#[tauri::command]
async fn media_normalize_url(input: String) -> Result<String, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("链接不能为空".into());
    }
    let extracted = media_platforms::extract_url_from_share_text(trimmed)
        .ok_or_else(|| "未识别到有效链接".to_string())?;
    let parsed = url::Url::parse(&extracted).map_err(|_| "未识别到有效链接".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("仅支持 HTTP/HTTPS 链接".into());
    }
    // 短链跟随失败时回退到提取后的 URL，前端预览仍可显示原短链。
    let expanded = match media_platforms::expand_short_url(&extracted).await {
        Ok(final_url) => final_url,
        Err(_) => extracted.clone(),
    };
    let normalized = media_platforms::strip_tracking_params(&expanded);
    Ok(normalized)
}

#[tauri::command]
async fn media_tool_status(
    app: tauri::AppHandle,
    tools: State<'_, MediaTools>,
    manager: State<'_, SharedManager>,
) -> Result<ToolStatus, String> {
    Ok(tools.status(&app, &manager.settings().await).await)
}

#[tauri::command]
fn media_tools_detect_system() -> DetectedMediaTools {
    media_tools::detect_system_tools()
}

#[tauri::command]
async fn media_tools_install(
    app: tauri::AppHandle,
    tools: State<'_, MediaTools>,
    manager: State<'_, SharedManager>,
) -> Result<(), String> {
    let settings = manager.settings().await;
    let status = tools.status(&app, &settings).await;
    let component = if !status.yt_dlp_available {
        ToolComponent::YtDlp
    } else {
        ToolComponent::Ffmpeg
    };
    tools.start_install(app, settings, component).await
}

#[tauri::command]
async fn media_tool_install(
    component: ToolComponent,
    app: tauri::AppHandle,
    tools: State<'_, MediaTools>,
    manager: State<'_, SharedManager>,
) -> Result<(), String> {
    tools
        .start_install(app, manager.settings().await, component)
        .await
}

#[tauri::command]
async fn media_tools_cancel(tools: State<'_, MediaTools>) -> Result<(), String> {
    tools.cancel().await;
    Ok(())
}

#[tauri::command]
async fn media_tools_remove(
    app: tauri::AppHandle,
    tools: State<'_, MediaTools>,
    manager: State<'_, SharedManager>,
) -> Result<(), String> {
    let settings = manager.settings().await;
    tools
        .uninstall(&app, &settings, ToolComponent::Ffmpeg)
        .await?;
    tools.uninstall(&app, &settings, ToolComponent::YtDlp).await
}

#[tauri::command]
async fn media_tool_remove(
    component: ToolComponent,
    app: tauri::AppHandle,
    tools: State<'_, MediaTools>,
    manager: State<'_, SharedManager>,
) -> Result<(), String> {
    tools
        .uninstall(&app, &manager.settings().await, component)
        .await
}

#[tauri::command]
async fn media_tools_check_update(
    app: tauri::AppHandle,
    tools: State<'_, MediaTools>,
    manager: State<'_, SharedManager>,
) -> Result<ToolStatus, String> {
    Ok(tools.status(&app, &manager.settings().await).await)
}

/// Task 26.2 / 26.5：检查猫步下载器应用更新。
///
/// 调用 `updater::check_app_update` 通过 GitHub Releases API 拉取最新 release
/// 的版本号、发布时间、HTML 页面 URL 和 release notes。**只检查不自动下载**
/// （AGENTS.md §6：自动更新只能检查并提醒，不得后台自动下载）。
///
/// 失败时返回的 `UpdateCheckResult.error` 为脱敏后的中文错误，不暴露内部细节。
#[tauri::command]
async fn app_check_update() -> Result<UpdateCheckResult, String> {
    Ok(updater::check_app_update().await)
}

/// Task 26.3 / 26.6：检查浏览器扩展版本与桌面端兼容性。
///
/// 调用方传入扩展自报的版本字符串（前端可从配对状态或扩展 manifest 读出），
/// 与当前桌面端编译期版本（`updater::APP_VERSION`）比较。
/// 返回中文 `message` 指导用户更新扩展或桌面端，不直接修改任何状态。
#[tauri::command]
async fn extension_check_compatibility(
    ext_version: String,
) -> Result<ExtensionCompatibilityResult, String> {
    Ok(updater::build_extension_compatibility_result(
        updater::APP_VERSION,
        &ext_version,
    ))
}

#[tauri::command]
async fn category_rule_add(
    rule: CategoryRule,
    manager: State<'_, SharedManager>,
) -> Result<CategoryRule, String> {
    let mut normalized = rule;
    normalized.target_directory = normalize_directory(&normalized.target_directory);
    if normalized.target_directory.is_empty() {
        return Err("目标目录不能为空".into());
    }
    if normalized.name.trim().is_empty() {
        return Err("规则名称不能为空".into());
    }
    if normalized.pattern.trim().is_empty() {
        return Err("匹配模式不能为空".into());
    }
    if normalized.id.trim().is_empty() {
        return Err("规则 ID 不能为空".into());
    }
    manager.store.category_rule_add(normalized).await
}

#[tauri::command]
async fn category_rule_update(
    rule: CategoryRule,
    manager: State<'_, SharedManager>,
) -> Result<(), String> {
    let mut normalized = rule;
    normalized.target_directory = normalize_directory(&normalized.target_directory);
    if normalized.target_directory.is_empty() {
        return Err("目标目录不能为空".into());
    }
    if normalized.name.trim().is_empty() {
        return Err("规则名称不能为空".into());
    }
    if normalized.pattern.trim().is_empty() {
        return Err("匹配模式不能为空".into());
    }
    manager.store.category_rule_update(normalized).await
}

#[tauri::command]
async fn category_rule_delete(id: String, manager: State<'_, SharedManager>) -> Result<(), String> {
    manager.store.category_rule_delete(&id).await
}

#[tauri::command]
async fn category_rule_list(
    manager: State<'_, SharedManager>,
) -> Result<Vec<CategoryRule>, String> {
    manager.store.category_rule_list().await
}

/// 测试单条规则是否命中指定 URL/文件名/Content-Type（Task 11）。
///
/// 输入完整规则对象（前端编辑中的草稿也可以测试，无需先保存）。
/// 返回 `matched` 标志与目标目录（命中时为规则中保存的目录，未命中为空字符串）。
#[tauri::command]
async fn category_rule_test(
    rule: CategoryRule,
    url: String,
    file_name: String,
    content_type: Option<String>,
) -> Result<CategoryRuleTestResult, String> {
    let matched = test_category_rule(&rule, &url, &file_name, content_type.as_deref());
    Ok(CategoryRuleTestResult {
        matched,
        target_directory: if matched {
            normalize_directory(&rule.target_directory)
        } else {
            String::new()
        },
    })
}

/// 应用全部启用的分类规则到指定 URL/文件名/Content-Type，返回首个命中规则的目标目录（Task 11）。
#[tauri::command]
async fn category_rule_apply(
    url: String,
    file_name: String,
    content_type: Option<String>,
    manager: State<'_, SharedManager>,
) -> Result<Option<String>, String> {
    let rules = manager.store.category_rule_list().await?;
    Ok(
        apply_category_rules(&rules, &url, &file_name, content_type.as_deref())
            .map(|s| normalize_directory(&s)),
    )
}

// ===== Task 36: 任务模板 CRUD 与匹配测试 =====

/// 新增任务模板（Task 36）。
///
/// 校验：
/// - `id` 不能为空（前端生成 UUID）
/// - `name` 不能为空白
/// - `domain_pattern` 不能为空白
/// - `connections`（若设置）必须为 1/2/4/8/16/32 之一（AGENTS.md §3）
/// - `priority` 会被 clamp 到 `[MIN_PRIORITY, MAX_PRIORITY]`
/// - `destination`（若设置）会被规范化（去除尾部斜杠与首尾空白）；空字符串视为未设置
#[tauri::command]
async fn task_template_add(
    template: TaskTemplate,
    manager: State<'_, SharedManager>,
) -> Result<TaskTemplate, String> {
    let mut normalized = template;
    if normalized.id.trim().is_empty() {
        return Err("模板 ID 不能为空".into());
    }
    if normalized.name.trim().is_empty() {
        return Err("模板名称不能为空".into());
    }
    if normalized.domain_pattern.trim().is_empty() {
        return Err("域名匹配模式不能为空".into());
    }
    if let Some(conn) = normalized.connections {
        if !matches!(conn, 1 | 2 | 4 | 8 | 16 | 32) {
            return Err("连接数必须为 1/2/4/8/16/32 之一".into());
        }
    }
    normalized.priority = normalized
        .priority
        .clamp(crate::models::MIN_PRIORITY, crate::models::MAX_PRIORITY);
    if let Some(dest) = normalized.destination.as_deref() {
        let cleaned = normalize_directory(dest);
        normalized.destination = if cleaned.is_empty() {
            None
        } else {
            Some(cleaned)
        };
    }
    manager.store.task_template_add(normalized).await
}

/// 更新任务模板（Task 36）。所有字段都会被覆盖。
#[tauri::command]
async fn task_template_update(
    template: TaskTemplate,
    manager: State<'_, SharedManager>,
) -> Result<(), String> {
    let mut normalized = template;
    if normalized.name.trim().is_empty() {
        return Err("模板名称不能为空".into());
    }
    if normalized.domain_pattern.trim().is_empty() {
        return Err("域名匹配模式不能为空".into());
    }
    if let Some(conn) = normalized.connections {
        if !matches!(conn, 1 | 2 | 4 | 8 | 16 | 32) {
            return Err("连接数必须为 1/2/4/8/16/32 之一".into());
        }
    }
    normalized.priority = normalized
        .priority
        .clamp(crate::models::MIN_PRIORITY, crate::models::MAX_PRIORITY);
    if let Some(dest) = normalized.destination.as_deref() {
        let cleaned = normalize_directory(dest);
        normalized.destination = if cleaned.is_empty() {
            None
        } else {
            Some(cleaned)
        };
    }
    manager.store.task_template_update(normalized).await
}

/// 删除任务模板（Task 36）。
#[tauri::command]
async fn task_template_delete(id: String, manager: State<'_, SharedManager>) -> Result<(), String> {
    manager.store.task_template_delete(&id).await
}

/// 列出全部任务模板，按 priority 升序、name 升序返回（Task 36）。
#[tauri::command]
async fn task_template_list(
    manager: State<'_, SharedManager>,
) -> Result<Vec<TaskTemplate>, String> {
    manager.store.task_template_list().await
}

/// 测试给定 URL 是否命中任意模板（Task 36）。
///
/// 供前端在新建任务对话框展示"已匹配模板：xxx"提示。
/// URL 解析失败或无模板命中时返回 `matched = false`。
#[tauri::command]
async fn task_template_test(
    url: String,
    manager: State<'_, SharedManager>,
) -> Result<TaskTemplateTestResult, String> {
    let templates = manager.store.task_template_list().await?;
    Ok(test_task_template(&url, &templates))
}

/// Task 46：保存（新增或更新）一条媒体凭证。
///
/// `credential.cookie` 为明文，后端使用 DPAPI 加密后落库。
/// `domain` 不能为空；为空时返回中文错误。
/// `updated_at` 由调用方生成（前端在保存时填入 ISO 8601 UTC 字符串）。
#[tauri::command]
async fn media_credential_save(
    credential: MediaCredential,
    manager: State<'_, SharedManager>,
) -> Result<MediaCredential, String> {
    if credential.domain.trim().is_empty() {
        return Err("域名不能为空".into());
    }
    let mut normalized = credential;
    normalized.domain = normalized.domain.trim().to_string();
    normalized.referer = normalized
        .referer
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    normalized.user_agent = normalized
        .user_agent
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if normalized.updated_at.is_empty() {
        normalized.updated_at = now_iso8601_utc();
    }
    manager.store.media_credential_upsert(normalized).await
}

/// Task 46：按 domain 查询单条凭证。
///
/// 返回的 `cookie` 为解密后的明文。解密失败（换机器/密文损坏）时返回中文错误。
/// 不存在时返回 `None`。
#[tauri::command]
async fn media_credential_get(
    domain: String,
    manager: State<'_, SharedManager>,
) -> Result<Option<MediaCredential>, String> {
    manager.store.media_credential_get_matching(&domain).await
}

/// Task 46：按 domain 删除单条凭证。不存在不算错误（幂等）。
#[tauri::command]
async fn media_credential_delete(
    domain: String,
    manager: State<'_, SharedManager>,
) -> Result<(), String> {
    manager.store.media_credential_delete(&domain).await
}

/// Task 46：列出全部已存储的媒体凭证，按 domain 升序返回。
///
/// 任一行解密失败时该行被跳过（不阻塞列表返回）。
#[tauri::command]
async fn media_credential_list(
    manager: State<'_, SharedManager>,
) -> Result<Vec<MediaCredential>, String> {
    manager.store.media_credential_list().await
}

/// Task 44：列出全部平台兼容性记录，按 platform 升序返回。
///
/// 用于前端在设置页"关于 > 平台兼容性"子区域展示矩阵，
/// 以及新建任务对话框在检测到平台后查询对应支持级别。
/// 内置 6 条默认记录（YouTube/哔哩哔哩=Verified，
/// 抖音/TikTok/Twitter/微博=Experimental）由 `Store::open` 自动 seed。
#[tauri::command]
async fn platform_compatibility_list(
    manager: State<'_, SharedManager>,
) -> Result<Vec<PlatformCompatibility>, String> {
    manager.store.platform_compatibility_list().await
}

/// Task 44：按 platform 查询单条兼容性记录。不存在时返回 `None`。
///
/// `platform` 应为 `MediaPlatform` 序列化值（`"douyin"` / `"tiktok"` /
/// `"twitter"` / `"youtube"` / `"bilibili"` / `"weibo"` / `"unknown"`）。
/// 前端在新建任务对话框检测到平台后调用此命令展示徽章。
#[tauri::command]
async fn platform_compatibility_get(
    platform: String,
    manager: State<'_, SharedManager>,
) -> Result<Option<PlatformCompatibility>, String> {
    manager.store.platform_compatibility_get(&platform).await
}

/// Task 46：返回 ISO 8601 UTC 时间字符串（如 `2026-07-20T10:30:00Z`）。
///
/// 用于 `media_credential_save` 在调用方未提供 `updated_at` 时填充默认值。
pub(crate) fn now_iso8601_utc() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // 简单格式化：将 Unix 秒拆分为 Y-M-D H:M:S。
    // 不引入 chrono 依赖（AGENTS.md §8：可用标准库或现有依赖完成时不得新增依赖）。
    format_unix_seconds(secs)
}

/// Task 46：将 Unix 秒格式化为 ISO 8601 UTC 字符串。
///
/// 实现：基于公历日期算法（Howard Hinnant 的 days_from_civil），不依赖 chrono。
/// 仅用于 `updated_at` 展示，精度到秒足够。
fn format_unix_seconds(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let seconds_of_day = (secs % 86_400) as u64;
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    // days_from_civil 反推：以 1970-01-01 为 days=0。
    // 算法来自 Howard Hinnant "date" 库的 civil_from_days。
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146097)
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, m, d, hour, minute, second
    )
}

/// 列出全部文件名清理规则（Task 20）。
#[tauri::command]
async fn filename_cleanup_rule_list(
    manager: State<'_, SharedManager>,
) -> Result<Vec<FilenameCleanupRule>, String> {
    manager.store.filename_cleanup_rule_list().await
}

/// 新增文件名清理规则（Task 20）。
#[tauri::command]
async fn filename_cleanup_rule_add(
    rule: FilenameCleanupRule,
    manager: State<'_, SharedManager>,
) -> Result<FilenameCleanupRule, String> {
    validate_filename_cleanup_rule(&rule)?;
    manager.store.filename_cleanup_rule_add(rule).await
}

/// 更新文件名清理规则（Task 20）。内置规则可编辑，但不可删除（由前端控制）。
#[tauri::command]
async fn filename_cleanup_rule_update(
    rule: FilenameCleanupRule,
    manager: State<'_, SharedManager>,
) -> Result<(), String> {
    validate_filename_cleanup_rule(&rule)?;
    manager.store.filename_cleanup_rule_update(rule).await
}

/// 删除文件名清理规则（Task 20）。
#[tauri::command]
async fn filename_cleanup_rule_delete(
    id: String,
    manager: State<'_, SharedManager>,
) -> Result<(), String> {
    manager.store.filename_cleanup_rule_delete(&id).await
}

/// Task 43：列出全部平台命名模板。
///
/// 排序：platform 升序、enabled 降序（启用的在前）、id 升序。
/// 前端在设置页与下载流程中均使用此命令读取模板列表。
#[tauri::command]
async fn platform_naming_template_list(
    manager: State<'_, SharedManager>,
) -> Result<Vec<PlatformNamingTemplate>, String> {
    manager.store.platform_naming_template_list().await
}

/// Task 43：新增一条平台命名模板。
///
/// 校验：
/// - `platform` 不能为空，会 trim 后转小写存储（与 `MediaPlatform::as_str()` 一致）
/// - `template` 不能为空（trim 后非空）
/// - `id` 不能为空；同 id 已存在时返回中文错误
/// - `is_builtin` 由调用方决定，自定义模板应为 `false`
#[tauri::command]
async fn platform_naming_template_add(
    mut template: PlatformNamingTemplate,
    manager: State<'_, SharedManager>,
) -> Result<PlatformNamingTemplate, String> {
    validate_platform_naming_template(&template)?;
    template.platform = template.platform.trim().to_ascii_lowercase();
    template.template = template.template.trim().to_string();
    manager.store.platform_naming_template_add(template).await
}

/// Task 43：更新一条平台命名模板。
///
/// 校验与 `platform_naming_template_add` 一致。
/// `is_builtin` 字段由数据库既有值决定，前端不应修改此字段
/// （数据库 update 语句不修改 `is_builtin`，见 `Store::platform_naming_template_update`）。
#[tauri::command]
async fn platform_naming_template_update(
    mut template: PlatformNamingTemplate,
    manager: State<'_, SharedManager>,
) -> Result<(), String> {
    validate_platform_naming_template(&template)?;
    template.platform = template.platform.trim().to_ascii_lowercase();
    template.template = template.template.trim().to_string();
    manager
        .store
        .platform_naming_template_update(template)
        .await
}

/// Task 43：按 id 删除一条平台命名模板。
///
/// 内置模板（`is_builtin = true`）的删除保护在前端实现：
/// 设置页对内置模板隐藏"删除"按钮。后端不强制校验，以便未来需要时
/// 可以由命令层显式清理内置模板（如版本迁移）。
#[tauri::command]
async fn platform_naming_template_delete(
    id: String,
    manager: State<'_, SharedManager>,
) -> Result<(), String> {
    manager.store.platform_naming_template_delete(&id).await
}

/// Task 43：校验平台命名模板字段。
///
/// - `id`：非空（trim 后）
/// - `platform`：非空（trim 后），不强制校验是否为已知平台 key，
///   以便未来扩展新平台时旧前端不会拒绝写入
/// - `template`：非空（trim 后）
fn validate_platform_naming_template(template: &PlatformNamingTemplate) -> Result<(), String> {
    if template.id.trim().is_empty() {
        return Err("模板 ID 不能为空".into());
    }
    if template.platform.trim().is_empty() {
        return Err("平台不能为空".into());
    }
    if template.template.trim().is_empty() {
        return Err("模板内容不能为空".into());
    }
    Ok(())
}

/// 预览文件名清理结果（Task 20）。
///
/// 输入原始文件名与可选规则列表（不传则使用数据库中启用的规则），
/// 返回应用清理后的文件名字符串。用于新建任务对话框的实时预览。
#[tauri::command]
async fn filename_cleanup_preview(
    file_name: String,
    rules: Option<Vec<FilenameCleanupRule>>,
    manager: State<'_, SharedManager>,
) -> Result<String, String> {
    let rules = match rules {
        Some(list) => list,
        None => manager.store.filename_cleanup_rule_list().await?,
    };
    Ok(apply_filename_cleanup(&file_name, &rules))
}

/// 校验文件名清理规则字段。失败时返回中文错误。
fn validate_filename_cleanup_rule(rule: &FilenameCleanupRule) -> Result<(), String> {
    if rule.id.trim().is_empty() {
        return Err("规则 ID 不能为空".into());
    }
    if rule.name.trim().is_empty() {
        return Err("规则名称不能为空".into());
    }
    if rule.pattern.trim().is_empty() {
        return Err("正则模式不能为空".into());
    }
    // 校验正则可编译，避免保存后运行时反复跳过。
    if regex::Regex::new(&rule.pattern).is_err() {
        return Err("正则模式不合法".into());
    }
    Ok(())
}

/// 列出全部下载预设（内置 + 自定义），Task 12。
#[tauri::command]
async fn preset_list(manager: State<'_, SharedManager>) -> Result<Vec<DownloadPreset>, String> {
    manager.preset_list().await
}

/// 新增自定义下载预设（`is_builtin` 强制为 false），Task 12。
#[tauri::command]
async fn preset_add(
    preset: DownloadPreset,
    manager: State<'_, SharedManager>,
) -> Result<DownloadPreset, String> {
    manager.preset_add(preset).await
}

/// 更新下载预设字段。内置预设可编辑字段，但 `is_builtin` 不可改（Task 12）。
#[tauri::command]
async fn preset_update(
    preset: DownloadPreset,
    manager: State<'_, SharedManager>,
) -> Result<(), String> {
    manager.preset_update(preset).await
}

/// 删除下载预设。仅允许删除 `is_builtin = false` 的自定义预设（Task 12）。
#[tauri::command]
async fn preset_delete(id: String, manager: State<'_, SharedManager>) -> Result<(), String> {
    manager.preset_delete(&id).await
}

/// 把预设配置应用到现有任务（Task 12）。
///
/// 应用字段：连接数、单任务限速、完成动作、计划时间。
/// 仅在任务处于 Queued / Paused / Scheduled / Failed / Cancelled 时允许应用。
#[tauri::command]
async fn preset_apply_to_task(
    task_id: String,
    preset_id: String,
    manager: State<'_, SharedManager>,
) -> Result<DownloadTask, String> {
    manager.preset_apply_to_task(&task_id, &preset_id).await
}

/// 新增或更新一条 URL 历史（Task 19）。
///
/// - URL 在表内唯一：重复添加时仅更新 `last_used`（LRU 语义）。
/// - 仅校验非空；协议过滤由调用方负责。
#[tauri::command]
async fn url_history_add(url: String, manager: State<'_, SharedManager>) -> Result<(), String> {
    manager.store.url_history_add(&url).await
}

/// 列出最近 20 条 URL 历史，按 last_used 降序返回（Task 19）。
#[tauri::command]
async fn url_history_list(
    manager: State<'_, SharedManager>,
) -> Result<Vec<UrlHistoryEntry>, String> {
    manager.store.url_history_list().await
}

/// 清空全部 URL 历史（Task 19）。
#[tauri::command]
async fn url_history_clear(manager: State<'_, SharedManager>) -> Result<(), String> {
    manager.store.url_history_clear().await
}

// ===== Task 25: 标签 CRUD 与任务-标签关联命令 =====

/// 校验标签颜色为 `#RRGGBB` 格式的十六进制字符串。
fn validate_tag_color(color: &str) -> Result<(), String> {
    let bytes = color.as_bytes();
    if bytes.len() != 7 || bytes[0] != b'#' {
        return Err("颜色格式必须为 #RRGGBB".into());
    }
    for &b in &bytes[1..] {
        if !b.is_ascii_hexdigit() {
            return Err("颜色格式必须为 #RRGGBB".into());
        }
    }
    Ok(())
}

/// 新增用户标签（Task 25）。`name` 不能为空，`color` 必须为 `#RRGGBB` 格式。
#[tauri::command]
async fn tag_add(tag: Tag, manager: State<'_, SharedManager>) -> Result<Tag, String> {
    if tag.id.trim().is_empty() {
        return Err("标签 ID 不能为空".into());
    }
    if tag.name.trim().is_empty() {
        return Err("标签名称不能为空".into());
    }
    validate_tag_color(&tag.color)?;
    manager.store.tag_add(tag).await
}

/// 更新标签（Task 25）。`name` 重复时返回中文错误。
#[tauri::command]
async fn tag_update(tag: Tag, manager: State<'_, SharedManager>) -> Result<(), String> {
    if tag.name.trim().is_empty() {
        return Err("标签名称不能为空".into());
    }
    validate_tag_color(&tag.color)?;
    manager.store.tag_update(tag).await
}

/// 删除标签（Task 25）。关联的 task_tags 由外键级联清理。
#[tauri::command]
async fn tag_delete(id: String, manager: State<'_, SharedManager>) -> Result<(), String> {
    manager.store.tag_delete(&id).await
}

/// 列出全部标签，按 name 升序排列（Task 25）。
#[tauri::command]
async fn tag_list(manager: State<'_, SharedManager>) -> Result<Vec<Tag>, String> {
    manager.store.tag_list().await
}

/// 替换任务的全部标签关联（Task 25）。
/// `tag_ids` 中不存在的 tag_id 会因外键约束失败。
#[tauri::command]
async fn task_tags_set(
    task_id: String,
    tag_ids: Vec<String>,
    manager: State<'_, SharedManager>,
) -> Result<(), String> {
    manager.store.task_tags_set(&task_id, tag_ids).await
}

/// 获取单个任务的标签列表（Task 25）。
#[tauri::command]
async fn task_tags_get(
    task_id: String,
    manager: State<'_, SharedManager>,
) -> Result<Vec<Tag>, String> {
    manager.store.task_tags_get(&task_id).await
}

/// 列出全部任务-标签关联，按 task_id 分组返回 HashMap（Task 25）。
#[tauri::command]
async fn task_tags_list_all(
    manager: State<'_, SharedManager>,
) -> Result<std::collections::HashMap<String, Vec<Tag>>, String> {
    manager.store.task_tags_list_all().await
}

/// Task 32.1 / 32.4：立即检测当前网络是否为计量网络。
///
/// 前端"立即检查"按钮调用此命令；返回 `true` 表示当前为计量网络。
/// 失败时返回 `false`（安全回退），不向上层抛错。
#[tauri::command]
async fn network_check_metered() -> Result<bool, String> {
    Ok(network_awareness::detect_metered_network()
        .await
        .unwrap_or(false))
}

#[tauri::command]
fn app_exit(app: tauri::AppHandle) {
    app.exit(0);
}

/// 打开日志目录（Task 23.3）。
///
/// 使用系统默认文件管理器打开日志目录。
/// 路径解析与 `setup` 中一致：环境变量 > 便携标记 > `app_data_dir`。
/// 路径不返回前端，避免暴露绝对路径。
#[tauri::command]
async fn open_logs_dir(app: tauri::AppHandle) -> Result<(), String> {
    let data_dir = portable::resolve_data_dir(&app);
    let logs_dir = data_dir.join("logs");
    if !logs_dir.exists() {
        std::fs::create_dir_all(&logs_dir).map_err(|e| format!("无法创建日志目录：{e}"))?;
    }
    open::that(&logs_dir).map_err(|e| format!("无法打开日志目录：{e}"))
}

/// 导出最近 24 小时日志（Task 23.4）。
///
/// 读取日志目录下最近 24 小时修改过的 `maobu.log*` 文件，
/// 对每行再次调用 `redact_sensitive`（双保险），拼接后写入 `output_path`。
///
/// 路径解析与 `setup` 中一致：优先使用 `logging::log_dir()`（已初始化的全局目录），
/// 回退到 `portable::resolve_data_dir` 推导的 logs 子目录。
///
/// 输出为纯文本（不引入 zip 依赖，保持紧凑），文件名建议使用 `.log` 后缀。
#[tauri::command]
async fn export_recent_logs(output_path: String, app: tauri::AppHandle) -> Result<usize, String> {
    let logs_dir = match logging::log_dir() {
        Some(dir) => dir.to_path_buf(),
        None => {
            // 日志系统未初始化（异常路径），回退到 portable::resolve_data_dir/logs
            let data_dir = portable::resolve_data_dir(&app);
            let logs_dir = data_dir.join("logs");
            if !logs_dir.exists() {
                return Err("日志目录不存在，请先运行应用一段时间生成日志".into());
            }
            logs_dir
        }
    };

    let files = logging::recent_log_files_in(&logs_dir, 24)?;
    if files.is_empty() {
        return Err("最近 24 小时内没有日志文件".into());
    }
    let count = files.len();
    logging::write_logs_to_file(&files, std::path::Path::new(&output_path))?;
    Ok(count)
}

// ===== Task 29：maobu:// 深链与 .maobu-task 文件关联 =====

/// `deep-link-received` 事件 payload。
///
/// - `add`：携带 `url`，前端打开新建任务对话框并预填。
/// - `import`：携带 `count`，前端显示导入成功 toast。
#[derive(Clone, serde::Serialize)]
struct DeepLinkReceivedPayload {
    action: &'static str,
    url: Option<String>,
    count: Option<usize>,
}

// ===== Task 34：便携版 / 应用信息 =====

/// 应用信息（Task 34.3）。
///
/// 由 `app_get_info` 命令返回，前端用于在设置页"关于"分组显示便携模式状态。
/// - `version`：编译期 `CARGO_PKG_VERSION`
/// - `portable_mode`：便携模式是否生效（环境变量覆盖时不视为便携）
/// - `data_dir`：当前生效的数据目录绝对路径
#[derive(Clone, serde::Serialize)]
struct AppInfo {
    version: &'static str,
    portable_mode: bool,
    data_dir: String,
}

/// 返回应用信息（Task 34.3）。
///
/// 前端在设置页"关于"分组调用此命令，便携模式启用时显示醒目提示
/// "便携模式已启用，数据存储于 EXE 同目录"。
///
/// `data_dir` 返回绝对路径字符串，仅用于在关于页展示，不用于其他用途。
#[tauri::command]
fn app_get_info(app: tauri::AppHandle) -> Result<AppInfo, String> {
    let data_dir = portable::resolve_data_dir(&app);
    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        portable_mode: portable::is_portable_mode_effective(),
        data_dir: data_dir.to_string_lossy().to_string(),
    })
}

/// 注册 `maobu://` 深链处理器。
///
/// 同时处理两种入口：
/// 1. `deep-link://new-url` 事件：程序已运行时，用户点击 `maobu://` 链接（由系统转发到运行中的实例）。
/// 2. `get_current`：程序通过 `maobu://` 链接冷启动时，URL 由安装器写入命令行。
///
/// `add` 动作只把 URL 透传到前端 `deep-link-received` 事件，由 NewTaskDialog 让用户确认；
/// `pause`/`resume` 直接调用 manager.action，失败时发 `deep-link-error`。
fn register_deep_link_handler(app: &tauri::AppHandle, manager: SharedManager) {
    use tauri_plugin_deep_link::DeepLinkExt;

    // 在 Windows/Linux 上动态注册 maobu:// 协议以支持开发/便携模式下的深链唤醒。
    let _ = app.deep_link().register("maobu");

    // 监听运行时 deep-link 事件（插件 emit "deep-link://new-url"，payload 为 Vec<url::Url>）。
    let event_app = app.clone();
    let event_manager = manager.clone();
    app.listen("deep-link://new-url", move |event| {
        let payload = event.payload();
        let urls: Vec<String> = match serde_json::from_str(payload) {
            Ok(urls) => urls,
            Err(_) => {
                let _ = event_app.emit("deep-link-error", "深链事件 payload 格式无效".to_string());
                return;
            }
        };
        for uri in urls {
            dispatch_deep_link(&event_app, &event_manager, &uri);
        }
    });

    // 处理冷启动时通过 maobu:// 链接启动的情况。
    let current_app = app.clone();
    let current_manager = manager.clone();
    if let Ok(Some(urls)) = app.deep_link().get_current() {
        for url in urls {
            let uri = url.to_string();
            dispatch_deep_link(&current_app, &current_manager, &uri);
        }
    }
}

/// 解析并分发单个 `maobu://` URI。
fn dispatch_deep_link(app: &tauri::AppHandle, manager: &SharedManager, uri: &str) {
    match parse_deep_link(uri) {
        Ok(DeepLinkAction::Add { url }) => {
            let _ = app.emit(
                "deep-link-received",
                DeepLinkReceivedPayload {
                    action: "add",
                    url: Some(url),
                    count: None,
                },
            );
        }
        Ok(DeepLinkAction::Pause { id }) => {
            let app = app.clone();
            let manager = manager.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = manager.action(&id, "pause").await {
                    let _ = app.emit("deep-link-error", error);
                }
            });
        }
        Ok(DeepLinkAction::Resume { id }) => {
            let app = app.clone();
            let manager = manager.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = manager.action(&id, "resume").await {
                    let _ = app.emit("deep-link-error", error);
                }
            });
        }
        Err(error) => {
            let _ = app.emit("deep-link-error", error);
        }
    }
}

/// 处理 `.maobu-task` 文件双击。
///
/// Windows 在文件关联双击时把文件路径作为 argv 传入。仅处理第一个匹配的文件，
/// 避免一次导入多个文件造成困惑。导入使用用户当前的 `download_dir` 作为目标目录。
async fn handle_maobu_task_file(app: &tauri::AppHandle, manager: &SharedManager) {
    let target = match std::env::args().skip(1).find(|arg| {
        std::path::Path::new(arg)
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("maobu-task"))
    }) {
        Some(path) => path,
        None => return,
    };

    let path = std::path::PathBuf::from(&target);
    if !path.exists() {
        let _ = app.emit("deep-link-error", format!("任务文件不存在：{target}"));
        return;
    }
    if !path.is_absolute() {
        let _ = app.emit(
            "deep-link-error",
            format!("任务文件路径必须是绝对路径：{target}"),
        );
        return;
    }

    let settings = manager.settings().await;
    let destination = settings.download_dir.clone();
    if destination.trim().is_empty() {
        let _ = app.emit(
            "deep-link-error",
            "请先在设置中指定默认下载目录".to_string(),
        );
        return;
    }

    match manager.import_tasks(&target, &destination).await {
        Ok(tasks) => {
            let _ = app.emit(
                "deep-link-received",
                DeepLinkReceivedPayload {
                    action: "import",
                    url: None,
                    count: Some(tasks.len()),
                },
            );
        }
        Err(error) => {
            let _ = app.emit("deep-link-error", format!("导入任务失败：{error}"));
        }
    }
}

// ===== Task 28：系统托盘进度显示 =====
//
// 设计要点（AGENTS.md §3 §8）：
// - 事件驱动：监听 `task-updated` / `task-created` / `task-removed` 事件触发更新，
//   不引入轮询。
// - 节流：同一秒内最多触发一次即时更新；如果事件继续到来，安排一次延迟更新，
//   保证下载完成后最终状态会被刷新到托盘。
// - 真实状态：进度、ETA 与活动任务数来自 `manager.list()`，不使用模拟数据。
// - 安全：`unwrap`/`expect` 仅用于不可恢复的不变量；网络/DB 错误返回 `Result`。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
use std::time::{Duration as StdDuration, SystemTime, UNIX_EPOCH};

/// 托盘进度更新的最小间隔（毫秒）。与原 `manager::update_tray_tooltip` 节流一致，
/// 避免 250ms 报告循环在多任务并发下刷屏。
const TRAY_UPDATE_THROTTLE_MS: u64 = 1_000;

static TRAY_LAST_UPDATE_MS: AtomicU64 = AtomicU64::new(0);
static TRAY_DEFERRED_PENDING: AtomicBool = AtomicBool::new(false);

fn now_millis_for_tray() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 异步更新托盘图标与 tooltip。
///
/// - 读取 `manager.list()` 获取全部任务（事件已先持久化到 DB）。
/// - 调用 `tray_icon::compute_tray_progress` 聚合进度。
/// - 调用 `tray_icon::format_tray_tooltip` 生成 tooltip 文本。
/// - 调用 `tray_icon::render_progress_icon` 在基础图标上叠加百分比徽章。
///
/// 错误（DB 查询失败、图标生成失败）会被吞掉并回退到默认图标，
/// 因为托盘失败不应影响下载流程（AGENTS.md §7）。
async fn update_tray_progress(app: &tauri::AppHandle, manager: &SharedManager) {
    let Some(tray) = app.tray_by_id("main-tray") else {
        // 托盘尚未构建（启动早期）或构建失败。
        return;
    };
    let tasks = match manager.list().await {
        Ok(list) => list,
        Err(_) => return,
    };
    let progress = tray_icon::compute_tray_progress(&tasks);
    let tooltip = tray_icon::format_tray_tooltip(&progress);
    let _ = tray.set_tooltip(Some(tooltip));

    let Some(base) = app.default_window_icon() else {
        // 无基础图标，仅更新 tooltip。
        return;
    };
    if let Err(_err) = tray.set_icon(Some(base.clone())) {
        // 设置失败时静默回退，避免阻断核心下载流程
    }
}

/// 节流触发托盘更新。
///
/// - 若距离上次更新 ≥ `TRAY_UPDATE_THROTTLE_MS`：立即异步更新。
/// - 否则：安排一次延迟任务，确保事件流尾部状态最终会被刷新。
/// - 同时只允许一个延迟任务挂起（`TRAY_DEFERRED_PENDING` 去重）。
fn schedule_tray_update(app: tauri::AppHandle, manager: SharedManager) {
    let now = now_millis_for_tray();
    let last = TRAY_LAST_UPDATE_MS.load(AtomicOrdering::Relaxed);
    if now.saturating_sub(last) >= TRAY_UPDATE_THROTTLE_MS {
        TRAY_LAST_UPDATE_MS.store(now, AtomicOrdering::Relaxed);
        let app_clone = app.clone();
        let manager_clone = manager.clone();
        tauri::async_runtime::spawn(async move {
            update_tray_progress(&app_clone, &manager_clone).await;
        });
        return;
    }
    if TRAY_DEFERRED_PENDING.swap(true, AtomicOrdering::SeqCst) {
        return;
    }
    let app_clone = app.clone();
    let manager_clone = manager.clone();
    tauri::async_runtime::spawn(async move {
        let elapsed_since_last = now_millis_for_tray().saturating_sub(last);
        let delay_ms = TRAY_UPDATE_THROTTLE_MS
            .saturating_sub(elapsed_since_last)
            .max(100);
        tokio::time::sleep(StdDuration::from_millis(delay_ms)).await;
        TRAY_DEFERRED_PENDING.store(false, AtomicOrdering::SeqCst);
        update_tray_progress(&app_clone, &manager_clone).await;
        TRAY_LAST_UPDATE_MS.store(now_millis_for_tray(), AtomicOrdering::Relaxed);
    });
}

/// 注册托盘进度更新的事件监听（Task 28）。
///
/// 监听 `task-updated` / `task-created` / `task-removed` 三类事件，
/// 通过 `schedule_tray_update` 触发节流更新。监听器在 `setup` 中注册，
/// 保证任务状态变化能反映到托盘图标与 tooltip。
fn register_tray_progress_listener(app: &tauri::AppHandle, manager: SharedManager) {
    for event in ["task-updated", "task-created", "task-removed"] {
        let app_clone = app.clone();
        let manager_clone = manager.clone();
        app.listen_any(event, move |_event| {
            schedule_tray_update(app_clone.clone(), manager_clone.clone());
        });
    }
}

// ===== Task 32：网络环境感知（计量网络检测 + 自动暂停）=====

/// `metered-network-detected` 事件 payload。
///
/// 后端在 `pause_tasks_for_metered_network` 实际暂停了 ≥1 个任务时 emit。
/// 前端据此弹出 toast "当前为计量网络，已暂停 N 个任务"。
#[derive(Clone, serde::Serialize)]
struct MeteredNetworkDetectedPayload {
    /// 本次被自动暂停的任务数（仅统计从 Downloading 切到 PausedByMetered 的任务）。
    paused_count: usize,
}

/// 计量网络定时检查间隔。60 秒一次不属于高频轮询（AGENTS.md §8）。
const METERED_CHECK_INTERVAL_SECS: u64 = 60;

/// 启动计量网络定时检查（Task 32.1 / 32.2 / 32.3）。
///
/// 每 60 秒执行一次：
/// 1. 调用 `network_awareness::detect_metered_network()` 读取系统网络计费状态。
/// 2. 读取 `AppSettings::metered_auto_pause` 与 `user_resumed_after_metered`，
///    通过 `should_pause_for_metered` 决定是否应自动暂停。
/// 3. 满足条件时调用 `pause_tasks_for_metered_network`，若实际暂停任务数 > 0
///    则 emit `metered-network-detected` 事件，前端展示 toast。
/// 4. 网络从计量切换为非计量时，调用 `clear_user_resumed_after_metered`
///    清零用户标记，使下次进入计量网络时仍能触发自动暂停。
///
/// 错误处理（AGENTS.md §7）：
/// - 检测失败：安全回退到非计量（`detect_metered_network` 已封装）。
/// - 暂停失败：记录到日志但不中断定时循环。
/// - 清零标记失败：同样不中断循环，下次检查会重试。
fn spawn_metered_network_check(app: tauri::AppHandle, manager: SharedManager) {
    tauri::async_runtime::spawn(async move {
        // 上一次检查时是否处于计量网络。用于检测"计量 → 非计量"的状态转换，
        // 触发 clear_user_resumed_after_metered。
        let mut was_metered = false;
        loop {
            tokio::time::sleep(StdDuration::from_secs(METERED_CHECK_INTERVAL_SECS)).await;

            let is_metered = network_awareness::detect_metered_network()
                .await
                .unwrap_or(false);

            let settings = manager.settings().await;
            let should_pause = network_awareness::should_pause_for_metered(
                settings.metered_auto_pause,
                is_metered,
                settings.user_resumed_after_metered,
            );

            if should_pause {
                match manager.pause_tasks_for_metered_network().await {
                    Ok(count) if count > 0 => {
                        let _ = app.emit(
                            "metered-network-detected",
                            MeteredNetworkDetectedPayload {
                                paused_count: count,
                            },
                        );
                    }
                    Ok(_) => {
                        // 没有正在下载的任务，无需 emit 事件。
                    }
                    Err(err) => {
                        tracing::warn!("计量网络自动暂停失败：{err}");
                    }
                }
            }

            // 状态转换：计量 → 非计量，清零用户标记，使下次计量时仍能自动暂停。
            if was_metered && !is_metered {
                if let Err(err) = manager.clear_user_resumed_after_metered().await {
                    tracing::warn!("清零 user_resumed_after_metered 失败：{err}");
                }
            }
            was_metered = is_metered;
        }
    });
}

// ===== Task 35：命令行接口执行 =====
//
// 设计要点（AGENTS.md §3 §7 §8）：
// - CLI 子命令在无 GUI 环境下执行，直接操作 Store（SQLite）。
// - 不调用 `unwrap()` / `expect()`，所有可恢复错误返回中文错误信息。
// - 认证信息（Cookie/Authorization/代理密码）不写入日志或输出。
// - `list` 输出表格（ID | 状态 | 文件名 | 进度 | 速度），进度与速度来自真实 DB 状态。
// - 当 GUI 正在运行时，CLI 写操作（pause/resume/remove）更新 DB，
//   GUI 调度器会在下次循环感知到状态变化。`add` 创建的任务会被 GUI 调度器拾取。

/// 返回当前时间的毫秒时间戳（UNIX_EPOCH）。
fn cli_now_millis() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 把文件名中的 Windows 非法字符替换为 `_`，并限制长度。
fn cli_safe_name(input: &str) -> String {
    let value: String = input
        .chars()
        .map(|c| {
            if "<>:\"/\\|?*".contains(c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let value = value.trim_matches([' ', '.']);
    if value.is_empty() {
        "download".into()
    } else {
        value.chars().take(180).collect()
    }
}

/// 根据文件扩展名推断分类（与 manager::category 一致）。
fn cli_category(name: &str) -> String {
    match std::path::Path::new(name)
        .extension()
        .and_then(|x| x.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "mp4" | "mkv" | "mov" | "webm" | "m3u8" => "video",
        "mp3" | "wav" | "flac" | "aac" | "m4a" => "audio",
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "svg" => "images",
        "zip" | "rar" | "7z" | "tar" | "gz" => "archives",
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "txt" => "documents",
        "exe" | "msi" | "dmg" | "pkg" | "appimage" => "apps",
        _ => "other",
    }
    .into()
}

/// 把速度（字节/秒）格式化为人类可读字符串。
fn cli_format_speed(bytes_per_sec: u64) -> String {
    if bytes_per_sec == 0 {
        return "0 B/s".into();
    }
    const UNITS: [&str; 4] = ["B/s", "KB/s", "MB/s", "GB/s"];
    let mut value = bytes_per_sec as f64;
    let mut unit_idx = 0;
    while value >= 1024.0 && unit_idx < UNITS.len() - 1 {
        value /= 1024.0;
        unit_idx += 1;
    }
    if unit_idx == 0 {
        format!("{} {}", bytes_per_sec, UNITS[0])
    } else {
        format!("{:.1} {}", value, UNITS[unit_idx])
    }
}

/// 解析 CLI 模式下的数据目录（无 AppHandle）。
///
/// 优先级与 `portable::resolve_data_dir` 一致：
/// 1. 环境变量 `MAOBU_FETCH_DATA_DIR` / `LUMAGET_DATA_DIR`
/// 2. 便携模式 `EXE_DIR/data/`
/// 3. 平台默认 `%APPDATA%/app.lumaget.desktop`（Windows）等
fn cli_resolve_data_dir() -> Result<PathBuf, String> {
    if let Some(dir) = portable::current_data_dir() {
        return Ok(dir);
    }
    // 平台默认：与 Tauri app_data_dir 行为一致。
    // 标识符 `app.lumaget.desktop` 是兼容标识，不得修改（AGENTS.md §2）。
    #[cfg(windows)]
    {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            return Ok(PathBuf::from(appdata).join("app.lumaget.desktop"));
        }
        return Err("无法确定应用数据目录：APPDATA 环境变量未设置".into());
    }
    #[cfg(not(windows))]
    {
        if let Some(home) = std::env::var_os("HOME") {
            return Ok(PathBuf::from(home)
                .join(".config")
                .join("app.lumaget.desktop"));
        }
        return Err("无法确定应用数据目录：HOME 环境变量未设置".into());
    }
}

/// 打开 CLI 模式下的 Store。
fn cli_open_store() -> Result<Arc<Store>, String> {
    let data_dir = cli_resolve_data_dir()?;
    std::fs::create_dir_all(&data_dir).map_err(|e| format!("无法创建数据目录：{e}"))?;
    let store = Store::open(data_dir)?;
    Ok(Arc::new(store))
}

/// 执行 `add` 子命令：创建并插入一个新任务。
async fn cli_add(
    store: &Arc<Store>,
    url: String,
    out: Option<String>,
    connections: Option<u8>,
) -> Result<String, String> {
    let parsed =
        url::Url::parse(url.trim()).map_err(|_| "请输入有效的 HTTP/HTTPS 链接".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("仅支持 HTTP/HTTPS 链接".into());
    }

    let settings = store.get_settings().await?;

    // 从 URL 推导默认文件名
    let url_filename = parsed
        .path_segments()
        .and_then(|mut s| s.next_back())
        .filter(|s| !s.is_empty())
        .unwrap_or("download");

    // 根据 --out 决定文件名与目标目录
    let (file_name, destination) = match out.as_deref() {
        Some(out_path) => {
            let path = std::path::Path::new(out_path);
            let is_dir = path.is_dir()
                || out_path.ends_with('/')
                || out_path.ends_with('\\')
                || out_path.ends_with(std::path::MAIN_SEPARATOR);
            if is_dir {
                let dir = normalize_directory(out_path);
                let fname = cli_safe_name(url_filename);
                (fname, dir)
            } else {
                let fname = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(cli_safe_name)
                    .unwrap_or_else(|| cli_safe_name(url_filename));
                let dir = path
                    .parent()
                    .and_then(|p| p.to_str())
                    .filter(|s| !s.is_empty())
                    .map(normalize_directory)
                    .unwrap_or_else(|| settings.download_dir.clone());
                (fname, dir)
            }
        }
        None => {
            let fname = cli_safe_name(url_filename);
            (fname, settings.download_dir.clone())
        }
    };

    let id = uuid::Uuid::new_v4().to_string();
    let connection_count = connections
        .unwrap_or(settings.connections_per_download)
        .clamp(1, 32);
    // 在 file_name move 之前计算 category，避免 E0382。
    let category = cli_category(&file_name);

    let task = DownloadTask {
        id: id.clone(),
        url: parsed.to_string(),
        file_name,
        destination,
        total_bytes: 0,
        downloaded_bytes: 0,
        speed: 0,
        eta_seconds: None,
        status: models::TaskStatus::Queued,
        error: None,
        created_at: cli_now_millis(),
        completed_at: None,
        scheduled_at: None,
        category,
        queue_position: store.next_queue_position().await?,
        priority: 0,
        retry_count: 0,
        max_retries: settings.max_retries,
        checksum_sha256: None,
        expected_checksum: None,
        source: "cli".into(),
        etag: None,
        last_modified: None,
        final_url: None,
        response_status: None,
        content_type: None,
        accepts_ranges: None,
        headers: std::collections::HashMap::new(),
        media: None,
        per_task_speed_limit: 0,
        collision_policy: CollisionPolicy::Rename,
        completion_action: CompletionAction::None,
        connection_count,
        active_connections: 0,
        segments: Vec::new(),
        retry_policy_override: None,
        proxy_override: None,
        proxy_auth: None,
    };

    store.upsert_task(&task).await?;
    Ok(id)
}

/// 执行 `list` 子命令：打印任务表格。
async fn cli_list(store: &Arc<Store>, status_filter: Option<String>) -> Result<(), String> {
    let tasks = store.list_tasks().await?;
    let filtered: Vec<&DownloadTask> = match status_filter.as_deref() {
        Some(filter) if !filter.is_empty() => tasks
            .iter()
            .filter(|t| t.status.as_str() == filter)
            .collect(),
        _ => tasks.iter().collect(),
    };

    if filtered.is_empty() {
        println!("没有匹配的任务");
        return Ok(());
    }

    // 表头
    println!(
        "{:<36} {:<14} {:<30} {:>8} {:>12}",
        "ID", "状态", "文件名", "进度", "速度"
    );
    println!("{}", "-".repeat(104));

    for task in filtered {
        let progress = if task.total_bytes > 0 {
            format!(
                "{:.1}%",
                (task.downloaded_bytes as f64 / task.total_bytes as f64) * 100.0
            )
        } else {
            "?".to_string()
        };
        let speed = cli_format_speed(task.speed);
        // 截断过长的文件名以保持表格对齐
        let truncated_name: String = task.file_name.chars().take(28).collect();
        println!(
            "{:<36} {:<14} {:<30} {:>8} {:>12}",
            task.id,
            task.status.as_str(),
            truncated_name,
            progress,
            speed
        );
    }
    Ok(())
}

/// 执行 `pause` / `resume` 子命令：更新任务状态。
async fn cli_action(store: &Arc<Store>, id: &str, action: &str) -> Result<(), String> {
    let mut task = store
        .get_task(id)
        .await?
        .ok_or_else(|| "任务不存在".to_string())?;

    match action {
        "pause" => {
            task.status = models::TaskStatus::Paused;
            task.speed = 0;
            task.eta_seconds = None;
            task.active_connections = 0;
        }
        "resume" => {
            if task.status == models::TaskStatus::Completed {
                return Ok(());
            }
            task.status = if task.scheduled_at.is_some_and(|t| t > cli_now_millis()) {
                models::TaskStatus::Scheduled
            } else {
                models::TaskStatus::Queued
            };
            task.error = None;
            task.active_connections = 0;
        }
        _ => return Err(format!("未知操作：{action}")),
    }

    store.upsert_task(&task).await?;
    Ok(())
}

/// 执行 `remove` 子命令：删除任务记录，可选删除文件。
async fn cli_remove(store: &Arc<Store>, id: &str, delete_file: bool) -> Result<(), String> {
    let task = store
        .get_task(id)
        .await?
        .ok_or_else(|| "任务不存在".to_string())?;

    let is_completed = task.status == models::TaskStatus::Completed;
    if delete_file || !is_completed {
        let path = std::path::PathBuf::from(&task.destination).join(&task.file_name);
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
        // 同时清理 .lumaget 临时文件
        let temp_path = std::path::PathBuf::from(format!("{}.lumaget", path.to_string_lossy()));
        if temp_path.exists() {
            let _ = std::fs::remove_file(&temp_path);
        }
    }

    store.remove_task(id).await?;
    Ok(())
}

/// CLI 入口：执行子命令并返回退出码。
///
/// 由 `main.rs` 调用。`Run` 变体不应到达此函数（main.rs 直接启动 GUI）。
/// 其他变体打开 Store、执行操作、打印结果到 stdout/stderr，返回退出码。
pub fn run_cli(command: CliCommand) -> i32 {
    match command {
        CliCommand::Run => {
            // 不应到达此处；main.rs 会直接调用 run()。
            run();
            0
        }
        CliCommand::Add {
            url,
            out,
            connections,
        } => {
            let store = match cli_open_store() {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("{e}");
                    return 1;
                }
            };
            match tauri::async_runtime::block_on(cli_add(&store, url, out, connections)) {
                Ok(id) => {
                    println!("Task created: {id}");
                    0
                }
                Err(e) => {
                    eprintln!("{e}");
                    1
                }
            }
        }
        CliCommand::List { status } => {
            let store = match cli_open_store() {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("{e}");
                    return 1;
                }
            };
            match tauri::async_runtime::block_on(cli_list(&store, status)) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("{e}");
                    1
                }
            }
        }
        CliCommand::Pause { id } => {
            let store = match cli_open_store() {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("{e}");
                    return 1;
                }
            };
            match tauri::async_runtime::block_on(cli_action(&store, &id, "pause")) {
                Ok(()) => {
                    println!("OK");
                    0
                }
                Err(e) => {
                    eprintln!("{e}");
                    1
                }
            }
        }
        CliCommand::Resume { id } => {
            let store = match cli_open_store() {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("{e}");
                    return 1;
                }
            };
            match tauri::async_runtime::block_on(cli_action(&store, &id, "resume")) {
                Ok(()) => {
                    println!("OK");
                    0
                }
                Err(e) => {
                    eprintln!("{e}");
                    1
                }
            }
        }
        CliCommand::Remove { id, delete_file } => {
            let store = match cli_open_store() {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("{e}");
                    return 1;
                }
            };
            match tauri::async_runtime::block_on(cli_remove(&store, &id, delete_file)) {
                Ok(()) => {
                    println!("OK");
                    0
                }
                Err(e) => {
                    eprintln!("{e}");
                    1
                }
            }
        }
    }
}

/// 单实例回调：第二个实例启动时把 argv 转发到运行中的实例（Task 35.5）。
///
/// 当用户在 GUI 已运行时执行 CLI 命令（如 `maobu add <url>`），
/// `tauri-plugin-single-instance` 会把 argv 转发到首个实例并退出第二个实例。
/// 此回调解析转发的 argv 并在运行中的 manager 上执行。
///
/// 限制：CLI 输出会写入首个实例的 stdout；Windows 发布构建（GUI 子系统）
/// 检查当前是否有正在运行的 GUI 实例。
pub fn is_gui_running() -> bool {
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;

    if let Ok(addr) = "127.0.0.1:17433".parse::<SocketAddr>() {
        TcpStream::connect_timeout(&addr, Duration::from_millis(150)).is_ok()
    } else {
        false
    }
}

fn handle_single_instance_forward(app: &tauri::AppHandle, argv: Vec<String>) {
    // 检查 argv 是否包含 deep link 或 .maobu-task 关联文件
    for arg in argv.iter().skip(1) {
        if arg.starts_with("maobu://") {
            let manager = app.state::<SharedManager>().inner().clone();
            dispatch_deep_link(app, &manager, arg);
        } else if arg.ends_with(".maobu-task") || arg.ends_with(".maobu") {
            let manager = app.state::<SharedManager>().inner().clone();
            let app_clone = app.clone();
            let file_path = arg.clone();
            tauri::async_runtime::spawn(async move {
                let default_dir = manager.settings().await.download_dir;
                match manager.import_tasks(&file_path, &default_dir).await {
                    Ok(tasks) if !tasks.is_empty() => {
                        let _ = app_clone.emit("tasks-imported", tasks.len() as u32);
                    }
                    Ok(_) => {}
                    Err(err) => {
                        eprintln!("单实例文件导入失败：{err}");
                    }
                }
            });
        }
    }

    let command = match cli::parse_args(argv) {
        Ok(cmd) => cmd,
        Err(error) => {
            eprintln!("单实例转发参数解析失败：{error}");
            return;
        }
    };

    match command {
        CliCommand::Run => {
            // 无子命令：用户再次启动了 GUI。聚焦主窗口。
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }
        other => {
            // CLI 子命令：在运行中的 manager 上执行。
            // 取出 owned Arc<DownloadManager>，避免 `app` 引用逃逸到 spawn 闭包。
            let manager: SharedManager = app.state::<SharedManager>().inner().clone();
            let app_clone = app.clone();
            tauri::async_runtime::spawn(async move {
                let result = run_forwarded_command(&manager, other).await;
                if let Err(error) = result {
                    let _ = app_clone.emit("cli-forward-error", error);
                }
            });
        }
    }
}

/// 在运行中的 manager 上执行转发的 CLI 命令。
async fn run_forwarded_command(manager: &SharedManager, command: CliCommand) -> Result<(), String> {
    match command {
        CliCommand::Add {
            url,
            out,
            connections,
        } => {
            let request = NewTaskRequest {
                url,
                file_name: out.as_deref().and_then(|p| {
                    let path = std::path::Path::new(p);
                    if path.is_dir() || p.ends_with('/') || p.ends_with('\\') {
                        None
                    } else {
                        path.file_name()
                            .and_then(|n| n.to_str())
                            .map(|s| s.to_string())
                    }
                }),
                destination: out.as_deref().and_then(|p| {
                    let path = std::path::Path::new(p);
                    if path.is_dir() || p.ends_with('/') || p.ends_with('\\') {
                        Some(normalize_directory(p))
                    } else {
                        path.parent()
                            .and_then(|parent| parent.to_str())
                            .filter(|s| !s.is_empty())
                            .map(normalize_directory)
                    }
                }),
                headers: std::collections::HashMap::new(),
                scheduled_at: None,
                priority: 0,
                expected_checksum: None,
                source: Some("cli".into()),
                per_task_speed_limit: 0,
                collision_policy: CollisionPolicy::Rename,
                completion_action: CompletionAction::None,
                media: None,
                connection_count: connections,
                start_paused: false,
                user_edited_file_name: false,
            };
            let task = manager.add(request).await?;
            println!("Task created: {}", task.id);
            Ok(())
        }
        CliCommand::List { status } => {
            let tasks = manager.list().await?;
            let filtered: Vec<&DownloadTask> = match status.as_deref() {
                Some(filter) if !filter.is_empty() => tasks
                    .iter()
                    .filter(|t| t.status.as_str() == filter)
                    .collect(),
                _ => tasks.iter().collect(),
            };
            if filtered.is_empty() {
                println!("没有匹配的任务");
                return Ok(());
            }
            println!(
                "{:<36} {:<14} {:<30} {:>8} {:>12}",
                "ID", "状态", "文件名", "进度", "速度"
            );
            println!("{}", "-".repeat(104));
            for task in filtered {
                let progress = if task.total_bytes > 0 {
                    format!(
                        "{:.1}%",
                        (task.downloaded_bytes as f64 / task.total_bytes as f64) * 100.0
                    )
                } else {
                    "?".to_string()
                };
                let speed = cli_format_speed(task.speed);
                let truncated_name: String = task.file_name.chars().take(28).collect();
                println!(
                    "{:<36} {:<14} {:<30} {:>8} {:>12}",
                    task.id,
                    task.status.as_str(),
                    truncated_name,
                    progress,
                    speed
                );
            }
            Ok(())
        }
        CliCommand::Pause { id } => {
            manager.action(&id, "pause").await?;
            println!("OK");
            Ok(())
        }
        CliCommand::Resume { id } => {
            manager.action(&id, "resume").await?;
            println!("OK");
            Ok(())
        }
        CliCommand::Remove { id, delete_file } => {
            manager.remove(&id, delete_file).await?;
            println!("OK");
            Ok(())
        }
        CliCommand::Run => Ok(()),
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            handle_single_instance_forward(app, argv);
        }))
        .setup(|app| {
            // Task 34：解析数据目录，优先级：环境变量 > 便携标记 > app_data_dir。
            // 便携模式下数据写入 EXE 同目录的 data/ 文件夹，与系统安装版隔离。
            let data_dir = portable::resolve_data_dir(app.handle());
            // 便携模式启用时提前创建 data/ 目录及子目录，避免后续写入失败。
            // 仅在便携模式下创建，不污染 app_data_dir 路径。
            if portable::is_portable_mode_effective() {
                if let Err(error) = std::fs::create_dir_all(&data_dir) {
                    tracing::warn!("无法创建便携数据目录：{error}");
                }
                // 预创建常用子目录，与 AGENTS.md §7"使用规范化后的明确路径"一致。
                for sub in ["logs", "cookies_tmp"] {
                    let subdir = data_dir.join(sub);
                    if let Err(error) = std::fs::create_dir_all(&subdir) {
                        tracing::warn!("无法创建便携子目录 {sub}：{error}");
                    }
                }
                tracing::info!(
                    portable = true,
                    data_dir = %data_dir.display(),
                    "便携模式已启用，数据将写入 EXE 同目录的 data/"
                );
            }
            // 初始化日志系统（Task 23.1）：按天滚动 + 脱敏 writer
            // debug 模式（dev build）启用 DEBUG 级别；release 启用 INFO
            logging::init(data_dir.join("logs"), cfg!(debug_assertions));
            let store = Arc::new(Store::open(data_dir)?);
            let manager =
                tauri::async_runtime::block_on(DownloadManager::new(store, app.handle().clone()))?;
            let pairing = PairingService::new(manager.clone());
            let media_tools = MediaTools::new(
                app.handle(),
                &tauri::async_runtime::block_on(manager.settings()),
            );
            app.manage(manager.clone());
            app.manage(pairing.clone());
            app.manage(media_tools);
            let bridge_app = app.handle().clone();
            tauri::async_runtime::spawn(bridge::run(manager.clone(), pairing, bridge_app));

            // Task 29：注册 maobu:// 深链处理器与 .maobu-task 文件双击导入。
            // - deep-link://new-url 事件在程序已运行、用户点击 maobu:// 链接时触发。
            // - get_current 处理程序通过 maobu:// 链接冷启动的情况（Windows 安装器注册 scheme）。
            // - argv 中以 .maobu-task 结尾的路径视为文件关联双击，调用 import_tasks。
            register_deep_link_handler(app.handle(), manager.clone());
            let file_app = app.handle().clone();
            let file_manager = manager.clone();
            tauri::async_runtime::spawn(async move {
                handle_maobu_task_file(&file_app, &file_manager).await;
            });

            // Task 28：注册托盘进度更新事件监听。
            // 监听 `task-updated` / `task-created` / `task-removed` 事件，节流触发
            // `update_tray_progress`（图标百分比 + tooltip "X 个任务进行中，预计 Y 完成"）。
            register_tray_progress_listener(app.handle(), manager.clone());

            // Task 32：启动计量网络定时检查（60 秒一次）。
            // 检测到计量网络且开关开启时，自动暂停 Downloading 任务并通过
            // `metered-network-detected` 事件通知前端展示 toast。
            spawn_metered_network_check(app.handle().clone(), manager.clone());

            if let Some(icon) = app.default_window_icon() {
                let show_item = MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
                let initial_settings = tauri::async_runtime::block_on(manager.settings());
                let clip_item = CheckMenuItem::with_id(
                    app,
                    "clipboard",
                    "监视剪贴板",
                    true,
                    initial_settings.clipboard_monitor,
                    None::<&str>,
                )?;
                let low_memory_item = CheckMenuItem::with_id(
                    app,
                    "low-memory",
                    "低内存模式",
                    true,
                    initial_settings.low_memory_mode,
                    None::<&str>,
                )?;
                let frosted_glass_item = CheckMenuItem::with_id(
                    app,
                    "frosted-glass",
                    "磨砂玻璃",
                    true,
                    initial_settings.frosted_glass,
                    None::<&str>,
                )?;
                app.manage(TrayMenuItems {
                    clipboard: clip_item.clone(),
                    low_memory: low_memory_item.clone(),
                    frosted_glass: frosted_glass_item.clone(),
                });

                let quit_item = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
                let menu = Menu::with_items(
                    app,
                    &[
                        &show_item,
                        &clip_item,
                        &low_memory_item,
                        &frosted_glass_item,
                        &quit_item,
                    ],
                )?;

                let _tray = TrayIconBuilder::with_id("main-tray")
                    .icon(icon.clone())
                    .tooltip("猫步下载器 · 无活动任务")
                    .menu(&menu)
                    .on_menu_event(|app, event| {
                        if event.id.as_ref() == "show" {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.unminimize();
                                let _ = window.set_focus();
                            }
                        } else if event.id.as_ref() == "clipboard" {
                            let tray_items = app.state::<TrayMenuItems>();
                            if let Ok(checked) = tray_items.clipboard.is_checked() {
                                let manager = app.state::<SharedManager>();
                                let mut settings =
                                    tauri::async_runtime::block_on(manager.settings());
                                settings.clipboard_monitor = checked;
                                let _ = tauri::async_runtime::block_on(
                                    manager.save_settings(settings.clone()),
                                );
                                let _ = app.emit("settings-changed", settings);
                            }
                        } else if event.id.as_ref() == "low-memory" {
                            let tray_items = app.state::<TrayMenuItems>();
                            if let Ok(checked) = tray_items.low_memory.is_checked() {
                                let manager = app.state::<SharedManager>();
                                let mut settings =
                                    tauri::async_runtime::block_on(manager.settings());
                                settings.low_memory_mode = checked;
                                let saved = tauri::async_runtime::block_on(
                                    manager.save_settings(settings.clone()),
                                );
                                if saved.is_ok() {
                                    let _ = app.emit("settings-changed", settings);
                                } else {
                                    let _ = tray_items.low_memory.set_checked(!checked);
                                }
                            }
                        } else if event.id.as_ref() == "frosted-glass" {
                            let tray_items = app.state::<TrayMenuItems>();
                            if let Ok(checked) = tray_items.frosted_glass.is_checked() {
                                let manager = app.state::<SharedManager>();
                                let mut settings =
                                    tauri::async_runtime::block_on(manager.settings());
                                settings.frosted_glass = checked;
                                let saved = tauri::async_runtime::block_on(
                                    manager.save_settings(settings.clone()),
                                );
                                if saved.is_ok() {
                                    let _ = app.emit("settings-changed", settings);
                                } else {
                                    let _ = tray_items.frosted_glass.set_checked(!checked);
                                }
                            }
                        } else if event.id.as_ref() == "quit" {
                            app.exit(0);
                        }
                    })
                    .on_tray_icon_event(|tray, event| {
                        if let TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } = event
                        {
                            let app = tray.app_handle();
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                    })
                    .build(app)?;
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            tasks_list,
            task_add,
            tasks_add_batch,
            tasks_export,
            tasks_import,
            backup_export,
            backup_preview,
            backup_restore,
            task_action,
            task_update_options,
            task_update_retry_policy,
            task_update_proxy,
            proxy_test,
            tasks_bulk_action,
            task_remove,
            task_rename,
            queue_reorder,
            settings_get,
            settings_save,
            power_action_get,
            power_action_arm,
            power_action_cancel,
            task_verify,
            log_to_backend,
            task_precheck,
            duplicate_check,
            task_diagnose,
            task_wait_reason,
            task_open_file,
            task_open_folder,
            history_clear,
            pairing_info,
            pairing_rotate,
            pairing_revoke,
            media_probe,
            media_detect_platform,
            media_normalize_url,
            media_tool_status,
            media_tools_detect_system,
            media_tools_install,
            media_tool_install,
            media_tools_cancel,
            media_tools_remove,
            media_tool_remove,
            media_tools_check_update,
            app_check_update,
            extension_check_compatibility,
            category_rule_add,
            category_rule_update,
            category_rule_delete,
            category_rule_list,
            category_rule_test,
            category_rule_apply,
            task_template_add,
            task_template_update,
            task_template_delete,
            task_template_list,
            task_template_test,
            media_credential_save,
            media_credential_get,
            media_credential_delete,
            media_credential_list,
            platform_compatibility_list,
            platform_compatibility_get,
            filename_cleanup_rule_add,
            filename_cleanup_rule_update,
            filename_cleanup_rule_delete,
            filename_cleanup_rule_list,
            filename_cleanup_preview,
            platform_naming_template_add,
            platform_naming_template_update,
            platform_naming_template_delete,
            platform_naming_template_list,
            preset_list,
            preset_add,
            preset_update,
            preset_delete,
            preset_apply_to_task,
            url_history_add,
            url_history_list,
            url_history_clear,
            tag_add,
            tag_update,
            tag_delete,
            tag_list,
            task_tags_set,
            task_tags_get,
            task_tags_list_all,
            open_logs_dir,
            export_recent_logs,
            app_exit,
            network_check_metered,
            app_get_info
        ])
        .run(tauri::generate_context!())
        .expect("error while running Maobu Fetch");
}
