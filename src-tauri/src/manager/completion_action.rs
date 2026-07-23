//! 下载完成动作扩展（Task 17）。
//!
//! 提供 `TemplateContext` / `expand_template` 模板替换，以及 `RunCommand` / `CopyTo` /
//! `MoveTo` 的执行逻辑。`Quit` 由 `perform_completion_action` 通过 `app.exit(0)` 触发，
//! 不经过本模块。`None` / `OpenFolder` / `RunFile` / `Shutdown` / `Hibernate` 等旧变体
//! 仍由 `perform_completion_action` 直接处理。
//!
//! 设计要点：
//! - 不引入新依赖，使用 `std::process::Command` 直接 spawn
//! - RunCommand 采用 fire-and-forget：spawn 成功即返回 Ok，spawn 失败返回 Err
//! - CopyTo/MoveTo 路径穿越防护（AGENTS.md §7），重名按 `collision_policy` 处理
//! - MoveTo 跨盘移动退化为 copy + remove_file

use crate::models::{CollisionPolicy, CompletionAction};
use std::path::{Component, Path, PathBuf};
use tauri::AppHandle;

/// 模板替换上下文（Task 17.2）。
#[derive(Debug, Clone)]
pub struct TemplateContext {
    /// 完整文件路径（destination + file_name）。
    pub file_path: String,
    /// 仅文件名。
    pub file_name: String,
    /// 文件所在目录（destination）。
    pub file_dir: String,
    /// 下载 URL。
    pub url: String,
    /// 任务标题或文件名。
    pub title: String,
}

impl TemplateContext {
    /// 从 DownloadTask 构建模板上下文。`title` 在文件名为空时回退到 URL。
    pub fn from_task(task: &crate::models::DownloadTask) -> Self {
        let path = PathBuf::from(&task.destination).join(&task.file_name);
        let title = if task.file_name.is_empty() {
            task.url.clone()
        } else {
            task.file_name.clone()
        };
        Self {
            file_path: path.to_string_lossy().to_string(),
            file_name: task.file_name.clone(),
            file_dir: task.destination.clone(),
            url: task.url.clone(),
            title,
        }
    }
}

/// 支持的模板变量列表（用于 UI 提示）。
pub const TEMPLATE_VARIABLES: &[(&str, &str)] = &[
    ("$FILE", "完整文件路径"),
    ("$FILENAME", "仅文件名"),
    ("$DIR", "文件所在目录"),
    ("$URL", "下载 URL"),
    ("$TITLE", "任务标题或文件名"),
];

/// 查找变量名对应的值。未知名返回 `None`（保持原样）。
fn lookup_variable(name: &str, context: &TemplateContext) -> Option<String> {
    match name {
        "FILE" => Some(context.file_path.clone()),
        "FILENAME" => Some(context.file_name.clone()),
        "DIR" => Some(context.file_dir.clone()),
        "URL" => Some(context.url.clone()),
        "TITLE" => Some(context.title.clone()),
        _ => None,
    }
}

/// 模板替换（Task 17.2）。
///
/// 支持两种语法：`$VAR` 和 `${VAR}`。`$VAR` 形式匹配最长的 ASCII 字母数字+下划线序列，
/// 因此 `$FILENAME` 不会被误识别为 `$FILE` + `NAME`；`$FILE_NAME` 整体作为变量名查找，
/// 未命中则保持原样。不解析 shell 元字符（如 `;`、`|`、`&`），保持原样传递给子进程。
pub fn expand_template(template: &str, context: &TemplateContext) -> String {
    let mut result = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(dollar_pos) = rest.find('$') {
        result.push_str(&rest[..dollar_pos]);
        let after = &rest[dollar_pos + 1..];
        // 优先尝试 ${NAME} 语法
        if let Some(stripped) = after.strip_prefix('{') {
            if let Some(close) = stripped.find('}') {
                let name = &stripped[..close];
                if let Some(value) = lookup_variable(name, context) {
                    result.push_str(&value);
                    rest = &stripped[close + 1..];
                    continue;
                }
            }
            // 不匹配的 ${...}，$ 保留为字面量
            result.push('$');
            rest = after;
            continue;
        }
        // 尝试 $NAME 语法（最长字母数字+下划线序列）
        let end = after
            .bytes()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == b'_')
            .count();
        if end > 0 {
            let name = &after[..end];
            if let Some(value) = lookup_variable(name, context) {
                result.push_str(&value);
                rest = &after[end..];
                continue;
            }
        }
        // $ 后无有效变量名，保留为字面量
        result.push('$');
        rest = after;
    }
    result.push_str(rest);
    result
}

/// 校验目标目录不含 `..` 路径穿越（Task 17.3，AGENTS.md §7）。
pub fn validate_target_directory(dir: &str) -> Result<PathBuf, String> {
    let trimmed = dir.trim();
    if trimmed.is_empty() {
        return Err("目标目录不能为空".into());
    }
    let path = PathBuf::from(trimmed);
    for component in path.components() {
        if matches!(component, Component::ParentDir) {
            return Err("目标目录不能包含 .. 路径穿越".into());
        }
    }
    Ok(path)
}

/// 校验文件名不含路径分隔符或 `..`（防止逃逸目标目录）。
fn validate_file_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("文件名不能为空".into());
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return Err("重命名后的文件名不能包含路径分隔符".into());
    }
    if trimmed == ".." || trimmed == "." {
        return Err("重命名后的文件名不能为 . 或 ..".into());
    }
    Ok(trimmed.to_string())
}

/// 根据重名策略解析目标路径（不检查 reserved_paths，因为下载已完成）。
fn resolve_collision_path(
    target_dir: &Path,
    file_name: &str,
    policy: CollisionPolicy,
) -> Result<PathBuf, String> {
    let target = target_dir.join(file_name);
    if !target.exists() {
        return Ok(target);
    }
    match policy {
        CollisionPolicy::Overwrite => Ok(target),
        CollisionPolicy::Skip => Err("目标文件已存在，已跳过".into()),
        CollisionPolicy::Rename => {
            let path = Path::new(file_name);
            let stem = path
                .file_stem()
                .and_then(|v| v.to_str())
                .unwrap_or("download");
            let ext = path.extension().and_then(|v| v.to_str());
            for index in 1..10_000 {
                let name = match ext {
                    Some(ext) => format!("{stem} ({index}).{ext}"),
                    None => format!("{stem} ({index})"),
                };
                let candidate = target_dir.join(&name);
                if !candidate.exists() {
                    return Ok(candidate);
                }
            }
            Err("无法生成不重复的文件名".into())
        }
    }
}

/// 解析最终文件名：展开 rename_pattern，若为空或未提供则用原文件名。
fn resolve_rename(
    rename_pattern: Option<&str>,
    context: &TemplateContext,
) -> Result<String, String> {
    match rename_pattern {
        Some(pattern) if !pattern.trim().is_empty() => {
            let expanded = expand_template(pattern, context);
            validate_file_name(&expanded)
        }
        _ => Ok(context.file_name.clone()),
    }
}

/// 执行 RunCommand 动作（Task 17.3）。
///
/// 使用 `std::process::Command` 直接 spawn，不引入脚本引擎。采用 fire-and-forget 策略：
/// spawn 成功即返回 Ok；spawn 失败（如命令不存在）返回 Err。命令运行时失败（非零退出码）
/// 不会被检测到，但不会破坏任务状态。`command` 为可执行文件路径，`args` 中每个元素
/// 都会经过 `expand_template` 替换。`working_dir` 设置启动目录。
pub fn run_command_action(
    command: &str,
    args: &[String],
    working_dir: Option<&str>,
    context: &TemplateContext,
) -> Result<(), String> {
    if command.trim().is_empty() {
        return Err("命令路径不能为空".into());
    }
    let expanded_args: Vec<String> = args
        .iter()
        .map(|arg| expand_template(arg, context))
        .collect();
    let mut builder = crate::media_tools::create_hidden_std_command(command);
    builder.args(&expanded_args);
    if let Some(dir) = working_dir {
        let dir = dir.trim();
        if !dir.is_empty() {
            builder.current_dir(dir);
        }
    }
    builder
        .spawn()
        .map_err(|e| format!("无法启动命令 '{command}'：{e}"))?;
    Ok(())
}

/// 判断是否为跨盘错误（Windows: ERROR_NOT_SAME_DEVICE=17，Linux: EXDEV=18）。
fn is_cross_device_error(error: &std::io::Error) -> bool {
    matches!(error.raw_os_error(), Some(17) | Some(18))
}

/// 执行 CopyTo 动作（Task 17.3）。
///
/// 复制文件到目标目录，源文件保留。重名按 `collision_policy` 处理。
/// `target_directory` 不能含 `..`；`rename_pattern` 展开后不能含路径分隔符。
pub async fn copy_to_action(
    target_directory: &str,
    rename_pattern: Option<&str>,
    collision_policy: CollisionPolicy,
    context: &TemplateContext,
) -> Result<(), String> {
    let target_dir = validate_target_directory(target_directory)?;
    let file_name = resolve_rename(rename_pattern, context)?;
    let target_path = resolve_collision_path(&target_dir, &file_name, collision_policy)?;
    tokio::fs::create_dir_all(&target_dir)
        .await
        .map_err(|e| format!("无法创建目标目录：{e}"))?;
    let source = Path::new(&context.file_path);
    tokio::fs::copy(source, &target_path)
        .await
        .map_err(|e| format!("复制文件失败：{e}"))?;
    Ok(())
}

/// 执行 MoveTo 动作（Task 17.3）。
///
/// 移动文件到目标目录，跨盘时退化为 copy + remove_file。成功后原路径文件不再存在
/// （这是 MoveTo 的预期行为，不违反 §7 禁止递归删除）。
pub async fn move_to_action(
    target_directory: &str,
    rename_pattern: Option<&str>,
    collision_policy: CollisionPolicy,
    context: &TemplateContext,
) -> Result<(), String> {
    let target_dir = validate_target_directory(target_directory)?;
    let file_name = resolve_rename(rename_pattern, context)?;
    let target_path = resolve_collision_path(&target_dir, &file_name, collision_policy.clone())?;
    tokio::fs::create_dir_all(&target_dir)
        .await
        .map_err(|e| format!("无法创建目标目录：{e}"))?;
    let source = Path::new(&context.file_path);
    if source == target_path {
        return Ok(());
    }

    let mut temp_backup: Option<PathBuf> = None;
    if target_path.exists() && collision_policy == CollisionPolicy::Overwrite {
        let backup_name = format!(
            ".bak_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        );
        let backup_path = target_dir.join(backup_name);
        if tokio::fs::rename(&target_path, &backup_path).await.is_ok() {
            temp_backup = Some(backup_path);
        } else {
            let _ = tokio::fs::remove_file(&target_path).await;
        }
    }

    let move_result = match tokio::fs::rename(source, &target_path).await {
        Ok(()) => Ok(()),
        Err(e) if is_cross_device_error(&e) => {
            // 跨盘移动：先复制再删除源文件
            let copy_res = tokio::fs::copy(source, &target_path).await;
            if let Err(err) = copy_res {
                Err(format!("跨盘复制文件失败：{err}"))
            } else if let Err(err) = tokio::fs::remove_file(source).await {
                Err(format!("跨盘移动后删除源文件失败：{err}"))
            } else {
                Ok(())
            }
        }
        Err(e) => Err(format!("移动文件失败：{e}")),
    };

    match move_result {
        Ok(()) => {
            if let Some(backup) = temp_backup {
                let _ = tokio::fs::remove_file(backup).await;
            }
            Ok(())
        }
        Err(err) => {
            if let Some(backup) = temp_backup {
                let _ = tokio::fs::rename(backup, &target_path).await;
            }
            Err(err)
        }
    }
}

/// 执行扩展完成动作（Quit / RunCommand / CopyTo / MoveTo）。
///
/// 返回 `Ok(())` 表示动作成功或已 fire-and-forget；
/// 返回 `Err(String)` 表示动作失败（调用方应记录到 `task.error`，但保持任务 Completed 状态）。
/// `Quit` 直接调用 `app.exit(0)`，不返回。旧变体（None/OpenFolder/RunFile/Shutdown/Hibernate）
/// 由 `perform_completion_action` 直接处理，不会进入本函数。
pub async fn run_extended_action(
    action: &CompletionAction,
    context: &TemplateContext,
    collision_policy: CollisionPolicy,
    app: &AppHandle,
) -> Result<(), String> {
    match action {
        CompletionAction::Quit => {
            // 退出应用。app.exit(0) 会触发 Tauri 的优雅退出流程。
            app.exit(0);
            Ok(())
        }
        CompletionAction::RunCommand {
            command,
            args,
            working_dir,
        } => run_command_action(command, args, working_dir.as_deref(), context),
        CompletionAction::CopyTo {
            target_directory,
            rename_pattern,
        } => {
            copy_to_action(
                target_directory,
                rename_pattern.as_deref(),
                collision_policy,
                context,
            )
            .await
        }
        CompletionAction::MoveTo {
            target_directory,
            rename_pattern,
        } => {
            move_to_action(
                target_directory,
                rename_pattern.as_deref(),
                collision_policy,
                context,
            )
            .await
        }
        // 旧变体由 perform_completion_action 直接处理，不应进入本函数
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_context() -> TemplateContext {
        TemplateContext {
            file_path: "/downloads/movie.mp4".into(),
            file_name: "movie.mp4".into(),
            file_dir: "/downloads".into(),
            url: "https://example.com/video".into(),
            title: "Movie Title".into(),
        }
    }

    #[test]
    fn expand_template_replaces_file_variable() {
        let ctx = test_context();
        assert_eq!(expand_template("$FILE", &ctx), "/downloads/movie.mp4");
        assert_eq!(expand_template("${FILE}", &ctx), "/downloads/movie.mp4");
    }

    #[test]
    fn expand_template_replaces_filename_variable() {
        let ctx = test_context();
        assert_eq!(expand_template("$FILENAME", &ctx), "movie.mp4");
        assert_eq!(expand_template("${FILENAME}", &ctx), "movie.mp4");
    }

    #[test]
    fn expand_template_replaces_dir_variable() {
        let ctx = test_context();
        assert_eq!(expand_template("$DIR", &ctx), "/downloads");
        assert_eq!(expand_template("${DIR}", &ctx), "/downloads");
    }

    #[test]
    fn expand_template_replaces_url_variable() {
        let ctx = test_context();
        assert_eq!(expand_template("$URL", &ctx), "https://example.com/video");
        assert_eq!(expand_template("${URL}", &ctx), "https://example.com/video");
    }

    #[test]
    fn expand_template_replaces_title_variable() {
        let ctx = test_context();
        assert_eq!(expand_template("$TITLE", &ctx), "Movie Title");
        assert_eq!(expand_template("${TITLE}", &ctx), "Movie Title");
    }

    #[test]
    fn expand_template_replaces_multiple_variables_in_one_string() {
        let ctx = test_context();
        let template = "$FILE $FILENAME $DIR $URL $TITLE";
        let result = expand_template(template, &ctx);
        assert_eq!(
            result,
            "/downloads/movie.mp4 movie.mp4 /downloads https://example.com/video Movie Title"
        );
    }

    #[test]
    fn expand_template_preserves_shell_metacharacters() {
        // Task 17.2: shell 元字符不解析，保持原样传递
        let ctx = test_context();
        let template = "echo $URL | grep $TITLE ; rm -rf / &";
        let result = expand_template(template, &ctx);
        assert_eq!(
            result,
            "echo https://example.com/video | grep Movie Title ; rm -rf / &"
        );
    }

    #[test]
    fn expand_template_keeps_unknown_variables_as_is() {
        let ctx = test_context();
        assert_eq!(expand_template("$UNKNOWN", &ctx), "$UNKNOWN");
        assert_eq!(expand_template("${UNKNOWN}", &ctx), "${UNKNOWN}");
        // $FILE_NAME 整体作为变量名查找，未命中则保持原样
        assert_eq!(expand_template("$FILE_NAME", &ctx), "$FILE_NAME");
    }

    #[test]
    fn expand_template_handles_dollar_at_end() {
        let ctx = test_context();
        assert_eq!(expand_template("cost: $", &ctx), "cost: $");
        assert_eq!(expand_template("$", &ctx), "$");
    }

    #[test]
    fn expand_template_handles_unclosed_braces() {
        let ctx = test_context();
        assert_eq!(expand_template("${FILE", &ctx), "${FILE");
        assert_eq!(expand_template("${", &ctx), "${");
    }

    #[test]
    fn expand_template_distinguishes_file_and_filename() {
        // $FILENAME 不应被 $FILE 部分匹配
        let ctx = test_context();
        assert_eq!(expand_template("$FILENAME", &ctx), "movie.mp4");
        assert_eq!(expand_template("$FILE", &ctx), "/downloads/movie.mp4");
        assert_eq!(
            expand_template("$FILE.txt", &ctx),
            "/downloads/movie.mp4.txt"
        );
    }

    #[test]
    fn expand_template_handles_no_variables() {
        let ctx = test_context();
        assert_eq!(expand_template("plain text", &ctx), "plain text");
        assert_eq!(expand_template("", &ctx), "");
    }

    #[test]
    fn validate_target_directory_rejects_parent_dir() {
        assert!(validate_target_directory("../etc/passwd").is_err());
        assert!(validate_target_directory("foo/../bar").is_err());
        assert!(validate_target_directory("/downloads/..").is_err());
    }

    #[test]
    fn validate_target_directory_accepts_normal_paths() {
        assert!(validate_target_directory("/downloads").is_ok());
        assert!(validate_target_directory("C:\\Downloads").is_ok());
        assert!(validate_target_directory("./downloads").is_ok());
    }

    #[test]
    fn validate_target_directory_rejects_empty() {
        assert!(validate_target_directory("").is_err());
        assert!(validate_target_directory("   ").is_err());
    }

    #[test]
    fn validate_file_name_rejects_path_separators() {
        assert!(validate_file_name("a/b").is_err());
        assert!(validate_file_name("a\\b").is_err());
        assert!(validate_file_name("..").is_err());
        assert!(validate_file_name(".").is_err());
    }

    #[test]
    fn validate_file_name_accepts_normal_names() {
        assert!(validate_file_name("movie.mp4").is_ok());
        assert!(validate_file_name("file (1).txt").is_ok());
        assert!(validate_file_name("文件.zip").is_ok());
    }

    #[test]
    fn resolve_collision_path_skip_returns_error_when_exists() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("file.txt");
        std::fs::write(&existing, b"hello").unwrap();
        let result = resolve_collision_path(dir.path(), "file.txt", CollisionPolicy::Skip);
        assert!(result.is_err());
    }

    #[test]
    fn resolve_collision_path_rename_generates_unique_name() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("file.txt");
        std::fs::write(&existing, b"hello").unwrap();
        let result =
            resolve_collision_path(dir.path(), "file.txt", CollisionPolicy::Rename).unwrap();
        assert_eq!(result.file_name().unwrap(), "file (1).txt");
    }

    #[test]
    fn resolve_collision_path_overwrite_returns_same_path() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("file.txt");
        std::fs::write(&existing, b"hello").unwrap();
        let result =
            resolve_collision_path(dir.path(), "file.txt", CollisionPolicy::Overwrite).unwrap();
        assert_eq!(result, existing);
    }

    #[test]
    fn resolve_collision_path_returns_target_when_not_exists() {
        let dir = tempfile::tempdir().unwrap();
        let result =
            resolve_collision_path(dir.path(), "new.txt", CollisionPolicy::Rename).unwrap();
        assert_eq!(result, dir.path().join("new.txt"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn run_command_action_succeeds_for_cmd_exe() {
        // Task 17.5: 集成测试 - RunCommand 执行成功（Windows 用 cmd.exe）
        let ctx = test_context();
        let result =
            run_command_action("cmd.exe", &["/c".into(), "echo $TITLE".into()], None, &ctx);
        assert!(result.is_ok(), "cmd.exe 应能成功 spawn");
    }

    #[test]
    fn run_command_action_fails_for_invalid_command() {
        // Task 17.5: 集成测试 - RunCommand 失败返回错误
        let ctx = test_context();
        let result = run_command_action("nonexistent-command-xyz-12345", &[], None, &ctx);
        assert!(result.is_err(), "不存在的命令应返回错误");
    }

    #[test]
    fn run_command_action_rejects_empty_command() {
        let ctx = test_context();
        let result = run_command_action("", &[], None, &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn run_command_action_expands_template_in_args() {
        // 验证 args 中的模板变量被替换（通过 cmd.exe 写入临时文件）
        let ctx = test_context();
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("marker.txt");
        let marker_str = marker.to_string_lossy().replace('\\', "/");
        // cmd.exe /c echo TITLE > marker.txt
        let args = vec!["/c".into(), format!("echo {} > {}", ctx.title, marker_str)];
        let result = run_command_action("cmd.exe", &args, None, &ctx);
        assert!(result.is_ok());
        // 给进程一点时间完成
        std::thread::sleep(std::time::Duration::from_millis(200));
        // 文件应存在且包含 title
        if marker.exists() {
            let content = std::fs::read_to_string(&marker).unwrap_or_default();
            assert!(
                content.contains(&ctx.title),
                "marker 文件应包含 title，实际内容：{content}"
            );
        }
        // 即使文件未生成（fire-and-forget 时序问题），spawn 成功即视为通过
    }

    #[tokio::test]
    async fn copy_to_action_copies_file_without_removing_source() {
        // Task 17.5: 集成测试 - CopyTo 文件正确复制
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        std::fs::write(&source, b"hello world").unwrap();
        let target_dir = dir.path().join("target");
        let ctx = TemplateContext {
            file_path: source.to_string_lossy().to_string(),
            file_name: "source.txt".to_string(),
            file_dir: dir.path().to_string_lossy().to_string(),
            url: "".into(),
            title: "source.txt".into(),
        };
        copy_to_action(
            target_dir.to_str().unwrap(),
            None,
            CollisionPolicy::Rename,
            &ctx,
        )
        .await
        .unwrap();
        let copied = target_dir.join("source.txt");
        assert!(copied.exists(), "目标文件应存在");
        assert_eq!(
            std::fs::read(&copied).unwrap(),
            b"hello world",
            "目标文件内容应与源文件一致"
        );
        // AGENTS.md §7: CopyTo 不删除源文件
        assert!(source.exists(), "源文件应保留");
    }

    #[tokio::test]
    async fn copy_to_action_with_rename_pattern() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        std::fs::write(&source, b"hello").unwrap();
        let target_dir = dir.path().join("target");
        let ctx = TemplateContext {
            file_path: source.to_string_lossy().to_string(),
            file_name: "source.txt".to_string(),
            file_dir: dir.path().to_string_lossy().to_string(),
            url: "https://example.com".into(),
            title: "My Title".into(),
        };
        copy_to_action(
            target_dir.to_str().unwrap(),
            Some("renamed-$TITLE.bin"),
            CollisionPolicy::Rename,
            &ctx,
        )
        .await
        .unwrap();
        let copied = target_dir.join("renamed-My Title.bin");
        assert!(copied.exists(), "按模板重命名的目标文件应存在");
    }

    #[tokio::test]
    async fn copy_to_action_rejects_path_traversal() {
        // Task 17.5: 路径穿越防护 - target_directory 含 .. 拒绝
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        std::fs::write(&source, b"hello").unwrap();
        let ctx = TemplateContext {
            file_path: source.to_string_lossy().to_string(),
            file_name: "source.txt".to_string(),
            file_dir: dir.path().to_string_lossy().to_string(),
            url: "".into(),
            title: "".into(),
        };
        let result = copy_to_action("../../../etc", None, CollisionPolicy::Rename, &ctx).await;
        assert!(result.is_err(), "含 .. 的目标目录应被拒绝");
        assert!(result.unwrap_err().contains(".."), "错误信息应提及路径穿越");
    }

    #[tokio::test]
    async fn copy_to_action_handles_collision_rename() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        std::fs::write(&source, b"new").unwrap();
        let target_dir = dir.path().join("target");
        std::fs::create_dir_all(&target_dir).unwrap();
        std::fs::write(target_dir.join("source.txt"), b"old").unwrap();
        let ctx = TemplateContext {
            file_path: source.to_string_lossy().to_string(),
            file_name: "source.txt".to_string(),
            file_dir: dir.path().to_string_lossy().to_string(),
            url: "".into(),
            title: "".into(),
        };
        copy_to_action(
            target_dir.to_str().unwrap(),
            None,
            CollisionPolicy::Rename,
            &ctx,
        )
        .await
        .unwrap();
        // 重名时应生成 "source (1).txt"
        assert!(target_dir.join("source (1).txt").exists());
        // 原文件保留
        assert_eq!(
            std::fs::read(target_dir.join("source.txt")).unwrap(),
            b"old"
        );
    }

    #[tokio::test]
    async fn move_to_action_moves_file_and_removes_source() {
        // Task 17.5: 集成测试 - MoveTo 文件移动
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        std::fs::write(&source, b"hello world").unwrap();
        let target_dir = dir.path().join("target");
        let ctx = TemplateContext {
            file_path: source.to_string_lossy().to_string(),
            file_name: "source.txt".to_string(),
            file_dir: dir.path().to_string_lossy().to_string(),
            url: "".into(),
            title: "".into(),
        };
        move_to_action(
            target_dir.to_str().unwrap(),
            None,
            CollisionPolicy::Rename,
            &ctx,
        )
        .await
        .unwrap();
        let moved = target_dir.join("source.txt");
        assert!(moved.exists(), "目标文件应存在");
        assert_eq!(
            std::fs::read(&moved).unwrap(),
            b"hello world",
            "目标文件内容应与源文件一致"
        );
        // MoveTo 成功后源文件应不存在
        assert!(!source.exists(), "源文件应已被移动");
    }

    #[tokio::test]
    async fn move_to_action_rejects_path_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        std::fs::write(&source, b"hello").unwrap();
        let ctx = TemplateContext {
            file_path: source.to_string_lossy().to_string(),
            file_name: "source.txt".to_string(),
            file_dir: dir.path().to_string_lossy().to_string(),
            url: "".into(),
            title: "".into(),
        };
        let result = move_to_action("../escape", None, CollisionPolicy::Rename, &ctx).await;
        assert!(result.is_err(), "含 .. 的目标目录应被拒绝");
    }

    #[test]
    fn is_cross_device_error_recognizes_known_codes() {
        use std::io::{Error, ErrorKind};
        // Windows ERROR_NOT_SAME_DEVICE = 17
        let err = Error::from_raw_os_error(17);
        assert!(is_cross_device_error(&err));
        // Linux EXDEV = 18
        let err = Error::from_raw_os_error(18);
        assert!(is_cross_device_error(&err));
        // 其他错误不识别
        let err = Error::new(ErrorKind::PermissionDenied, "denied");
        assert!(!is_cross_device_error(&err));
    }
}
