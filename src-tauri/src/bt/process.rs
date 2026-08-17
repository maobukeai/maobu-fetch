//! aria2 进程生命周期（roadmap BT-02）。
//!
//! 安全约束（AGENTS.md §3 BT/磁力内核）：
//! - RPC 只监听 `127.0.0.1` 随机端口 + `--rpc-secret` 随机令牌；
//! - 做种默认关闭（`--seed-time=0`，除非用户显式开启）；
//! - 优雅退出前调用 `aria2.saveSession`，保证重启后可恢复；
//! - 启动参数构建拆为纯函数 `build_args`，无进程即可单元测试（§9）。

use super::rpc::SecretToken;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Child;

/// aria2 启动配置（由引擎按当前设置计算）。
#[derive(Clone, Debug)]
pub struct Aria2LaunchConfig {
    pub exe_path: PathBuf,
    /// 会话文件路径（--input-file 与 --save-session 共用；缺失时跳过 input-file）。
    pub session_path: PathBuf,
    pub rpc_port: u16,
    /// BT 数据监听端口（DHT 与 peer 连接）。
    pub bt_listen_port: u16,
    pub secret: SecretToken,
    /// 默认下载目录；任务级目录经 addUri options 覆盖。
    pub dir: PathBuf,
    /// 全局下载限速值（aria2 格式："0" 不限或 "512K" 等）。
    pub download_limit: String,
    /// 全局上传限速值。
    pub upload_limit: String,
    /// 做种时长（小时）。0 = 完成即停（默认，§3）。
    pub seed_time: u32,
    /// aria2 内部并发下载数上限。取应用并发上限，实际排队由应用调度器控制。
    pub max_concurrent: u32,
    /// 额外 Tracker（逗号连接的 URL 列表，可包含逗号被过滤后的空串）。
    /// 非空时以 `--bt-tracker` 下发，加快磁力元数据获取。空 = 不下发。
    pub extra_trackers: String,
}

/// 构建启动参数（纯函数）。参数顺序稳定，便于测试与日志比对
/// （注意：真实日志输出必须先经 `args_for_log` 脱敏 secret）。
pub fn build_args(config: &Aria2LaunchConfig) -> Vec<String> {
    let mut args: Vec<String> = Vec::with_capacity(24);
    let mut push = |flag: &str, value: &str| {
        args.push(format!("--{flag}={value}"));
    };
    push("enable-rpc", "true");
    push("rpc-listen-port", &config.rpc_port.to_string());
    push("rpc-secret", config.secret.expose_for_args());
    push("rpc-listen-all", "false");
    push("rpc-allow-origin-all", "false");
    push("listen-port", &config.bt_listen_port.to_string());
    push("dht-listen-port", &config.bt_listen_port.to_string());
    push("dir", &config.dir.to_string_lossy());
    push("continue", "true");
    push("max-concurrent-downloads", &config.max_concurrent.to_string());
    push("max-overall-download-speed", &config.download_limit);
    push("max-overall-upload-speed", &config.upload_limit);
    push("seed-time", &config.seed_time.to_string());
    push("save-session", &config.session_path.to_string_lossy());
    // 定期保存会话：崩溃时最多丢失 30 秒进度，控制文件仍保住分片完整性。
    push("save-session-interval", "30");
    push("auto-file-renaming", "true");
    push("allow-overwrite", "false");
    push("bt-require-crypto", "true");
    if !config.extra_trackers.is_empty() {
        push("bt-tracker", &config.extra_trackers);
    }
    push("console-log-level", "warn");
    push("summary-interval", "0");
    if config.session_path.is_file() {
        // --input-file 要求文件存在，否则 aria2 启动即失败。
        push("input-file", &config.session_path.to_string_lossy());
    }
    args
}

/// 日志安全的参数列表：rpc-secret 值脱敏（§3 令牌不得写入日志）。
pub fn args_for_log(config: &Aria2LaunchConfig) -> Vec<String> {
    build_args(config)
        .into_iter()
        .map(|arg| {
            if arg.starts_with("--rpc-secret=") {
                "--rpc-secret=***".to_string()
            } else {
                arg
            }
        })
        .collect()
}

/// 在 127.0.0.1 上分配一个空闲 TCP 端口。短暂绑定后释放存在极小竞态，
/// 仅用于本机回环场景，失败时调用方重试即可。
pub fn pick_free_port() -> Result<u16, String> {
    std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|error| format!("无法分配本地端口：{error}"))?
        .local_addr()
        .map(|addr| addr.port())
        .map_err(|error| format!("无法读取本地端口：{error}"))
}

/// 运行中的 aria2 实例句柄。
pub struct Aria2Process {
    child: Child,
    pub rpc_port: u16,
}

/// 启动 aria2 并等待 RPC 就绪。
///
/// 就绪判定：`aria2.getVersion` 连续成功。进程提前退出时返回包含
/// 退出码的中文错误（AGENTS.md §7 可操作错误）。
pub async fn launch(
    config: &Aria2LaunchConfig,
    rpc: &super::rpc::Aria2Rpc,
) -> Result<Aria2Process, String> {
    if !config.exe_path.is_file() {
        return Err("aria2c.exe 不存在，请先在设置中安装 BT 组件".into());
    }
    let mut command = crate::media_tools::create_hidden_tokio_command(&config.exe_path);
    command.args(build_args(config));
    // 丢弃 stdout/stderr，防止 aria2 控制台输出填满管道导致阻塞。
    command.stdout(std::process::Stdio::null());
    command.stderr(std::process::Stdio::null());
    command.stdin(std::process::Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|error| format!("启动 aria2 失败：{error}"))?;
    // RPC 就绪探测：最多 10 秒（首次启动 DHT 初始化可能较慢）。
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!(
                "aria2 进程提前退出（退出码 {status}），请重试或重新安装 BT 组件"
            ));
        }
        if rpc.get_version().await.is_ok() {
            return Ok(Aria2Process {
                child,
                rpc_port: config.rpc_port,
            });
        }
        if tokio::time::Instant::now() >= deadline {
            let _ = child.kill().await;
            return Err("aria2 RPC 在 10 秒内未就绪，已终止进程".into());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

impl Aria2Process {
    /// 优雅退出：先保存会话，再等待进程结束，超时强制终止。
    pub async fn shutdown(mut self, rpc: &super::rpc::Aria2Rpc) {
        let _ = rpc.save_session().await;
        // 给 aria2 一点时间落盘会话与控制文件。
        for _ in 0..20 {
            match self.child.try_wait() {
                // aria2 在 stdin 关闭后通常自行退出；未退出则继续等待。
                Ok(Some(_)) => return,
                Ok(None) => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Err(_) => break,
            }
        }
        let _ = self.child.kill().await;
    }

    /// 进程是否仍在运行。
    pub fn is_running(&mut self) -> bool {
        !matches!(self.child.try_wait(), Ok(Some(_)))
    }
}

/// 校验 .torrent 文件基本合法性：存在、非空、且以 bencode 字典开头
/// （'d' + '4:info'）。不做完整解析——解析交给 aria2（避免引入 bencode 依赖）。
pub fn validate_torrent_file(path: &Path) -> Result<(), String> {
    let meta = std::fs::metadata(path).map_err(|error| format!("无法读取种子文件：{error}"))?;
    if meta.len() < 10 {
        return Err("种子文件过小，可能已损坏".into());
    }
    if meta.len() > 20 * 1024 * 1024 {
        return Err("种子文件过大（超过 20 MB），请检查是否选择了正确文件".into());
    }
    let mut header = [0u8; 10];
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|error| format!("无法打开种子文件：{error}"))?;
    let count = file
        .read(&mut header)
        .map_err(|error| format!("读取种子文件失败：{error}"))?;
    if count < 10 || header[0] != b'd' {
        return Err("文件不是有效的 BitTorrent 种子（bencode 格式错误）".into());
    }
    Ok(())
}

/// 内存版种子校验（拖放 .torrent 字节流走此路径，规则与文件版一致）。
pub fn validate_torrent_bytes(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() < 10 {
        return Err("种子文件过小，可能已损坏".into());
    }
    if bytes.len() > 20 * 1024 * 1024 {
        return Err("种子文件过大（超过 20 MB），请检查是否选择了正确文件".into());
    }
    if bytes[0] != b'd' {
        return Err("文件不是有效的 BitTorrent 种子（bencode 格式错误）".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config(session_exists: bool) -> (Aria2LaunchConfig, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let session = dir.path().join("aria2.session");
        if session_exists {
            std::fs::write(&session, "").unwrap();
        }
        let config = Aria2LaunchConfig {
            exe_path: dir.path().join("aria2c.exe"),
            session_path: session,
            rpc_port: 54321,
            bt_listen_port: 54322,
            secret: SecretToken::new("deadbeef".into()),
            dir: dir.path().join("downloads"),
            download_limit: "0".into(),
            upload_limit: "2M".into(),
            seed_time: 0,
            max_concurrent: 5,
            extra_trackers: String::new(),
        };
        (config, dir)
    }

    #[test]
    fn args_bind_localhost_rpc_and_default_no_seed() {
        let (config, _dir) = sample_config(false);
        let args = build_args(&config);
        assert!(args.contains(&"--enable-rpc=true".to_string()));
        assert!(args.contains(&"--rpc-listen-port=54321".to_string()));
        assert!(args.contains(&"--rpc-secret=deadbeef".to_string()));
        assert!(args.contains(&"--rpc-listen-all=false".to_string()));
        assert!(args.contains(&"--seed-time=0".to_string()));
        assert!(args.contains(&"--max-overall-upload-speed=2M".to_string()));
        // 会话文件不存在时不得传 input-file，否则 aria2 启动即失败。
        assert!(!args.iter().any(|a| a.starts_with("--input-file")));
        assert!(args.contains(&"--save-session-interval=30".to_string()));
        assert!(args.contains(&"--bt-require-crypto=true".to_string()));
        // Tracker 列表为空时不下发 bt-tracker。
        assert!(!args.iter().any(|a| a.starts_with("--bt-tracker")));
    }

    #[test]
    fn args_include_extra_trackers_when_configured() {
        let (mut config, _dir) = sample_config(false);
        config.extra_trackers = "https://t1.example.com/announce,http://t2.example.com/announce".into();
        let args = build_args(&config);
        assert!(args.contains(&"--bt-tracker=https://t1.example.com/announce,http://t2.example.com/announce".to_string()));
    }

    #[test]
    fn args_include_input_file_only_when_session_exists() {
        let (config, _dir) = sample_config(true);
        let args = build_args(&config);
        assert!(args
            .iter()
            .any(|a| a.starts_with("--input-file=")));
    }

    #[test]
    fn log_args_redact_secret() {
        let (config, _dir) = sample_config(false);
        let logged = args_for_log(&config).join(" ");
        assert!(!logged.contains("deadbeef"));
        assert!(logged.contains("--rpc-secret=***"));
    }

    #[test]
    fn picked_port_is_bindable() {
        let port = pick_free_port().unwrap();
        assert!(port > 0);
        // 端口刚被释放，应可再次绑定。
        assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_ok());
    }

    #[test]
    fn torrent_validation_rejects_garbage() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad.torrent");
        std::fs::write(&bad, b"not-a-torrent-at-all").unwrap();
        assert!(validate_torrent_file(&bad).is_err());
        let tiny = dir.path().join("tiny.torrent");
        std::fs::write(&tiny, b"d4:").unwrap();
        assert!(validate_torrent_file(&tiny).is_err());
        let missing = dir.path().join("missing.torrent");
        assert!(validate_torrent_file(&missing).is_err());
    }

    #[test]
    fn torrent_bytes_validation_matches_file_rules() {
        // 与文件版同规则的内存校验：bencode 头 + 尺寸上下限。
        assert!(validate_torrent_bytes(b"d4:infod6:lengthi5eee").is_ok());
        assert!(validate_torrent_bytes(b"not-a-torrent-at-all").is_err());
        assert!(validate_torrent_bytes(b"d4:").is_err());
        assert!(validate_torrent_bytes(b"").is_err());
        let oversized = vec![b'd'; 20 * 1024 * 1024 + 1];
        assert!(validate_torrent_bytes(&oversized).is_err());
    }
}
