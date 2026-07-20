use crate::models::{
    AppSettings, BackupBundle, BackupCipherInfo, BackupKdfInfo, BackupManifest, CategoryRule,
    CompletionAction, DownloadPreset, DownloadTask, FilenameCleanupRule, NewTaskRequest,
    RestorePreview, SettingsDiff, TaskExportFile, TaskExportItem, UrlHistoryEntry,
    BACKUP_BUNDLE_VERSION, BACKUP_KDF_ITERATIONS, BACKUP_KEY_SIZE, BACKUP_NONCE_SIZE,
    BACKUP_SALT_SIZE, MAX_PRIORITY, MIN_PRIORITY,
};
use aes_gcm::KeyInit;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tokio::fs;
use url::Url;
use uuid::Uuid;

const TASK_EXPORT_SCHEMA_VERSION: u32 = 1;
const MAX_IMPORT_BYTES: u64 = 10 * 1024 * 1024;
const MAX_IMPORT_TASKS: usize = 5_000;
/// 备份文件大小上限：20 MB。任务列表 + 设置 + 规则通常远小于此值；
/// 加密后 ciphertext 略大于明文，仍在此上限内。
const MAX_BACKUP_BYTES: u64 = 20 * 1024 * 1024;

pub async fn export_file(
    path: &str,
    tasks: &[DownloadTask],
    exported_at: u64,
) -> Result<usize, String> {
    let path = json_path(path, "导出")?;
    let payload = TaskExportFile {
        schema_version: TASK_EXPORT_SCHEMA_VERSION,
        exported_at,
        tasks: tasks.iter().map(export_item).collect(),
    };
    let bytes = serde_json::to_vec_pretty(&payload)
        .map_err(|error| format!("生成导出文件失败：{error}"))?;
    atomic_write(&path, &bytes).await?;
    Ok(payload.tasks.len())
}

pub async fn import_requests(path: &str, destination: &str) -> Result<Vec<NewTaskRequest>, String> {
    let path = json_path(path, "导入")?;
    let destination = PathBuf::from(destination);
    if !destination.is_absolute() {
        return Err("请选择有效的绝对下载目录".into());
    }
    fs::create_dir_all(&destination)
        .await
        .map_err(|error| format!("无法创建下载目录：{error}"))?;
    let metadata = fs::metadata(&path)
        .await
        .map_err(|error| format!("无法读取导入文件：{error}"))?;
    if metadata.len() > MAX_IMPORT_BYTES {
        return Err("导入文件不能超过 10 MB".into());
    }
    let bytes = fs::read(&path)
        .await
        .map_err(|error| format!("无法读取导入文件：{error}"))?;
    let payload: TaskExportFile =
        serde_json::from_slice(&bytes).map_err(|error| format!("任务文件格式无效：{error}"))?;
    if payload.schema_version != TASK_EXPORT_SCHEMA_VERSION {
        return Err(format!(
            "不支持任务文件版本 {}，当前支持版本 {}",
            payload.schema_version, TASK_EXPORT_SCHEMA_VERSION
        ));
    }
    if payload.tasks.is_empty() || payload.tasks.len() > MAX_IMPORT_TASKS {
        return Err(format!("导入任务数量必须为 1–{MAX_IMPORT_TASKS}"));
    }
    payload
        .tasks
        .into_iter()
        .enumerate()
        .map(|(index, item)| {
            import_request(item, &destination)
                .map_err(|error| format!("第 {} 个任务无效：{error}", index + 1))
        })
        .collect()
}

fn export_item(task: &DownloadTask) -> TaskExportItem {
    TaskExportItem {
        url: sanitized_export_url(&task.url),
        file_name: task.file_name.clone(),
        priority: task.priority,
        scheduled_at: task.scheduled_at,
        expected_checksum: task.expected_checksum.clone(),
        per_task_speed_limit: task.per_task_speed_limit,
        collision_policy: task.collision_policy.clone(),
        completion_action: if task.completion_action == CompletionAction::RunFile {
            CompletionAction::None
        } else {
            task.completion_action.clone()
        },
        media: task.media.clone(),
        connection_count: task.connection_count,
    }
}

fn import_request(item: TaskExportItem, destination: &Path) -> Result<NewTaskRequest, String> {
    let parsed = Url::parse(&item.url).map_err(|_| "URL 无效")?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("仅支持 HTTP/HTTPS URL".into());
    }
    Ok(NewTaskRequest {
        url: parsed.to_string(),
        file_name: Some(item.file_name),
        destination: Some(destination.to_string_lossy().into_owned()),
        headers: Default::default(),
        scheduled_at: item.scheduled_at,
        priority: item.priority.clamp(MIN_PRIORITY, MAX_PRIORITY),
        expected_checksum: item.expected_checksum,
        source: Some("import".into()),
        per_task_speed_limit: item.per_task_speed_limit,
        collision_policy: item.collision_policy,
        completion_action: if item.completion_action == CompletionAction::RunFile {
            CompletionAction::None
        } else {
            item.completion_action
        },
        media: item.media,
        connection_count: Some(item.connection_count.clamp(1, 32)),
        start_paused: true,
        user_edited_file_name: true,
    })
}

fn json_path(value: &str, label: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(format!("{label}路径必须是绝对路径"));
    }
    let is_json = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"));
    if !is_json {
        return Err(format!("{label}文件必须使用 .json 扩展名"));
    }
    Ok(path)
}

fn sanitized_export_url(value: &str) -> String {
    let Ok(mut url) = Url::parse(value) else {
        return value.to_string();
    };
    let _ = url.set_username("");
    let _ = url.set_password(None);
    let safe_pairs: Vec<_> = url
        .query_pairs()
        .filter(|(name, _)| !is_sensitive_query_name(name))
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect();
    url.set_query(None);
    if !safe_pairs.is_empty() {
        url.query_pairs_mut().extend_pairs(safe_pairs);
    }
    url.to_string()
}

fn is_sensitive_query_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase().replace(['-', '_'], "");
    [
        "token",
        "accesstoken",
        "authorization",
        "auth",
        "signature",
        "sig",
        "apikey",
        "credential",
        "xamzsignature",
        "xamzcredential",
        "policy",
    ]
    .iter()
    .any(|sensitive| name == *sensitive || name.ends_with(sensitive))
}

async fn atomic_write(target: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = target.parent().ok_or("导出路径缺少父目录")?;
    if !parent.is_dir() {
        return Err("导出文件夹不存在".into());
    }
    let name = target
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or("导出文件名无效")?;
    let temporary = parent.join(format!(".{name}.{}.tmp", Uuid::new_v4()));
    let backup = parent.join(format!(".{name}.{}.bak", Uuid::new_v4()));
    let mut file = fs::File::create(&temporary)
        .await
        .map_err(|error| format!("无法创建导出临时文件：{error}"))?;
    use tokio::io::AsyncWriteExt;
    file.write_all(bytes)
        .await
        .map_err(|error| format!("写入导出文件失败：{error}"))?;
    file.sync_all()
        .await
        .map_err(|error| format!("同步导出文件失败：{error}"))?;
    drop(file);
    let had_target = target.exists();
    if had_target {
        fs::rename(target, &backup)
            .await
            .map_err(|error| format!("无法替换原导出文件：{error}"))?;
    }
    if let Err(error) = fs::rename(&temporary, target).await {
        if had_target {
            let _ = fs::rename(&backup, target).await;
        }
        let _ = fs::remove_file(&temporary).await;
        return Err(format!("保存导出文件失败：{error}"));
    }
    if had_target {
        let _ = fs::remove_file(&backup).await;
    }
    Ok(())
}

// ===== Task 27: 完整备份与恢复 =====

/// 当前数据库状态快照，用于与备份 bundle 比对生成 [`RestorePreview`]。
///
/// 由 manager 在调用 `compute_preview` 前组装；字段按引用传入以避免不必要克隆。
pub struct CurrentState<'a> {
    pub settings: &'a AppSettings,
    pub category_rules: &'a [CategoryRule],
    pub filename_cleanup_rules: &'a [FilenameCleanupRule],
    pub download_presets: &'a [DownloadPreset],
    pub url_history: &'a [UrlHistoryEntry],
    pub task_ids: &'a HashSet<String>,
}

/// 备份文件顶层 JSON 结构。
///
/// `encrypted = false` 时使用 `bundle` 字段（明文）；
/// `encrypted = true` 时使用 `ciphertext` 字段（base64 编码的 AES-256-GCM 密文），
/// 加密参数（KDF / cipher）放在 `manifest` 中，便于恢复前不解密就能展示元信息。
#[derive(serde::Serialize, serde::Deserialize)]
struct BackupFile {
    manifest: BackupManifest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bundle: Option<BackupBundle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ciphertext: Option<String>,
}

/// 构造一份 [`BackupBundle`]。
///
/// 调用方传入当前数据库的全部数据；本函数负责：
/// 1. 当 `include_auth = false` 时对设置和任务做脱敏（清空 Cookie/Authorization/代理密码）；
/// 2. 填充版本号、创建时间、应用版本号等元数据。
///
/// `app_version` 来自 Cargo 包版本（如 "0.5.7"），用于恢复时展示兼容性提示。
pub fn build_bundle(
    mut settings: AppSettings,
    mut tasks: Vec<DownloadTask>,
    category_rules: Vec<CategoryRule>,
    filename_cleanup_rules: Vec<FilenameCleanupRule>,
    download_presets: Vec<DownloadPreset>,
    url_history: Vec<UrlHistoryEntry>,
    app_version: &str,
    include_auth: bool,
) -> BackupBundle {
    if !include_auth {
        settings.proxy_password = String::new();
        for task in &mut tasks {
            sanitize_task_headers(&mut task.headers);
        }
    }
    BackupBundle {
        version: BACKUP_BUNDLE_VERSION,
        created_at: now_iso8601(),
        app_version: app_version.to_string(),
        settings: Some(settings),
        category_rules,
        filename_cleanup_rules,
        download_presets,
        url_history,
        tasks,
        includes_auth: include_auth,
    }
}

/// 清空任务 headers 中的敏感字段（Cookie/Authorization/Proxy-Password）。
///
/// 大小写不敏感匹配；清空值但保留键，便于用户识别曾经存在的认证字段。
fn sanitize_task_headers(headers: &mut std::collections::HashMap<String, String>) {
    let keys_to_clear: Vec<String> = headers
        .keys()
        .filter(|key| {
            let lower = key.to_ascii_lowercase();
            lower == "cookie"
                || lower == "authorization"
                || lower == "proxy-password"
                || lower == "proxy-authorization"
                || lower == "set-cookie"
        })
        .cloned()
        .collect();
    for key in keys_to_clear {
        if let Some(value) = headers.get_mut(&key) {
            value.clear();
        }
    }
}

/// 把 bundle 写入指定路径。
///
/// - `password = None` 且 `bundle.includes_auth = false`：明文 JSON；
/// - `password = Some(_)` 且 `bundle.includes_auth = true`：AES-256-GCM 加密；
/// - 其它组合视为配置错误（包含认证信息但未提供密码 → 拒绝写入；
///   不包含认证信息但提供密码 → 同样拒绝，避免误用）。
///
/// 路径必须是绝对路径且以 `.json` 结尾。文件写入使用原子替换（同 `export_file`）。
pub async fn export_bundle(
    path: &str,
    bundle: &BackupBundle,
    password: Option<&str>,
) -> Result<(), String> {
    let path = json_path(path, "备份")?;
    if bundle.includes_auth && password.is_none() {
        return Err("包含认证信息的备份必须设置加密密码".into());
    }
    if !bundle.includes_auth && password.is_some() {
        return Err("未包含认证信息的备份不应设置密码".into());
    }
    let file = if let Some(pw) = password {
        let (ciphertext, kdf, cipher) = encrypt_bundle(bundle, pw)?;
        let manifest = BackupManifest {
            version: BACKUP_BUNDLE_VERSION,
            created_at: bundle.created_at.clone(),
            app_version: bundle.app_version.clone(),
            encrypted: true,
            includes_auth: bundle.includes_auth,
            kdf: Some(kdf),
            cipher: Some(cipher),
        };
        BackupFile {
            manifest,
            bundle: None,
            ciphertext: Some(ciphertext),
        }
    } else {
        let manifest = BackupManifest {
            version: BACKUP_BUNDLE_VERSION,
            created_at: bundle.created_at.clone(),
            app_version: bundle.app_version.clone(),
            encrypted: false,
            includes_auth: bundle.includes_auth,
            kdf: None,
            cipher: None,
        };
        BackupFile {
            manifest,
            bundle: Some(bundle.clone()),
            ciphertext: None,
        }
    };
    let bytes = serde_json::to_vec_pretty(&file).map_err(|e| format!("生成备份文件失败：{e}"))?;
    if bytes.len() as u64 > MAX_BACKUP_BYTES {
        return Err(format!(
            "备份文件超过 {} MB 上限，请减少任务数量或拆分备份",
            MAX_BACKUP_BYTES / 1024 / 1024
        ));
    }
    atomic_write(&path, &bytes).await
}

/// 仅读取备份文件的 manifest（不解密、不验证密码）。
///
/// 用于前端在用户选择备份文件后立即判断是否需要弹出密码输入框。
pub async fn read_backup_manifest(path: &str) -> Result<BackupManifest, String> {
    let path = json_path(path, "备份")?;
    let bytes = read_backup_bytes(&path).await?;
    let file: BackupFile =
        serde_json::from_slice(&bytes).map_err(|e| format!("备份文件格式无效：{e}"))?;
    Ok(file.manifest)
}

/// 读取并（如需）解密备份文件，返回完整的 [`BackupBundle`]。
///
/// 加密文件必须提供密码；密码错误或 nonce/盐被篡改时返回中文错误。
/// 明文文件忽略 `password` 参数（即使前端误传也不会影响读取）。
pub async fn read_bundle(path: &str, password: Option<&str>) -> Result<BackupBundle, String> {
    let path = json_path(path, "备份")?;
    let bytes = read_backup_bytes(&path).await?;
    let file: BackupFile =
        serde_json::from_slice(&bytes).map_err(|e| format!("备份文件格式无效：{e}"))?;
    if file.manifest.version != BACKUP_BUNDLE_VERSION {
        return Err(format!(
            "不支持的备份版本 {}，当前支持版本 {}",
            file.manifest.version, BACKUP_BUNDLE_VERSION
        ));
    }
    if !file.manifest.encrypted {
        return file
            .bundle
            .ok_or_else(|| "备份文件缺少 bundle 字段".to_string());
    }
    let ciphertext = file
        .ciphertext
        .ok_or_else(|| "加密备份文件缺少 ciphertext 字段".to_string())?;
    let kdf = file
        .manifest
        .kdf
        .ok_or_else(|| "加密备份文件缺少 KDF 参数".to_string())?;
    let cipher = file
        .manifest
        .cipher
        .ok_or_else(|| "加密备份文件缺少加密参数".to_string())?;
    let password = password.ok_or_else(|| "备份文件已加密，请输入密码".to_string())?;
    decrypt_bundle(&ciphertext, &kdf, &cipher, password)
}

/// 读取备份文件字节，校验大小上限。
async fn read_backup_bytes(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = fs::metadata(path)
        .await
        .map_err(|e| format!("无法读取备份文件：{e}"))?;
    if metadata.len() > MAX_BACKUP_BYTES {
        return Err(format!(
            "备份文件超过 {} MB 上限",
            MAX_BACKUP_BYTES / 1024 / 1024
        ));
    }
    fs::read(path)
        .await
        .map_err(|e| format!("无法读取备份文件：{e}"))
}

/// 计算恢复预览：把 bundle 与当前数据库状态比对，统计新增/覆盖/跳过条数。
///
/// 纯函数，不读写文件、不修改数据库。`current` 引用由 manager 在调用前组装。
pub fn compute_preview(bundle: &BackupBundle, current: &CurrentState<'_>) -> RestorePreview {
    let settings_diff = compute_settings_diff(bundle.settings.as_ref(), current.settings);

    let mut new_category_rules = 0u32;
    let mut override_category_rules = 0u32;
    let existing_rule_ids: HashSet<&str> = current
        .category_rules
        .iter()
        .map(|r| r.id.as_str())
        .collect();
    for rule in &bundle.category_rules {
        if existing_rule_ids.contains(rule.id.as_str()) {
            override_category_rules += 1;
        } else {
            new_category_rules += 1;
        }
    }

    let mut new_filename_cleanup_rules = 0u32;
    let mut override_filename_cleanup_rules = 0u32;
    let existing_cleanup_ids: HashSet<&str> = current
        .filename_cleanup_rules
        .iter()
        .map(|r| r.id.as_str())
        .collect();
    for rule in &bundle.filename_cleanup_rules {
        if existing_cleanup_ids.contains(rule.id.as_str()) {
            override_filename_cleanup_rules += 1;
        } else {
            new_filename_cleanup_rules += 1;
        }
    }

    let mut new_presets = 0u32;
    let mut override_presets = 0u32;
    let existing_preset_ids: HashSet<&str> = current
        .download_presets
        .iter()
        .map(|p| p.id.as_str())
        .collect();
    for preset in &bundle.download_presets {
        if existing_preset_ids.contains(preset.id.as_str()) {
            override_presets += 1;
        } else {
            new_presets += 1;
        }
    }

    let existing_urls: HashSet<&str> = current.url_history.iter().map(|h| h.url.as_str()).collect();
    let new_url_history = bundle
        .url_history
        .iter()
        .filter(|h| !existing_urls.contains(h.url.as_str()))
        .count() as u32;

    let mut new_tasks = 0u32;
    let mut duplicate_tasks = 0u32;
    for task in &bundle.tasks {
        if current.task_ids.contains(&task.id) {
            duplicate_tasks += 1;
        } else {
            new_tasks += 1;
        }
    }

    RestorePreview {
        settings_diff,
        new_category_rules,
        override_category_rules,
        new_filename_cleanup_rules,
        override_filename_cleanup_rules,
        new_presets,
        override_presets,
        new_url_history,
        new_tasks,
        duplicate_tasks,
        includes_auth: bundle.includes_auth,
        encrypted: false, // 由调用方在读取时填充
        created_at: bundle.created_at.clone(),
        app_version: bundle.app_version.clone(),
    }
}

/// 比较 backup 与当前设置，列出变更字段名。
///
/// 比较方式：把两边设置序列化为 JSON 对象，逐键比较。新增/缺失/值不同的字段
/// 都计入 `changed_fields`。`identical = true` 表示两边完全相同。
fn compute_settings_diff(backup: Option<&AppSettings>, current: &AppSettings) -> SettingsDiff {
    let Some(backup_settings) = backup else {
        return SettingsDiff {
            changed_fields: vec!["settings".into()],
            identical: false,
        };
    };
    let backup_value = serde_json::to_value(backup_settings).unwrap_or(serde_json::Value::Null);
    let current_value = serde_json::to_value(current).unwrap_or(serde_json::Value::Null);
    let backup_obj = backup_value.as_object();
    let current_obj = current_value.as_object();
    let mut changed: Vec<String> = Vec::new();
    match (backup_obj, current_obj) {
        (Some(backup_map), Some(current_map)) => {
            let mut all_keys: Vec<&String> = backup_map.keys().collect();
            for key in current_map.keys() {
                if !backup_map.contains_key(key) {
                    all_keys.push(key);
                }
            }
            for key in all_keys {
                let bv = backup_map.get(key);
                let cv = current_map.get(key);
                if bv != cv {
                    changed.push(key.clone());
                }
            }
        }
        _ => changed.push("settings".into()),
    }
    let identical = changed.is_empty();
    SettingsDiff {
        changed_fields: changed,
        identical,
    }
}

/// 加密 bundle：返回 (base64 密文, KDF 信息, 加密信息)。
///
/// 流程：
/// 1. 生成 16 字节随机盐 + 12 字节随机 nonce；
/// 2. PBKDF2-HMAC-SHA256 派生 32 字节密钥（100k 迭代）；
/// 3. AES-256-GCM 加密 bundle JSON；
/// 4. 密文 base64 编码，盐/nonce 也 base64 编码便于 JSON 存储。
fn encrypt_bundle(
    bundle: &BackupBundle,
    password: &str,
) -> Result<(String, BackupKdfInfo, BackupCipherInfo), String> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let salt: [u8; BACKUP_SALT_SIZE] = rand::random();
    let nonce_bytes: [u8; BACKUP_NONCE_SIZE] = rand::random();
    let mut key = [0u8; BACKUP_KEY_SIZE];
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(
        password.as_bytes(),
        &salt,
        BACKUP_KDF_ITERATIONS,
        &mut key,
    );
    let plaintext = serde_json::to_vec(bundle).map_err(|e| format!("序列化备份失败：{e}"))?;
    let cipher =
        aes_gcm::Aes256Gcm::new_from_slice(&key).map_err(|_| "密钥派生失败".to_string())?;
    use aes_gcm::aead::Aead;
    let nonce = aes_gcm::Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_ref())
        .map_err(|_| "加密备份失败".to_string())?;
    Ok((
        STANDARD.encode(&ciphertext),
        BackupKdfInfo {
            algorithm: "pbkdf2-sha256".into(),
            iterations: BACKUP_KDF_ITERATIONS,
            salt: STANDARD.encode(salt),
            key_size: BACKUP_KEY_SIZE as u32,
        },
        BackupCipherInfo {
            algorithm: "aes-256-gcm".into(),
            nonce: STANDARD.encode(nonce_bytes),
        },
    ))
}

/// 解密 bundle：根据 KDF/加密参数和密码还原原始 JSON。
fn decrypt_bundle(
    ciphertext_b64: &str,
    kdf: &BackupKdfInfo,
    cipher: &BackupCipherInfo,
    password: &str,
) -> Result<BackupBundle, String> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    if kdf.algorithm != "pbkdf2-sha256" {
        return Err(format!("不支持的 KDF 算法：{}", kdf.algorithm));
    }
    if cipher.algorithm != "aes-256-gcm" {
        return Err(format!("不支持的加密算法：{}", cipher.algorithm));
    }
    if kdf.key_size as usize != BACKUP_KEY_SIZE {
        return Err(format!("密钥长度异常：{}", kdf.key_size));
    }
    let salt = STANDARD
        .decode(&kdf.salt)
        .map_err(|_| "备份文件盐解码失败".to_string())?;
    if salt.len() < 16 || salt.len() > 64 {
        return Err(format!("备份文件盐长度异常：{}", salt.len()));
    }
    if kdf.iterations < 10_000 || kdf.iterations > 500_000 {
        return Err(format!(
            "备份文件 KDF 迭代次数不在安全范围：{}",
            kdf.iterations
        ));
    }
    let nonce_bytes = STANDARD
        .decode(&cipher.nonce)
        .map_err(|_| "备份文件 nonce 解码失败".to_string())?;
    if nonce_bytes.len() != BACKUP_NONCE_SIZE {
        return Err("备份文件 nonce 长度异常".into());
    }
    let ciphertext = STANDARD
        .decode(ciphertext_b64)
        .map_err(|_| "备份文件密文解码失败".to_string())?;
    let mut key = [0u8; BACKUP_KEY_SIZE];
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(password.as_bytes(), &salt, kdf.iterations, &mut key);
    let cipher =
        aes_gcm::Aes256Gcm::new_from_slice(&key).map_err(|_| "密钥派生失败".to_string())?;
    use aes_gcm::aead::Aead;
    let nonce = aes_gcm::Nonce::from_slice(&nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, ciphertext.as_ref())
        .map_err(|_| "密码错误或备份文件已损坏".to_string())?;
    serde_json::from_slice(&plaintext).map_err(|e| format!("解析备份 JSON 失败：{e}"))
}

/// 返回 ISO 8601 UTC 时间字符串（如 "2026-07-20T12:34:56Z"）。
fn now_iso8601() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_iso8601(secs)
}

/// 把 Unix 秒数格式化为 ISO 8601 UTC 字符串。
///
/// 仅用于备份文件元数据展示，精度为秒。算法是简单的儒略日转换，
/// 不引入 chrono 等额外依赖（保持紧凑）。
fn format_iso8601(unix_secs: u64) -> String {
    let days = (unix_secs / 86_400) as i64;
    let secs_of_day = (unix_secs % 86_400) as u64;
    let (year, month, day) = days_to_ymd(days);
    let hour = secs_of_day / 3600;
    let minute = (secs_of_day % 3600) / 60;
    let second = secs_of_day % 60;
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hour, minute, second
    )
}

/// 把"自 1970-01-01 起的天数"转换为 (year, month, day)。
///
/// 使用 Howard Hinnant 的 civil_from_days 算法，无外部依赖。
fn days_to_ymd(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{CollisionPolicy, CompletionAction, DownloadSegment, TaskStatus};
    use std::collections::HashMap;

    fn task() -> DownloadTask {
        DownloadTask {
            id: "task".into(),
            url: "https://user:password@example.com/file?id=42&token=secret&X-Amz-Signature=hidden"
                .into(),
            file_name: "file.bin".into(),
            destination: "C:\\secret\\path".into(),
            total_bytes: 10,
            downloaded_bytes: 5,
            speed: 0,
            eta_seconds: None,
            status: TaskStatus::Paused,
            error: None,
            created_at: 0,
            completed_at: None,
            scheduled_at: None,
            category: "other".into(),
            queue_position: 0,
            priority: 1,
            retry_count: 0,
            max_retries: 3,
            checksum_sha256: None,
            expected_checksum: None,
            source: "desktop".into(),
            etag: None,
            last_modified: None,
            final_url: None,
            response_status: None,
            content_type: None,
            accepts_ranges: None,
            headers: HashMap::from([("Authorization".into(), "Bearer private".into())]),
            media: None,
            per_task_speed_limit: 0,
            collision_policy: CollisionPolicy::Rename,
            completion_action: CompletionAction::RunFile,
            connection_count: 8,
            active_connections: 0,
            segments: vec![DownloadSegment {
                index: 0,
                start_byte: 0,
                end_byte: 9,
                downloaded_bytes: 5,
                status: "paused".into(),
            }],
            retry_policy_override: None,
            proxy_override: None,
            proxy_auth: None,
        }
    }

    #[test]
    fn export_omits_credentials_paths_headers_and_runtime_state() {
        let item = export_item(&task());
        let json = serde_json::to_string(&item).unwrap();
        assert!(json.contains("id=42"));
        for secret in [
            "password",
            "secret",
            "hidden",
            "private",
            "C:\\\\secret",
            "segments",
            "downloaded_bytes",
        ] {
            assert!(!json.contains(secret), "export leaked {secret}");
        }
        assert_eq!(item.completion_action, CompletionAction::None);
    }

    #[test]
    fn imported_tasks_are_paused_and_have_no_headers() {
        let directory = tempfile::tempdir().unwrap();
        let request = import_request(export_item(&task()), directory.path()).unwrap();
        assert!(request.start_paused);
        assert!(request.headers.is_empty());
        assert_eq!(request.source.as_deref(), Some("import"));
    }

    #[tokio::test]
    async fn exported_json_round_trips_from_disk() {
        let directory = tempfile::tempdir().unwrap();
        let export_path = directory.path().join("tasks.json");
        let destination = directory.path().join("downloads");
        let count = export_file(export_path.to_str().unwrap(), &[task()], 123)
            .await
            .unwrap();
        let requests =
            import_requests(export_path.to_str().unwrap(), destination.to_str().unwrap())
                .await
                .unwrap();
        assert_eq!(count, 1);
        assert_eq!(requests.len(), 1);
        assert!(requests[0].start_paused);
        assert_eq!(requests[0].destination.as_deref(), destination.to_str());
    }

    // ===== Task 27: 备份与恢复测试 =====

    fn sample_bundle(include_auth: bool) -> BackupBundle {
        let mut task_with_auth = task();
        task_with_auth.headers.insert(
            "Cookie".into(),
            "session=private-session-id; token=secret".into(),
        );
        task_with_auth
            .headers
            .insert("X-Custom".into(), "keep".into());
        let mut settings = AppSettings::default();
        settings.proxy_password = "super-secret-proxy-pw".into();
        settings.download_dir = "D:\\Downloads".into();
        build_bundle(
            settings,
            vec![task_with_auth],
            vec![CategoryRule {
                id: "rule-1".into(),
                name: "GitHub".into(),
                rule_type: crate::models::CategoryRuleType::Domain,
                pattern: "github.com".into(),
                target_directory: "D:\\GitHub".into(),
                enabled: true,
                priority: 10,
            }],
            vec![FilenameCleanupRule {
                id: "cleanup-1".into(),
                name: "去除水印".into(),
                pattern: "\\[www\\.\\w+\\.com\\]".into(),
                replacement: String::new(),
                enabled: true,
                priority: 0,
            }],
            vec![DownloadPreset {
                id: "default".into(),
                name: "普通".into(),
                connections: 8,
                speed_limit: None,
                completion_action: None,
                verify_checksum: false,
                scheduled_at: None,
                is_builtin: true,
            }],
            vec![UrlHistoryEntry {
                url: "https://example.com/file.zip".into(),
                last_used: 1_700_000_000_000,
            }],
            "0.5.7",
            include_auth,
        )
    }

    fn empty_current_settings() -> AppSettings {
        let mut settings = AppSettings::default();
        // 与 sample_bundle 不同的下载目录，确保 settings_diff 非空
        settings.download_dir = "C:\\Users\\Test\\Downloads".into();
        settings
    }

    #[test]
    fn build_bundle_without_auth_strips_sensitive_headers_and_proxy_password() {
        let bundle = sample_bundle(false);
        let json = serde_json::to_string(&bundle).unwrap();
        // 备份中不应出现任何敏感值
        for secret in [
            "super-secret-proxy-pw",
            "private-session-id",
            "Bearer private",
            "session=private-session-id",
        ] {
            assert!(!json.contains(secret), "bundle leaked {secret}");
        }
        // Cookie/Authorization 键应保留但值为空
        let task_in_bundle = &bundle.tasks[0];
        assert_eq!(
            task_in_bundle.headers.get("Cookie").map(String::as_str),
            Some("")
        );
        assert_eq!(
            task_in_bundle
                .headers
                .get("Authorization")
                .map(String::as_str),
            Some("")
        );
        // 非敏感 header 应保留
        assert_eq!(
            task_in_bundle.headers.get("X-Custom").map(String::as_str),
            Some("keep")
        );
        // proxy_password 应为空
        assert_eq!(bundle.settings.as_ref().unwrap().proxy_password, "");
        assert!(!bundle.includes_auth);
    }

    #[test]
    fn build_bundle_with_auth_keeps_sensitive_data() {
        let bundle = sample_bundle(true);
        let json = serde_json::to_string(&bundle).unwrap();
        assert!(
            json.contains("super-secret-proxy-pw"),
            "auth bundle should keep proxy password"
        );
        assert!(
            json.contains("private-session-id"),
            "auth bundle should keep cookies"
        );
        assert!(bundle.includes_auth);
    }

    #[tokio::test]
    async fn export_and_read_plaintext_bundle_round_trips() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("backup.json");
        let bundle = sample_bundle(false);
        export_bundle(path.to_str().unwrap(), &bundle, None)
            .await
            .unwrap();
        let manifest = read_backup_manifest(path.to_str().unwrap()).await.unwrap();
        assert!(!manifest.encrypted);
        assert!(!manifest.includes_auth);
        let restored = read_bundle(path.to_str().unwrap(), None).await.unwrap();
        assert_eq!(restored, bundle);
    }

    #[tokio::test]
    async fn export_and_read_encrypted_bundle_round_trips() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("backup-encrypted.json");
        let bundle = sample_bundle(true);
        export_bundle(path.to_str().unwrap(), &bundle, Some("correct-password"))
            .await
            .unwrap();
        let manifest = read_backup_manifest(path.to_str().unwrap()).await.unwrap();
        assert!(manifest.encrypted);
        assert!(manifest.includes_auth);
        assert_eq!(manifest.kdf.as_ref().unwrap().algorithm, "pbkdf2-sha256");
        assert_eq!(
            manifest.kdf.as_ref().unwrap().iterations,
            BACKUP_KDF_ITERATIONS
        );
        assert_eq!(manifest.cipher.as_ref().unwrap().algorithm, "aes-256-gcm");

        let restored = read_bundle(path.to_str().unwrap(), Some("correct-password"))
            .await
            .unwrap();
        assert_eq!(restored, bundle);
        // 加密文件不应在磁盘上明文出现敏感值
        let on_disk = std::fs::read_to_string(path).unwrap();
        assert!(!on_disk.contains("super-secret-proxy-pw"));
        assert!(!on_disk.contains("private-session-id"));
    }

    #[tokio::test]
    async fn encrypted_bundle_wrong_password_returns_chinese_error() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("backup-wrong.json");
        let bundle = sample_bundle(true);
        export_bundle(path.to_str().unwrap(), &bundle, Some("correct"))
            .await
            .unwrap();
        let err = read_bundle(path.to_str().unwrap(), Some("wrong"))
            .await
            .unwrap_err();
        assert!(err.contains("密码错误") || err.contains("已损坏"));
    }

    #[tokio::test]
    async fn encrypted_bundle_missing_password_returns_error() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("backup-no-pw.json");
        let bundle = sample_bundle(true);
        export_bundle(path.to_str().unwrap(), &bundle, Some("correct"))
            .await
            .unwrap();
        let err = read_bundle(path.to_str().unwrap(), None).await.unwrap_err();
        assert!(err.contains("已加密") || err.contains("密码"));
    }

    #[tokio::test]
    async fn export_bundle_rejects_auth_without_password() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("backup-invalid.json");
        let bundle = sample_bundle(true);
        let err = export_bundle(path.to_str().unwrap(), &bundle, None)
            .await
            .unwrap_err();
        assert!(err.contains("加密密码"));
    }

    #[tokio::test]
    async fn export_bundle_rejects_password_without_auth() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("backup-mismatch.json");
        let bundle = sample_bundle(false);
        let err = export_bundle(path.to_str().unwrap(), &bundle, Some("password"))
            .await
            .unwrap_err();
        assert!(err.contains("不应设置密码"));
    }

    #[test]
    fn compute_preview_counts_new_and_duplicate_entries() {
        let bundle = sample_bundle(false);
        let current_settings = empty_current_settings();
        let existing_task = DownloadTask {
            id: "task".into(), // 与 sample_bundle 中 task ID 相同
            ..task()
        };
        let current_category_rules = vec![CategoryRule {
            id: "rule-1".into(), // 与 bundle 中 rule-1 相同
            name: "Old GitHub".into(),
            rule_type: crate::models::CategoryRuleType::Domain,
            pattern: "github.com".into(),
            target_directory: "E:\\Old".into(),
            enabled: true,
            priority: 10,
        }];
        let current_cleanup = vec![FilenameCleanupRule {
            id: "cleanup-existing".into(),
            name: "Other".into(),
            pattern: "other".into(),
            replacement: "x".into(),
            enabled: true,
            priority: 0,
        }];
        let current_presets = vec![DownloadPreset {
            id: "default".into(), // 与 bundle 中 default 相同
            name: "Built-in".into(),
            connections: 4,
            speed_limit: None,
            completion_action: None,
            verify_checksum: false,
            scheduled_at: None,
            is_builtin: true,
        }];
        let current_history = vec![UrlHistoryEntry {
            url: "https://example.com/file.zip".into(), // 与 bundle 中相同
            last_used: 1,
        }];
        let current_task_ids: HashSet<String> = std::iter::once(existing_task.id.clone()).collect();
        let current = CurrentState {
            settings: &current_settings,
            category_rules: &current_category_rules,
            filename_cleanup_rules: &current_cleanup,
            download_presets: &current_presets,
            url_history: &current_history,
            task_ids: &current_task_ids,
        };
        let preview = compute_preview(&bundle, &current);
        assert_eq!(preview.new_category_rules, 0);
        assert_eq!(preview.override_category_rules, 1);
        assert_eq!(preview.new_filename_cleanup_rules, 1);
        assert_eq!(preview.override_filename_cleanup_rules, 0);
        assert_eq!(preview.new_presets, 0);
        assert_eq!(preview.override_presets, 1);
        assert_eq!(preview.new_url_history, 0); // 已存在
        assert_eq!(preview.new_tasks, 0); // task ID 已存在
        assert_eq!(preview.duplicate_tasks, 1);
        assert!(!preview.settings_diff.identical);
        assert!(preview
            .settings_diff
            .changed_fields
            .iter()
            .any(|f| f == "download_dir" || f == "proxy_password"));
    }

    #[test]
    fn compute_preview_reports_identical_settings_when_bundle_matches_current() {
        let bundle = sample_bundle(false);
        let current_settings = bundle.settings.clone().unwrap();
        let current_rules = bundle.category_rules.clone();
        let current_cleanup = bundle.filename_cleanup_rules.clone();
        let current_presets = bundle.download_presets.clone();
        let current_history = bundle.url_history.clone();
        let mut current_task_ids = HashSet::new();
        for t in &bundle.tasks {
            current_task_ids.insert(t.id.clone());
        }
        let current = CurrentState {
            settings: &current_settings,
            category_rules: &current_rules,
            filename_cleanup_rules: &current_cleanup,
            download_presets: &current_presets,
            url_history: &current_history,
            task_ids: &current_task_ids,
        };
        let preview = compute_preview(&bundle, &current);
        assert!(preview.settings_diff.identical);
        assert!(preview.settings_diff.changed_fields.is_empty());
        assert_eq!(preview.new_tasks, 0);
        assert_eq!(preview.duplicate_tasks, bundle.tasks.len() as u32);
    }

    #[test]
    fn format_iso8601_matches_expected_utc_shape() {
        // 2026-07-20T12:34:56Z 对应的 Unix 秒数：
        // 20654 天 * 86400 + 12*3600 + 34*60 + 56 = 1_784_505_600 + 45_296 = 1_784_550_896
        let secs = 1_784_550_896u64;
        let formatted = format_iso8601(secs);
        assert_eq!(formatted, "2026-07-20T12:34:56Z");
    }

    #[test]
    fn days_to_ymd_handles_epoch_and_known_dates() {
        // 1970-01-01
        assert_eq!(days_to_ymd(0), (1970, 1, 1));
        // 2026-07-20（从 1970-01-01 起第 20654 天）
        assert_eq!(days_to_ymd(20_654), (2026, 7, 20));
    }
}
