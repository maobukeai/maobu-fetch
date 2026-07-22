//! Task 35：命令行接口。
//!
//! 提供轻量 CLI 解析，支持 `add` / `list` / `pause` / `resume` / `remove` 子命令。
//! 解析逻辑为纯函数，不产生副作用，便于单元测试。实际执行由 `lib::run_cli` 完成。
//!
//! 设计要点（AGENTS.md §8）：
//! - 使用 `pico-args`（<10KB，MIT）避免引入 clap 等大型依赖。
//! - 用户可见文案使用简体中文；CLI 输出同样使用中文。
//! - 连接数严格校验为 `1 / 2 / 4 / 8 / 16 / 32`（AGENTS.md §3）。
//! - `parse_args` 不调用 `unwrap()` / `expect()`，所有可恢复错误返回 `Err(String)`。

use std::ffi::OsString;

/// CLI 子命令（Task 35.1）。
///
/// `Run` 变体表示无子命令，正常启动 GUI；其他变体对应各子命令。
/// 字段使用 `String` / `Option` 保持简单，由 `run_cli` 负责后续校验与执行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliCommand {
    /// 正常启动 GUI（无子命令或空参数）。
    Run,
    /// `add <url> --out <path> --connections <n>`（Task 35.2）。
    Add {
        url: String,
        out: Option<String>,
        connections: Option<u8>,
    },
    /// `list --status <status>`（Task 35.3）。
    List { status: Option<String> },
    /// `pause <id>`（Task 35.4）。
    Pause { id: String },
    /// `resume <id>`（Task 35.4）。
    Resume { id: String },
    /// `remove <id> [--delete-file]`（Task 35.4）。
    Remove { id: String, delete_file: bool },
}

/// 允许的连接数集合（AGENTS.md §3 强约束：只能为 1/2/4/8/16/32）。
const ALLOWED_CONNECTIONS: [u8; 6] = [1, 2, 4, 8, 16, 32];

/// 校验连接数为允许值之一。
fn parse_connections(raw: &str) -> Result<u8, pico_args::Error> {
    let n: u8 = raw
        .parse()
        .map_err(|_| pico_args::Error::ArgumentParsingFailed {
            cause: "连接数必须是 1/2/4/8/16/32 之一".into(),
        })?;
    if !ALLOWED_CONNECTIONS.contains(&n) {
        return Err(pico_args::Error::ArgumentParsingFailed {
            cause: "连接数必须是 1/2/4/8/16/32 之一".into(),
        });
    }
    Ok(n)
}

/// 把字符串解析为 `Option<String>`，用于 `--out` / `--status` 等。
fn parse_optional_string(raw: &str) -> Result<String, pico_args::Error> {
    Ok(raw.to_string())
}

/// 解析命令行参数为 [`CliCommand`]（Task 35.1）。
///
/// `args` 应为 `std::env::args().collect()` 的结果，包含 `args[0]`（程序路径）。
/// 空参数或无子命令时返回 [`CliCommand::Run`]。
///
/// 错误处理（AGENTS.md §7）：所有解析失败返回中文错误信息，不 panic。
pub fn parse_args(args: Vec<String>) -> Result<CliCommand, String> {
    // args[0] 是程序路径，跳过。无后续参数时正常启动 GUI。
    if args.len() <= 1 {
        return Ok(CliCommand::Run);
    }

    let first = &args[1];
    if first.starts_with("maobu://") || first.ends_with(".maobu-task") || first.ends_with(".maobu")
    {
        return Ok(CliCommand::Run);
    }

    let mut pargs =
        pico_args::Arguments::from_vec(args.into_iter().skip(1).map(OsString::from).collect());

    let subcommand = pargs
        .subcommand()
        .map_err(|e| format!("参数解析失败：{e}"))?;

    match subcommand.as_deref() {
        None | Some("") => Ok(CliCommand::Run),
        Some("add") => {
            let url = pargs
                .subcommand()
                .map_err(|e| format!("参数解析失败：{e}"))?
                .ok_or_else(|| "缺少必填参数：URL".to_string())?;
            if url.trim().is_empty() {
                return Err("URL 不能为空".into());
            }
            let out = pargs
                .opt_value_from_fn("--out", parse_optional_string)
                .map_err(|e| format!("参数解析失败：{e}"))?;
            let connections = pargs
                .opt_value_from_fn("--connections", parse_connections)
                .map_err(|e| format!("参数解析失败：{e}"))?;
            Ok(CliCommand::Add {
                url,
                out,
                connections,
            })
        }
        Some("list") => {
            let status = pargs
                .opt_value_from_fn("--status", parse_optional_string)
                .map_err(|e| format!("参数解析失败：{e}"))?;
            Ok(CliCommand::List { status })
        }
        Some("pause") => {
            let id = pargs
                .subcommand()
                .map_err(|e| format!("参数解析失败：{e}"))?
                .ok_or_else(|| "缺少必填参数：任务 ID".to_string())?;
            if id.trim().is_empty() {
                return Err("任务 ID 不能为空".into());
            }
            Ok(CliCommand::Pause { id })
        }
        Some("resume") => {
            let id = pargs
                .subcommand()
                .map_err(|e| format!("参数解析失败：{e}"))?
                .ok_or_else(|| "缺少必填参数：任务 ID".to_string())?;
            if id.trim().is_empty() {
                return Err("任务 ID 不能为空".into());
            }
            Ok(CliCommand::Resume { id })
        }
        Some("remove") => {
            let id = pargs
                .subcommand()
                .map_err(|e| format!("参数解析失败：{e}"))?
                .ok_or_else(|| "缺少必填参数：任务 ID".to_string())?;
            if id.trim().is_empty() {
                return Err("任务 ID 不能为空".into());
            }
            let delete_file = pargs.contains("--delete-file");
            Ok(CliCommand::Remove { id, delete_file })
        }
        Some(other) => {
            if other.starts_with("maobu://")
                || other.ends_with(".maobu-task")
                || other.ends_with(".maobu")
                || other.starts_with('-')
            {
                Ok(CliCommand::Run)
            } else {
                Err(format!("未知子命令：{other}"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造测试用参数向量，自动补上程序路径占位。
    fn args(parts: &[&str]) -> Vec<String> {
        let mut v = vec!["maobu".to_string()];
        v.extend(parts.iter().map(|s| s.to_string()));
        v
    }

    #[test]
    fn no_args_returns_run() {
        let cmd = parse_args(vec!["maobu".to_string()]).unwrap();
        assert_eq!(cmd, CliCommand::Run);
    }

    #[test]
    fn empty_subcommand_returns_run() {
        let cmd = parse_args(args(&[])).unwrap();
        assert_eq!(cmd, CliCommand::Run);
    }

    #[test]
    fn add_with_url_only() {
        let cmd = parse_args(args(&["add", "https://example.com/file.zip"])).unwrap();
        assert_eq!(
            cmd,
            CliCommand::Add {
                url: "https://example.com/file.zip".into(),
                out: None,
                connections: None,
            }
        );
    }

    #[test]
    fn add_with_out_and_connections() {
        let cmd = parse_args(args(&[
            "add",
            "https://example.com/file.zip",
            "--out",
            "C:\\Downloads",
            "--connections",
            "8",
        ]))
        .unwrap();
        assert_eq!(
            cmd,
            CliCommand::Add {
                url: "https://example.com/file.zip".into(),
                out: Some("C:\\Downloads".into()),
                connections: Some(8),
            }
        );
    }

    #[test]
    fn add_with_connections_32() {
        let cmd = parse_args(args(&[
            "add",
            "https://example.com/file.zip",
            "--connections",
            "32",
        ]))
        .unwrap();
        assert_eq!(
            cmd,
            CliCommand::Add {
                url: "https://example.com/file.zip".into(),
                out: None,
                connections: Some(32),
            }
        );
    }

    #[test]
    fn add_missing_url_returns_error() {
        let result = parse_args(args(&["add"]));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("URL"));
    }

    #[test]
    fn add_invalid_connections_returns_error() {
        let result = parse_args(args(&[
            "add",
            "https://example.com/file.zip",
            "--connections",
            "7",
        ]));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("连接数"));
    }

    #[test]
    fn add_non_numeric_connections_returns_error() {
        let result = parse_args(args(&[
            "add",
            "https://example.com/file.zip",
            "--connections",
            "abc",
        ]));
        assert!(result.is_err());
    }

    #[test]
    fn list_no_status() {
        let cmd = parse_args(args(&["list"])).unwrap();
        assert_eq!(cmd, CliCommand::List { status: None });
    }

    #[test]
    fn list_with_status_downloading() {
        let cmd = parse_args(args(&["list", "--status", "downloading"])).unwrap();
        assert_eq!(
            cmd,
            CliCommand::List {
                status: Some("downloading".into()),
            }
        );
    }

    #[test]
    fn list_with_status_completed() {
        let cmd = parse_args(args(&["list", "--status", "completed"])).unwrap();
        assert_eq!(
            cmd,
            CliCommand::List {
                status: Some("completed".into()),
            }
        );
    }

    #[test]
    fn pause_with_id() {
        let cmd = parse_args(args(&["pause", "task-abc-123"])).unwrap();
        assert_eq!(
            cmd,
            CliCommand::Pause {
                id: "task-abc-123".into(),
            }
        );
    }

    #[test]
    fn pause_missing_id_returns_error() {
        let result = parse_args(args(&["pause"]));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("任务 ID"));
    }

    #[test]
    fn resume_with_id() {
        let cmd = parse_args(args(&["resume", "task-xyz"])).unwrap();
        assert_eq!(
            cmd,
            CliCommand::Resume {
                id: "task-xyz".into(),
            }
        );
    }

    #[test]
    fn resume_missing_id_returns_error() {
        let result = parse_args(args(&["resume"]));
        assert!(result.is_err());
    }

    #[test]
    fn remove_without_delete_file() {
        let cmd = parse_args(args(&["remove", "task-1"])).unwrap();
        assert_eq!(
            cmd,
            CliCommand::Remove {
                id: "task-1".into(),
                delete_file: false,
            }
        );
    }

    #[test]
    fn remove_with_delete_file() {
        let cmd = parse_args(args(&["remove", "task-1", "--delete-file"])).unwrap();
        assert_eq!(
            cmd,
            CliCommand::Remove {
                id: "task-1".into(),
                delete_file: true,
            }
        );
    }

    #[test]
    fn remove_missing_id_returns_error() {
        let result = parse_args(args(&["remove"]));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("任务 ID"));
    }

    #[test]
    fn unknown_subcommand_returns_error() {
        let result = parse_args(args(&["foobar"]));
        assert!(result.is_err());
        let error = result.unwrap_err();
        assert!(error.contains("未知子命令"));
        assert!(error.contains("foobar"));
    }

    #[test]
    fn all_allowed_connection_values() {
        for n in [1u8, 2, 4, 8, 16, 32] {
            let cmd = parse_args(args(&[
                "add",
                "https://example.com/file.zip",
                "--connections",
                &n.to_string(),
            ]))
            .unwrap_or_else(|e| panic!("连接数 {n} 应被接受，但解析失败：{e}"));
            assert_eq!(
                cmd,
                CliCommand::Add {
                    url: "https://example.com/file.zip".into(),
                    out: None,
                    connections: Some(n),
                }
            );
        }
    }
}
