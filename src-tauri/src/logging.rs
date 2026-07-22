//! 日志系统（Task 23）。
//!
//! 基于 `tracing` + `tracing-appender` + `tracing-subscriber` 实现：
//! - 按天滚动日志文件，保留 7 天
//! - 同时输出到 stderr（开发模式）和文件
//! - 自定义 writer，在写入前对每行调用 `redact_sensitive` 脱敏
//!
//! 安全约束（AGENTS.md §3 / §7）：
//! - 认证信息、Cookie、Authorization、代理密码和持久令牌不得写入日志
//! - 双保险：写入时脱敏 + 导出时再脱敏（见 `export_recent_logs`）
//!
//! 路径不暴露到前端：所有路径操作仅在 Rust 侧进行，前端只能通过
//! `open_logs_dir` 命令打开目录、`export_recent_logs` 命令导出。

use crate::manager::redact_sensitive;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::{Duration, SystemTime},
};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt::MakeWriter, layer::SubscriberExt, util::SubscriberInitExt, Layer};

/// 日志保留天数。
const MAX_LOG_FILES: usize = 7;

/// 全局 guard，drop 时关闭非阻塞 writer。
/// 进程退出前必须保持存活，否则日志可能丢失。
static LOG_GUARD: OnceLock<WorkerGuard> = OnceLock::new();

/// 全局日志目录，供 `open_logs_dir` 和 `export_recent_logs` 使用。
static LOG_DIR: OnceLock<PathBuf> = OnceLock::new();

/// 初始化日志系统。
///
/// - `log_dir`：日志文件存放目录（`app_data_dir/logs/`）
/// - `debug`：true 时启用 DEBUG 级别；false 时为 INFO
///
/// 初始化失败仅记录到 stderr，不阻塞启动。
pub fn init(log_dir: PathBuf, debug: bool) {
    if let Err(error) = fs::create_dir_all(&log_dir) {
        eprintln!("无法创建日志目录：{error}");
        return;
    }

    let file_appender = tracing_appender::rolling::daily(&log_dir, "maobu.log");
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);
    let make_writer = RedactingMakeWriter::new(file_writer);

    let level_filter = if debug {
        tracing_subscriber::EnvFilter::new("debug")
    } else {
        tracing_subscriber::EnvFilter::new("info")
    };

    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(false)
        .with_filter(level_filter.clone());

    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(make_writer)
        .with_target(false)
        .with_ansi(false)
        .with_filter(level_filter);

    let result = tracing_subscriber::registry()
        .with(stderr_layer)
        .with(file_layer)
        .try_init();

    if let Err(error) = result {
        eprintln!("日志系统已存在或初始化失败：{error}");
    }

    let _ = LOG_GUARD.set(guard);
    let _ = LOG_DIR.set(log_dir.clone());

    if let Err(error) = cleanup_old_logs(&log_dir) {
        tracing::warn!("清理过期日志失败：{error}");
    }

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        "猫步下载器日志系统已初始化"
    );
}

/// 返回日志目录，未初始化时返回 None。
pub fn log_dir() -> Option<&'static Path> {
    LOG_DIR.get().map(PathBuf::as_path)
}

/// 清理过期日志文件（保留最近 `MAX_LOG_FILES` 个）。
///
/// 只删除以 `maobu.log` 开头的文件，避免误删其他文件。
fn cleanup_old_logs(dir: &Path) -> Result<(), String> {
    let mut files = list_log_files(dir)?;
    // 按修改时间降序：最新的在前
    files.sort_by(|a, b| b.1.cmp(&a.1));
    // 跳过最新 MAX_LOG_FILES 个，删除其余
    for (path, _) in files.iter().skip(MAX_LOG_FILES) {
        if let Err(error) = fs::remove_file(path) {
            tracing::warn!("删除过期日志失败：{error}");
        }
    }
    Ok(())
}

/// 从指定目录返回最近 `hours` 小时内修改过的日志文件路径列表。
///
/// 仅返回以 `maobu.log` 开头的文件，避免误读其他文件。按修改时间升序排列。
/// 接受外部目录参数，便于测试和处理 logging::init 未调用的场景。
pub fn recent_log_files_in(dir: &Path, hours: u64) -> Result<Vec<PathBuf>, String> {
    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(hours * 3600))
        .ok_or_else(|| "时间回溯失败".to_string())?;

    let mut files = list_log_files(dir)?;
    files.retain(|(_, modified)| *modified >= cutoff);
    files.sort_by(|a, b| a.1.cmp(&b.1));
    Ok(files.into_iter().map(|(p, _)| p).collect())
}

/// 列出日志目录下所有 `maobu.log*` 文件及其修改时间。
fn list_log_files(dir: &Path) -> Result<Vec<(PathBuf, SystemTime)>, String> {
    let entries = fs::read_dir(dir).map_err(|e| e.to_string())?;
    let mut files: Vec<(PathBuf, SystemTime)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if !name.starts_with("maobu.log") {
            continue;
        }
        if let Ok(meta) = entry.metadata() {
            if let Ok(modified) = meta.modified() {
                files.push((path, modified));
            }
        }
    }
    Ok(files)
}

/// 把多个日志文件按顺序拼接为单个文件，并对每行再次调用 `redact_sensitive`。
///
/// 输出路径必须是可写的有效路径。失败时返回中文错误。
pub fn write_logs_to_file(files: &[PathBuf], output_path: &Path) -> Result<(), String> {
    let mut out = fs::File::create(output_path).map_err(|e| format!("无法创建导出文件：{e}"))?;
    for file in files {
        let content = fs::read_to_string(file).map_err(|e| format!("无法读取日志文件：{e}"))?;
        for line in content.lines() {
            let redacted = redact_sensitive(line);
            writeln!(out, "{redacted}").map_err(|e| format!("写入失败：{e}"))?;
        }
    }
    out.flush().map_err(|e| format!("flush 失败：{e}"))?;
    Ok(())
}

// ===== 脱敏 writer =====

/// 包装 `tracing_appender::non_blocking::NonBlocking` writer，
/// 在写入前对每行调用 `redact_sensitive` 脱敏。
///
/// tracing-subscriber 的 fmt layer 会调用 `MakeWriter::make_writer`
/// 拿到一个 `Write` 实例，然后把格式化后的日志行写入。
/// 我们在这里拦截写入字节，按 `\n` 切分后脱敏再写。
struct RedactingMakeWriter {
    inner: tracing_appender::non_blocking::NonBlocking,
}

impl RedactingMakeWriter {
    fn new(inner: tracing_appender::non_blocking::NonBlocking) -> Self {
        Self { inner }
    }
}

impl<'a> MakeWriter<'a> for RedactingMakeWriter {
    type Writer = RedactingWriter;

    fn make_writer(&'a self) -> Self::Writer {
        RedactingWriter::new(self.inner.clone())
    }
}

/// 脱敏 writer：内部缓冲，遇到 `\n` 时对当前行脱敏后写入底层 writer。
///
/// 不直接写底层 writer 的字节，而是按行处理。这对于 tracing 默认的
/// 行式格式（每条日志一行）是安全的。
pub struct RedactingWriter {
    inner: tracing_appender::non_blocking::NonBlocking,
    buf: Vec<u8>,
}

impl RedactingWriter {
    fn new(inner: tracing_appender::non_blocking::NonBlocking) -> Self {
        Self {
            inner,
            buf: Vec::with_capacity(512),
        }
    }

    fn flush_line(&mut self) {
        if self.buf.is_empty() {
            return;
        }
        let line = String::from_utf8_lossy(&self.buf);
        let redacted = redact_sensitive(&line);
        let _ = self.inner.write_all(redacted.as_bytes());
        self.buf.clear();
    }
}

impl Write for RedactingWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        for &b in bytes {
            if b == b'\n' {
                self.flush_line();
                let _ = self.inner.write_all(b"\n");
            } else {
                self.buf.push(b);
            }
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.flush_line();
        self.inner.flush()
    }
}

// ===== 单元测试 =====
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::OpenOptions;

    /// 以写权限打开文件并设置修改时间。
    ///
    /// Windows 上 `File::open` 默认只读，`set_modified` 会因权限不足失败。
    /// 必须用 `OpenOptions::new().write(true).open()` 获取写句柄。
    fn set_mtime(path: &Path, time: SystemTime) {
        let file = OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(time).unwrap();
    }

    /// 模拟日志写入并验证脱敏结果。
    #[test]
    fn redact_writer_strips_cookie_and_authorization() {
        let sensitive = "Cookie: session=abc123\nAuthorization: Bearer xyz\ntoken=secret\n";
        let redacted = redact_sensitive(sensitive);
        assert!(redacted.contains("Cookie: ***"));
        assert!(redacted.contains("Authorization: ***"));
        assert!(redacted.contains("token=***"));
        assert!(!redacted.contains("abc123"));
        assert!(!redacted.contains("Bearer xyz"));
        assert!(!redacted.contains("secret"));
    }

    /// 验证按行脱敏：每行单独脱敏，行间不影响。
    #[test]
    fn redact_per_line_preserves_structure() {
        let lines = "line 1: password=hunter2\nline 2: clean log\nline 3: token=abc\n";
        let redacted = redact_sensitive(lines);
        let redacted_lines: Vec<&str> = redacted.lines().collect();
        assert_eq!(redacted_lines.len(), 3);
        assert!(redacted_lines[0].contains("password=***"));
        assert!(redacted_lines[1].contains("clean log"));
        assert!(redacted_lines[2].contains("token=***"));
    }

    /// 验证 URL 中的 token 段被脱敏。
    #[test]
    fn redact_url_token_in_log() {
        let log = "GET https://example.com/file?token=secret123&sign=abc456";
        let redacted = redact_sensitive(log);
        assert!(redacted.contains("token=***"));
        assert!(redacted.contains("sign=***"));
        assert!(!redacted.contains("secret123"));
        assert!(!redacted.contains("abc456"));
    }

    /// 验证代理密码被脱敏。
    #[test]
    fn redact_proxy_password_in_log() {
        let log = "proxy-password: s3cret\nproxy_password=another";
        let redacted = redact_sensitive(log);
        assert!(redacted.contains("proxy-password: ***"));
        assert!(redacted.contains("proxy_password=***"));
        assert!(!redacted.contains("s3cret"));
        assert!(!redacted.contains("another"));
    }

    /// 验证 `write_logs_to_file` 双保险：拼接 + 每行再脱敏。
    #[test]
    fn write_logs_to_file_redacts_each_line() {
        let temp = tempfile::tempdir().unwrap();
        let log_file = temp.path().join("maobu.log.test");
        std::fs::write(&log_file, "Cookie: session=abc\ntoken=secret\nclean line\n").unwrap();
        let out = temp.path().join("export.txt");
        write_logs_to_file(&[log_file], &out).unwrap();
        let content = std::fs::read_to_string(&out).unwrap();
        assert!(content.contains("Cookie: ***"));
        assert!(content.contains("token=***"));
        assert!(content.contains("clean line"));
        assert!(!content.contains("session=abc"));
        assert!(!content.contains("secret"));
    }

    /// 验证 `cleanup_old_logs` 只保留最新 7 个文件。
    #[test]
    fn cleanup_old_logs_keeps_latest_seven() {
        let temp = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        for i in 0..10u64 {
            let path = temp.path().join(format!("maobu.log.{}", i));
            std::fs::write(&path, b"test").unwrap();
            // i 越大越新
            let time = now - Duration::from_secs((100 - i) * 86400);
            set_mtime(&path, time);
        }
        cleanup_old_logs(temp.path()).unwrap();
        let remaining = std::fs::read_dir(temp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
            .count();
        assert_eq!(remaining, MAX_LOG_FILES);
    }

    /// 验证 `cleanup_old_logs` 不删除非 `maobu.log` 开头的文件。
    #[test]
    fn cleanup_old_logs_skips_unrelated_files() {
        let temp = tempfile::tempdir().unwrap();
        let unrelated = temp.path().join("other.log");
        std::fs::write(&unrelated, b"keep me").unwrap();
        cleanup_old_logs(temp.path()).unwrap();
        assert!(unrelated.exists());
    }

    /// 模拟 grep 验证：日志文件中不含敏感字段原文。
    #[test]
    fn exported_log_has_no_sensitive_strings() {
        let temp = tempfile::tempdir().unwrap();
        let log_file = temp.path().join("maobu.log.test");
        let content = "INFO request with Cookie: session=secret\nERROR Authorization: Bearer abc\nwarn token=xyz\n";
        std::fs::write(&log_file, content).unwrap();
        let out = temp.path().join("export.txt");
        write_logs_to_file(&[log_file], &out).unwrap();
        let exported = std::fs::read_to_string(&out).unwrap();
        for forbidden in &["session=secret", "Bearer abc", "token=xyz"] {
            assert!(
                !exported.contains(forbidden),
                "导出文件中不应包含敏感字段 '{}': {exported}",
                forbidden
            );
        }
    }

    /// 验证 `list_log_files` 只返回 `maobu.log*` 文件。
    #[test]
    fn list_log_files_filters_prefix() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("maobu.log.2026-07-20"), b"a").unwrap();
        std::fs::write(temp.path().join("maobu.log"), b"b").unwrap();
        std::fs::write(temp.path().join("other.log"), b"c").unwrap();
        std::fs::write(temp.path().join("maobu.log.txt.bak"), b"d").unwrap();
        let files = list_log_files(temp.path()).unwrap();
        assert_eq!(files.len(), 3);
        for (path, _) in &files {
            let name = path.file_name().unwrap().to_str().unwrap();
            assert!(name.starts_with("maobu.log"));
        }
    }

    /// 集成测试：`recent_log_files_in` 时间过滤逻辑。
    #[test]
    fn recent_log_filter_logic() {
        let temp = tempfile::tempdir().unwrap();
        let now = SystemTime::now();

        let recent_path = temp.path().join("maobu.log.recent");
        std::fs::write(&recent_path, b"recent").unwrap();
        set_mtime(&recent_path, now);

        let old_path = temp.path().join("maobu.log.old");
        std::fs::write(&old_path, b"old").unwrap();
        set_mtime(&old_path, now - Duration::from_secs(48 * 3600));

        let cutoff = now - Duration::from_secs(24 * 3600);
        let mut files = list_log_files(temp.path()).unwrap();
        files.retain(|(_, modified)| *modified >= cutoff);
        assert_eq!(files.len(), 1);
        let (path, _) = &files[0];
        assert!(path
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .contains("recent"));
    }

    /// 端到端集成测试：模拟导出最近 24 小时日志的完整流程。
    ///
    /// 1. 创建多个日志文件，部分在 24 小时内、部分超出
    /// 2. 调用 `recent_log_files_in` 获取文件列表
    /// 3. 调用 `write_logs_to_file` 拼接并脱敏
    /// 4. 验证导出文件不含敏感字段
    #[test]
    fn end_to_end_export_recent_logs() {
        let temp = tempfile::tempdir().unwrap();
        let now = SystemTime::now();

        let log1 = temp.path().join("maobu.log.2026-07-19");
        std::fs::write(&log1, "INFO starting download\nCookie: session=secret1\n").unwrap();
        set_mtime(&log1, now - Duration::from_secs(2 * 3600));

        let log2 = temp.path().join("maobu.log.2026-07-20");
        std::fs::write(&log2, "ERROR Authorization: Bearer abc\ntoken=xyz\n").unwrap();
        set_mtime(&log2, now);

        let log_old = temp.path().join("maobu.log.2026-07-01");
        std::fs::write(&log_old, "old log password=hunter2\n").unwrap();
        set_mtime(&log_old, now - Duration::from_secs(48 * 3600));

        let unrelated = temp.path().join("other.log");
        std::fs::write(&unrelated, "unrelated\n").unwrap();

        // 1. 获取最近 24 小时的日志文件
        let files = recent_log_files_in(temp.path(), 24).unwrap();
        assert_eq!(files.len(), 2);

        // 2. 导出（拼接 + 每行脱敏）
        let out = temp.path().join("export.txt");
        write_logs_to_file(&files, &out).unwrap();
        assert!(out.exists());

        // 3. 验证导出文件内容
        let exported = std::fs::read_to_string(&out).unwrap();
        assert!(exported.contains("INFO starting download"));
        assert!(exported.contains("ERROR Authorization: ***"));
        assert!(exported.contains("token=***"));
        assert!(exported.contains("Cookie: ***"));
        // 4. 验证不含敏感字段原文（grep 风格断言）
        for forbidden in &[
            "session=secret1",
            "Bearer abc",
            "token=xyz",
            "password=hunter2",
        ] {
            assert!(
                !exported.contains(forbidden),
                "导出文件不应包含敏感字段 '{}': {exported}",
                forbidden
            );
        }
        assert!(!exported.contains("old log"));
        assert!(!exported.contains("unrelated"));
    }

    /// 验证 `recent_log_files_in` 返回空列表时导出会失败（不创建文件）。
    #[test]
    fn export_returns_error_when_no_recent_logs() {
        let temp = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let old = temp.path().join("maobu.log.old");
        std::fs::write(&old, b"old").unwrap();
        set_mtime(&old, now - Duration::from_secs(48 * 3600));

        let files = recent_log_files_in(temp.path(), 24).unwrap();
        assert!(files.is_empty());
    }

    /// 验证 `recent_log_files_in` 按修改时间升序返回。
    #[test]
    fn recent_log_files_in_returns_ascending_order() {
        let temp = tempfile::tempdir().unwrap();
        let now = SystemTime::now();

        let newer = temp.path().join("maobu.log.newer");
        std::fs::write(&newer, b"newer").unwrap();
        set_mtime(&newer, now);

        let older = temp.path().join("maobu.log.older");
        std::fs::write(&older, b"older").unwrap();
        set_mtime(&older, now - Duration::from_secs(3600));

        let files = recent_log_files_in(temp.path(), 24).unwrap();
        assert_eq!(files.len(), 2);
        assert!(files[0]
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .contains("older"));
        assert!(files[1]
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .contains("newer"));
    }
}
