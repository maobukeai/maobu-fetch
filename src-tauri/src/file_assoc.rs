//! Windows 视频文件关联管理模块（file_assoc）。
//!
//! 在 Windows 注册表中注册与查询 `MaobuFetch.Video` ProgID，
//! 支持将 .mp4 / .webm / .mkv / .mov 等视频格式注册关联到猫步下载器内置播放器。
//!
//! 设计要点（AGENTS.md §7 & §8）：
//! - 仅操作 `HKCU\Software\Classes`，无需管理员提权，安全无侵入。
//! - 遵循 Windows 10/11 现代应用规范，注册 `OpenWithProgids` 与 `shell\open\command`。
//! - 非 Windows 平台优雅回退为空操作，确保跨平台可编译。

use serde::{Deserialize, Serialize};

/// 支持关联的常见视频扩展名列表
pub const SUPPORTED_VIDEO_EXTS: &[&str] = &[
    "mp4", "webm", "mkv", "mov", "m4v", "flv", "avi", "ts", "wmv",
];

/// 文件关联状态结构体
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileAssocInfo {
    pub extension: String,
    pub is_associated: bool,
}

#[cfg(windows)]
const PROG_ID: &str = "MaobuFetch.Video";
#[cfg(windows)]
const PROG_DESCRIPTION: &str = "猫步下载器 媒体文件";

/// 获取当前所有支持扩展名的关联状态
pub fn get_file_associations() -> Result<Vec<FileAssocInfo>, String> {
    #[cfg(windows)]
    {
        use winreg::enums::*;
        use winreg::RegKey;

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let classes = match hkcu.open_subkey("Software\\Classes") {
            Ok(k) => k,
            Err(e) => return Err(format!("无法打开注册表 Classes 键: {e}")),
        };

        let mut results = Vec::with_capacity(SUPPORTED_VIDEO_EXTS.len());

        for ext in SUPPORTED_VIDEO_EXTS {
            let dot_ext = format!(".{ext}");
            let is_assoc = if let Ok(ext_key) = classes.open_subkey(&dot_ext) {
                // 检查默认 ProgID 或者 OpenWithProgids 是否包含 MaobuFetch.Video
                let default_val: Result<String, _> = ext_key.get_value("");
                if let Ok(val) = default_val {
                    if val == PROG_ID {
                        true
                    } else if let Ok(open_with) = ext_key.open_subkey("OpenWithProgids") {
                        open_with.get_value::<String, _>(PROG_ID).is_ok()
                    } else {
                        false
                    }
                } else if let Ok(open_with) = ext_key.open_subkey("OpenWithProgids") {
                    open_with.get_value::<String, _>(PROG_ID).is_ok()
                } else {
                    false
                }
            } else {
                false
            };

            results.push(FileAssocInfo {
                extension: ext.to_string(),
                is_associated: is_assoc,
            });
        }

        Ok(results)
    }

    #[cfg(not(windows))]
    {
        Ok(SUPPORTED_VIDEO_EXTS
            .iter()
            .map(|ext| FileAssocInfo {
                extension: ext.to_string(),
                is_associated: false,
            })
            .collect())
    }
}

/// 设置/更新指定扩展名的文件关联
pub fn set_file_associations(exts: Vec<String>, enable: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        use winreg::enums::*;
        use winreg::RegKey;

        let current_exe = std::env::current_exe()
            .map_err(|e| format!("无法获取当前程序路径: {e}"))?;
        let exe_path_str = current_exe.to_string_lossy();

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (classes, _) = hkcu
            .create_subkey("Software\\Classes")
            .map_err(|e| format!("无法打开注册表 Classes 键: {e}"))?;

        if enable {
            // 1. 注册 ProgID 基础信息与 open 命令
            let (prog_key, _) = classes
                .create_subkey(PROG_ID)
                .map_err(|e| format!("无法创建 ProgID 注册表项: {e}"))?;
            let _ = prog_key.set_value("", &PROG_DESCRIPTION);

            if let Ok((icon_key, _)) = prog_key.create_subkey("DefaultIcon") {
                let _ = icon_key.set_value("", &format!("\"{exe_path_str}\",0"));
            }

            if let Ok((cmd_key, _)) = prog_key.create_subkey("shell\\open\\command") {
                let command_str = format!("\"{exe_path_str}\" --play \"%1\"");
                let _ = cmd_key.set_value("", &command_str);
            }

            // 2. 为每个选中的扩展名添加关联
            for ext in &exts {
                let clean_ext = ext.trim().trim_start_matches('.');
                let dot_ext = format!(".{clean_ext}");
                if let Ok((ext_key, _)) = classes.create_subkey(&dot_ext) {
                    if let Ok((open_with, _)) = ext_key.create_subkey("OpenWithProgids") {
                        let _ = open_with.set_value(PROG_ID, &"");
                    }
                }
            }
        } else {
            // 取消关联
            for ext in &exts {
                let clean_ext = ext.trim().trim_start_matches('.');
                let dot_ext = format!(".{clean_ext}");
                if let Ok(ext_key) = classes.open_subkey_with_flags(&dot_ext, KEY_WRITE) {
                    if let Ok(open_with) = ext_key.open_subkey_with_flags("OpenWithProgids", KEY_WRITE) {
                        let _ = open_with.delete_value(PROG_ID);
                    }
                }
            }
        }

        // 通知 Windows Shell 刷新关联缓存
        notify_shell_assoc_changed();
        Ok(())
    }

    #[cfg(not(windows))]
    {
        let _ = (exts, enable);
        Ok(())
    }
}

/// 打开 Windows 系统的默认应用设置面板
pub fn open_default_apps_settings() -> Result<(), String> {
    #[cfg(windows)]
    {
        let _ = open::that("ms-settings:defaultapps");
        Ok(())
    }
    #[cfg(not(windows))]
    {
        Ok(())
    }
}

/// Windows Shell 刷新文件关联
#[cfg(windows)]
fn notify_shell_assoc_changed() {
    use windows_sys::Win32::UI::Shell::SHChangeNotify;
    const SHCNE_ASSOCCHANGED: i32 = 0x08000000;
    const SHCNF_IDLIST: u32 = 0x0000;
    unsafe {
        SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, std::ptr::null(), std::ptr::null());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_supported_exts() {
        assert!(SUPPORTED_VIDEO_EXTS.contains(&"mp4"));
        assert!(SUPPORTED_VIDEO_EXTS.contains(&"webm"));
        assert!(SUPPORTED_VIDEO_EXTS.contains(&"mkv"));
    }

    #[test]
    fn test_get_file_associations_returns_all_supported() {
        let assocs = get_file_associations().unwrap();
        assert_eq!(assocs.len(), SUPPORTED_VIDEO_EXTS.len());
        for item in assocs {
            assert!(SUPPORTED_VIDEO_EXTS.contains(&item.extension.as_str()));
        }
    }
}
