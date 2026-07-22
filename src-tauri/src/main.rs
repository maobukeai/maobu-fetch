// Task 35：命令行接口入口。
//
// 启动时解析 CLI 参数：
// - `Run` 变体：正常启动 Tauri GUI（`maobu_fetch_lib::run`）。
// - 其他子命令变体：调用 `maobu_fetch_lib::run_cli` 执行后退出，不启动 GUI。
//
// Windows 发布构建使用 `windows_subsystem = "windows"`，默认无控制台。
// 检测到 CLI 子命令时尝试附加父进程控制台，使 stdout/stderr 可见。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use maobu_fetch_lib::cli::{parse_args, CliCommand};

/// Windows：尝试附加父进程控制台，使 CLI 输出在 cmd/PowerShell 中可见。
///
/// `windows_subsystem = "windows"` 的 GUI 程序默认无 stdout/stderr 句柄。
/// `AttachConsole(ATTACH_PARENT_PROCESS)` 把当前进程附加到启动它的控制台。
/// 失败时（例如从资源管理器双击启动）静默忽略，不影响 GUI 模式。
#[cfg(windows)]
fn attach_parent_console() {
    use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let command = match parse_args(args) {
        Ok(cmd) => cmd,
        Err(error) => {
            // 解析失败时也需要附加控制台，否则用户看不到错误。
            #[cfg(windows)]
            attach_parent_console();
            eprintln!("{error}");
            std::process::exit(2);
        }
    };

    match command {
        CliCommand::Run => {
            // 正常启动 GUI。
            maobu_fetch_lib::run();
        }
        other => {
            // CLI 子命令：若 GUI 正在运行，启动 run() 触发单实例插件将参数转发给运行中的首实例；
            // 若无实例运行，附加控制台后直接操作 SQLite 数据库。
            if maobu_fetch_lib::is_gui_running() {
                maobu_fetch_lib::run();
            } else {
                #[cfg(windows)]
                attach_parent_console();
                let exit_code = maobu_fetch_lib::run_cli(other);
                std::process::exit(exit_code);
            }
        }
    }
}
