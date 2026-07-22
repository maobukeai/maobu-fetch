//! 便携版模式（Task 34）。
//!
//! 启动时检测 EXE 同目录下是否存在 `maobu.portable` 标记文件：
//! - 存在：所有数据（`lumaget.db`、`logs/`、`cookies_tmp/` 等）写入 `EXE_DIR/data/`
//! - 不存在：使用系统标准 `app_data_dir`（如 `%APPDATA%/maobu-fetch`）
//!
//! 兼容性（AGENTS.md §2）：
//! - 保留 `MAOBU_FETCH_DATA_DIR` / `LUMAGET_DATA_DIR` 环境变量覆盖能力，
//!   优先级：环境变量 > 便携标记 > app_data_dir。
//!   环境变量主要用于自动化测试和高级用户调试，便携模式是面向终端用户的快捷方式。
//! - 不修改 Tauri 标识 `app.lumaget.desktop`、数据库名 `lumaget.db` 等兼容标识。
//!
//! 路径安全（AGENTS.md §7）：
//! - 所有路径由 `current_exe()` 推导，不信任用户输入。
//! - 仅创建 `data/` 目录本身，不递归创建或删除其他路径。

use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

/// 便携模式标记文件名。位于 EXE 同目录下，存在即启用便携模式。
pub const PORTABLE_MARKER: &str = "maobu.portable";

/// 便携模式下数据目录名，拼接在 EXE 同目录下。
pub const PORTABLE_DATA_DIR_NAME: &str = "data";

/// 检测指定 EXE 路径下是否存在便携标记文件。
///
/// 纯函数版本，便于单元测试。传入的 `exe_path` 应指向可执行文件本身，
/// 函数会取其父目录后拼接 `maobu.portable`。
///
/// - `exe_path` 不存在或无法获取父目录时返回 `false`（安全回退到非便携模式）。
/// - 仅检查文件是否存在，不读取内容（标记文件可以是空文件）。
pub fn is_portable_mode_at(exe_path: &Path) -> bool {
    let Some(parent) = exe_path.parent() else {
        return false;
    };
    parent.join(PORTABLE_MARKER).exists()
}

/// 检测当前进程是否处于便携模式。
///
/// 通过 `std::env::current_exe()` 获取 EXE 路径后调用 `is_portable_mode_at`。
/// EXE 路径获取失败时返回 `false`（安全回退）。
pub fn is_portable_mode() -> bool {
    match std::env::current_exe() {
        Ok(path) => is_portable_mode_at(&path),
        Err(_) => false,
    }
}

/// 返回便携模式下的数据目录路径（`EXE_DIR/data/`）。
///
/// 纯函数版本，便于单元测试。如果 `exe_path` 无父目录或便携标记不存在，
/// 返回 `None`。
pub fn portable_data_dir_at(exe_path: &Path) -> Option<PathBuf> {
    let parent = exe_path.parent()?;
    if !is_portable_mode_at(exe_path) {
        return None;
    }
    Some(parent.join(PORTABLE_DATA_DIR_NAME))
}

/// 返回当前进程便携模式下的数据目录路径。
///
/// 便携模式未启用或 EXE 路径获取失败时返回 `None`。
pub fn portable_data_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    portable_data_dir_at(&exe)
}

/// 解析应用数据目录，按以下优先级返回：
///
/// 1. 环境变量 `MAOBU_FETCH_DATA_DIR`（兼容旧名 `LUMAGET_DATA_DIR`）
///    —— 用于自动化测试与高级用户调试，可指向任意绝对路径。
/// 2. 便携模式：EXE 同目录下 `maobu.portable` 标记存在时，返回 `EXE_DIR/data/`。
/// 3. 默认：`app.path().app_data_dir()`（Windows 下为 `%APPDATA%/<identifier>`）。
///
/// 注意：环境变量优先级高于便携标记，是为了让测试可以强制指定目录而不受
/// 便携标记干扰。普通用户不会设置环境变量，便携模式仍按预期生效。
///
/// 失败回退：所有路径解析失败时返回 `PathBuf::from(".")`，与原 `setup` 行为一致，
/// 避免启动中断。
pub fn resolve_data_dir(app: &AppHandle) -> PathBuf {
    if let Some(env_dir) = std::env::var_os("MAOBU_FETCH_DATA_DIR")
        .or_else(|| std::env::var_os("LUMAGET_DATA_DIR"))
        .map(PathBuf::from)
    {
        return env_dir;
    }
    if let Some(portable) = portable_data_dir() {
        return portable;
    }
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
}

/// 返回当前是否处于便携模式，供前端通过 `app_get_info` 命令读取。
///
/// 与 `is_portable_mode()` 一致，但语义上明确"已被环境变量覆盖"的情况：
/// 如果设置了 `MAOBU_FETCH_DATA_DIR`，即使存在便携标记，也不视为便携模式生效
/// （因为实际数据目录已被环境变量接管）。
pub fn is_portable_mode_effective() -> bool {
    if std::env::var_os("MAOBU_FETCH_DATA_DIR").is_some()
        || std::env::var_os("LUMAGET_DATA_DIR").is_some()
    {
        return false;
    }
    is_portable_mode()
}

/// 返回当前生效的数据目录，与 `resolve_data_dir` 一致但不要求 `AppHandle`。
///
/// 用于无法访问 `AppHandle` 的场景（如部分测试）。生产代码应使用 `resolve_data_dir`。
pub fn current_data_dir() -> Option<PathBuf> {
    if let Some(env_dir) = std::env::var_os("MAOBU_FETCH_DATA_DIR")
        .or_else(|| std::env::var_os("LUMAGET_DATA_DIR"))
        .map(PathBuf::from)
    {
        return Some(env_dir);
    }
    portable_data_dir()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    /// 辅助：在 tempdir 中模拟一个 EXE 文件路径。
    /// 返回 `(tempdir, exe_path)`，调用方需保留 tempdir 以延长生命周期。
    fn make_fake_exe_path() -> (tempfile::TempDir, PathBuf) {
        let dir = tempdir().expect("无法创建 tempdir");
        let exe_path = dir.path().join("maobu-fetch.exe");
        fs::write(&exe_path, b"fake exe").expect("无法写入模拟 EXE");
        (dir, exe_path)
    }

    #[test]
    fn is_portable_mode_with_marker_file_returns_true() {
        let (dir, exe_path) = make_fake_exe_path();
        let marker = dir.path().join(PORTABLE_MARKER);
        fs::write(&marker, b"").expect("无法写入标记文件");

        assert!(is_portable_mode_at(&exe_path));
    }

    #[test]
    fn is_portable_mode_without_marker_file_returns_false() {
        let (_dir, exe_path) = make_fake_exe_path();
        // 不创建 maobu.portable 文件
        assert!(!is_portable_mode_at(&exe_path));
    }

    #[test]
    fn is_portable_mode_returns_false_when_exe_has_no_parent() {
        // 仅文件名，无父目录
        let exe_path = Path::new("maobu-fetch.exe");
        assert!(!is_portable_mode_at(exe_path));
    }

    #[test]
    fn is_portable_mode_returns_false_for_nonexistent_exe() {
        // 父目录存在但 EXE 自身不存在 —— 标记文件检查不依赖 EXE 是否存在，
        // 只看父目录下是否有 maobu.portable。
        let dir = tempdir().expect("无法创建 tempdir");
        let exe_path = dir.path().join("nonexistent.exe");
        // 不创建 EXE，但创建标记文件
        fs::write(dir.path().join(PORTABLE_MARKER), b"").expect("无法写入标记文件");

        // 函数应当返回 true，因为它只检查父目录下是否存在标记文件
        assert!(is_portable_mode_at(&exe_path));
    }

    #[test]
    fn portable_data_dir_returns_exe_sibling_data() {
        let (dir, exe_path) = make_fake_exe_path();
        fs::write(dir.path().join(PORTABLE_MARKER), b"").expect("无法写入标记文件");

        let data_dir = portable_data_dir_at(&exe_path).expect("应返回 Some(data/)");
        assert_eq!(data_dir, dir.path().join(PORTABLE_DATA_DIR_NAME));
    }

    #[test]
    fn portable_data_dir_returns_none_without_marker() {
        let (_dir, exe_path) = make_fake_exe_path();
        // 不创建标记文件
        assert_eq!(portable_data_dir_at(&exe_path), None);
    }

    #[test]
    fn portable_data_dir_returns_none_when_no_parent() {
        let exe_path = Path::new("maobu-fetch.exe");
        assert_eq!(portable_data_dir_at(exe_path), None);
    }

    /// 验证便携目录路径在 EXE 直接位于 tempdir 根目录时的正确性。
    /// 模拟真实场景：用户把 EXE 放在桌面/下载文件夹，旁边放一个 maobu.portable 文件。
    #[test]
    fn portable_data_dir_path_is_sibling_of_exe() {
        let (dir, exe_path) = make_fake_exe_path();
        let marker_path = dir.path().join(PORTABLE_MARKER);
        fs::write(&marker_path, b"").expect("无法写入标记文件");

        let data_dir = portable_data_dir_at(&exe_path).expect("应返回 Some");
        let exe_parent = exe_path.parent().expect("EXE 应有父目录");

        // data_dir 的父目录应与 EXE 父目录相同
        assert_eq!(data_dir.parent().expect("data_dir 应有父目录"), exe_parent);
        // data_dir 末尾组件应为 "data"
        assert_eq!(
            data_dir.file_name().and_then(|n| n.to_str()),
            Some(PORTABLE_DATA_DIR_NAME)
        );
    }

    /// 验证标记文件名正确性。
    #[test]
    fn marker_file_name_is_maobu_portable() {
        assert_eq!(PORTABLE_MARKER, "maobu.portable");
    }

    /// 验证数据目录名正确性。
    #[test]
    fn data_dir_name_is_data() {
        assert_eq!(PORTABLE_DATA_DIR_NAME, "data");
    }

    /// 综合测试：模拟便携模式启用 → 数据目录解析 → 目录创建的完整流程。
    /// 这是 SubTask 34.4"便携模式数据隔离"的单元测试版本。
    #[test]
    fn portable_mode_end_to_end_data_isolation() {
        let (dir, exe_path) = make_fake_exe_path();
        let marker_path = dir.path().join(PORTABLE_MARKER);
        fs::write(&marker_path, b"portable mode").expect("无法写入标记文件");

        // 1. 检测便携模式
        assert!(is_portable_mode_at(&exe_path));

        // 2. 解析数据目录
        let data_dir = portable_data_dir_at(&exe_path).expect("应返回便携目录");
        let expected_data_dir = dir.path().join(PORTABLE_DATA_DIR_NAME);
        assert_eq!(data_dir, expected_data_dir);

        // 3. 创建数据目录（模拟 setup 中的行为）
        fs::create_dir_all(&data_dir).expect("无法创建 data 目录");

        // 4. 模拟写入 lumaget.db 文件
        let db_path = data_dir.join("lumaget.db");
        fs::write(&db_path, b"fake db content").expect("无法写入 db");
        assert!(db_path.exists(), "数据库文件应存在于便携目录");

        // 5. 模拟 logs 子目录
        let logs_dir = data_dir.join("logs");
        fs::create_dir_all(&logs_dir).expect("无法创建 logs 子目录");
        assert!(logs_dir.exists());

        // 6. 验证隔离：所有数据都在 EXE 同目录的 data/ 下，不污染其他位置
        assert!(expected_data_dir.exists());
        assert!(db_path.starts_with(&expected_data_dir));
        assert!(logs_dir.starts_with(&expected_data_dir));
    }

    /// 测试不启用便携模式时数据目录的隔离性：便携目录不被创建。
    #[test]
    fn non_portable_mode_does_not_create_portable_data_dir() {
        let (dir, exe_path) = make_fake_exe_path();
        // 不创建 maobu.portable 标记

        assert!(!is_portable_mode_at(&exe_path));
        assert_eq!(portable_data_dir_at(&exe_path), None);

        // 验证 EXE 同目录下没有 data/ 文件夹
        let potential_data_dir = dir.path().join(PORTABLE_DATA_DIR_NAME);
        assert!(!potential_data_dir.exists());
    }

    /// 验证 `is_portable_mode` 与 `portable_data_dir` 在真实 EXE 环境下不 panic。
    /// 这两个函数依赖 `std::env::current_exe()`，在测试环境中应当返回
    /// 当前测试进程的 EXE 路径（通常是 cargo test 进程）。
    #[test]
    fn current_exe_based_functions_do_not_panic() {
        // 仅验证不 panic，不验证返回值（取决于运行测试的进程是否在便携目录下）
        let _ = is_portable_mode();
        let _ = portable_data_dir();
        let _ = current_data_dir();
        let _ = is_portable_mode_effective();
    }
}
