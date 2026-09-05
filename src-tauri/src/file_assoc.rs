//! Windows 媒体与图片文件关联管理模块（file_assoc）。
//!
//! 在 Windows 注册表中注册与查询 `MaobuFetch.Video` 与 `MaobuFetch.Image` ProgID，
//! 支持将视频（.mp4 / .webm / .mkv 等）与图片（.png / .jpg / .webp 等）注册关联到猫步下载器内置播放器/看图器。
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

/// 支持关联的常见图片扩展名列表
pub const SUPPORTED_IMAGE_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "webp", "gif", "bmp", "svg", "ico", "avif",
];

fn default_assoc_category() -> String {
    "video".into()
}

/// 文件关联状态结构体
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileAssocInfo {
    pub extension: String,
    pub is_associated: bool,
    #[serde(default = "default_assoc_category")]
    pub category: String,
}

#[cfg(windows)]
const VIDEO_PROG_ID: &str = "MaobuFetch.Video";
#[cfg(windows)]
const VIDEO_PROG_DESCRIPTION: &str = "猫步播放器";

#[cfg(windows)]
const IMAGE_PROG_ID: &str = "MaobuFetch.Image";
#[cfg(windows)]
const IMAGE_PROG_DESCRIPTION: &str = "猫步看图器";

/// 获取当前所有支持扩展名（视频 + 图片）的关联状态
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

        let total_len = SUPPORTED_VIDEO_EXTS.len() + SUPPORTED_IMAGE_EXTS.len();
        let mut results = Vec::with_capacity(total_len);

        // 1. 检查视频扩展名
        for ext in SUPPORTED_VIDEO_EXTS {
            let dot_ext = format!(".{ext}");
            let is_assoc = if let Ok(ext_key) = classes.open_subkey(&dot_ext) {
                let default_val: Result<String, _> = ext_key.get_value("");
                if let Ok(val) = default_val {
                    if val == VIDEO_PROG_ID {
                        true
                    } else if let Ok(open_with) = ext_key.open_subkey("OpenWithProgids") {
                        open_with.get_value::<String, _>(VIDEO_PROG_ID).is_ok()
                    } else {
                        false
                    }
                } else if let Ok(open_with) = ext_key.open_subkey("OpenWithProgids") {
                    open_with.get_value::<String, _>(VIDEO_PROG_ID).is_ok()
                } else {
                    false
                }
            } else {
                false
            };

            results.push(FileAssocInfo {
                extension: ext.to_string(),
                is_associated: is_assoc,
                category: "video".into(),
            });
        }

        // 2. 检查图片扩展名
        for ext in SUPPORTED_IMAGE_EXTS {
            let dot_ext = format!(".{ext}");
            let is_assoc = if let Ok(ext_key) = classes.open_subkey(&dot_ext) {
                let default_val: Result<String, _> = ext_key.get_value("");
                if let Ok(val) = default_val {
                    if val == IMAGE_PROG_ID {
                        true
                    } else if let Ok(open_with) = ext_key.open_subkey("OpenWithProgids") {
                        open_with.get_value::<String, _>(IMAGE_PROG_ID).is_ok()
                    } else {
                        false
                    }
                } else if let Ok(open_with) = ext_key.open_subkey("OpenWithProgids") {
                    open_with.get_value::<String, _>(IMAGE_PROG_ID).is_ok()
                } else {
                    false
                }
            } else {
                false
            };

            results.push(FileAssocInfo {
                extension: ext.to_string(),
                is_associated: is_assoc,
                category: "image".into(),
            });
        }

        Ok(results)
    }

    #[cfg(not(windows))]
    {
        let mut results = Vec::new();
        for ext in SUPPORTED_VIDEO_EXTS {
            results.push(FileAssocInfo {
                extension: ext.to_string(),
                is_associated: false,
                category: "video".into(),
            });
        }
        for ext in SUPPORTED_IMAGE_EXTS {
            results.push(FileAssocInfo {
                extension: ext.to_string(),
                is_associated: false,
                category: "image".into(),
            });
        }
        Ok(results)
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
            // 1. 注册 Applications\maobu-fetch.exe 及其支持格式（Windows "建议的应用" 关键识别来源）
            if let Ok((app_key, _)) = classes.create_subkey("Applications\\maobu-fetch.exe") {
                let _ = app_key.set_value("FriendlyAppName", &"猫步下载器");
                let _ = app_key.set_value("ApplicationCompany", &"Maobu Fetch");
                if let Ok((app_icon, _)) = app_key.create_subkey("DefaultIcon") {
                    let _ = app_icon.set_value("", &format!("\"{exe_path_str}\",0"));
                }
                if let Ok((shell_cmd, _)) = app_key.create_subkey("shell\\open\\command") {
                    let _ = shell_cmd.set_value("", &format!("\"{exe_path_str}\" \"%1\""));
                }
                if let Ok((supp_types, _)) = app_key.create_subkey("SupportedTypes") {
                    for ext in &exts {
                        let clean_ext = ext.trim().trim_start_matches('.');
                        let dot_ext = format!(".{clean_ext}");
                        let _ = supp_types.set_value(&dot_ext, &"");
                    }
                }
            }

            // 2. 注册 Video ProgID 基础信息与 open 命令
            let has_video = exts.iter().any(|e| {
                let c = e.trim().trim_start_matches('.');
                SUPPORTED_VIDEO_EXTS.contains(&c)
            });
            if has_video {
                let (prog_key, _) = classes
                    .create_subkey(VIDEO_PROG_ID)
                    .map_err(|e| format!("无法创建 Video ProgID 注册表项: {e}"))?;
                let _ = prog_key.set_value("", &VIDEO_PROG_DESCRIPTION);
                let _ = prog_key.set_value("FriendlyTypeName", &"猫步播放器 媒体文件");
                let _ = prog_key.set_value("AppUserModelId", &"app.lumaget.desktop");

                if let Ok((icon_key, _)) = prog_key.create_subkey("DefaultIcon") {
                    let _ = icon_key.set_value("", &format!("\"{exe_path_str}\",0"));
                }

                if let Ok((cmd_key, _)) = prog_key.create_subkey("shell\\open\\command") {
                    let command_str = format!("\"{exe_path_str}\" --play \"%1\"");
                    let _ = cmd_key.set_value("", &command_str);
                }
            }

            // 3. 注册 Image ProgID 基础信息与 open 命令
            let has_image = exts.iter().any(|e| {
                let c = e.trim().trim_start_matches('.');
                SUPPORTED_IMAGE_EXTS.contains(&c)
            });
            if has_image {
                let (prog_key, _) = classes
                    .create_subkey(IMAGE_PROG_ID)
                    .map_err(|e| format!("无法创建 Image ProgID 注册表项: {e}"))?;
                let _ = prog_key.set_value("", &IMAGE_PROG_DESCRIPTION);
                let _ = prog_key.set_value("FriendlyTypeName", &"猫步看图器 图像文件");
                let _ = prog_key.set_value("AppUserModelId", &"app.lumaget.desktop");

                if let Ok((icon_key, _)) = prog_key.create_subkey("DefaultIcon") {
                    let _ = icon_key.set_value("", &format!("\"{exe_path_str}\",0"));
                }

                if let Ok((cmd_key, _)) = prog_key.create_subkey("shell\\open\\command") {
                    let command_str = format!("\"{exe_path_str}\" --view-image \"%1\"");
                    let _ = cmd_key.set_value("", &command_str);
                }
            }

            // 4. 为每个选中的扩展名添加关联到对应 ProgID 并加入 OpenWithProgids & OpenWithList
            for ext in &exts {
                let clean_ext = ext.trim().trim_start_matches('.');
                let dot_ext = format!(".{clean_ext}");
                let target_prog_id = if SUPPORTED_IMAGE_EXTS.contains(&clean_ext) {
                    IMAGE_PROG_ID
                } else {
                    VIDEO_PROG_ID
                };

                if let Ok((ext_key, _)) = classes.create_subkey(&dot_ext) {
                    if let Ok((open_with, _)) = ext_key.create_subkey("OpenWithProgids") {
                        let _ = open_with.set_value(target_prog_id, &"");
                    }
                    let _ = ext_key.create_subkey("OpenWithList\\maobu-fetch.exe");
                }

                // 同步写入 Explorer 缓存以加速 Windows 建议应用识别
                if let Ok(explorer_exts) = hkcu.open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\FileExts") {
                    if let Ok((fext_key, _)) = explorer_exts.create_subkey(&dot_ext) {
                        if let Ok((f_ow, _)) = fext_key.create_subkey("OpenWithProgids") {
                            let _ = f_ow.set_value(target_prog_id, &"");
                        }
                    }
                }
            }

            // 5. 注册到 Windows 默认应用能力中心（RegisteredApplications & Capabilities）
            if let Ok((reg_apps, _)) = hkcu.create_subkey("Software\\RegisteredApplications") {
                let _ = reg_apps.set_value("MaobuFetch", &"Software\\MaobuFetch\\Capabilities");
            }
            if let Ok((caps, _)) = hkcu.create_subkey("Software\\MaobuFetch\\Capabilities") {
                let _ = caps.set_value("ApplicationName", &"猫步下载器");
                let _ = caps.set_value("ApplicationDescription", &"猫步下载器与极速媒体看图器");
                let _ = caps.set_value("ApplicationIcon", &format!("\"{exe_path_str}\",0"));
                if let Ok((assoc_key, _)) = caps.create_subkey("FileAssociations") {
                    for ext in &exts {
                        let clean_ext = ext.trim().trim_start_matches('.');
                        let dot_ext = format!(".{clean_ext}");
                        let target_prog_id = if SUPPORTED_IMAGE_EXTS.contains(&clean_ext) {
                            IMAGE_PROG_ID
                        } else {
                            VIDEO_PROG_ID
                        };
                        let _ = assoc_key.set_value(&dot_ext, &target_prog_id);
                    }
                }
            }
        } else {
            // 取消关联
            for ext in &exts {
                let clean_ext = ext.trim().trim_start_matches('.');
                let dot_ext = format!(".{clean_ext}");
                let target_prog_id = if SUPPORTED_IMAGE_EXTS.contains(&clean_ext) {
                    IMAGE_PROG_ID
                } else {
                    VIDEO_PROG_ID
                };

                if let Ok(ext_key) = classes.open_subkey_with_flags(&dot_ext, KEY_WRITE) {
                    if let Ok(open_with) = ext_key.open_subkey_with_flags("OpenWithProgids", KEY_WRITE) {
                        let _ = open_with.delete_value(target_prog_id);
                    }
                }

                if let Ok(explorer_exts) = hkcu.open_subkey_with_flags("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\FileExts", KEY_WRITE) {
                    if let Ok(fext_key) = explorer_exts.open_subkey_with_flags(&dot_ext, KEY_WRITE) {
                        if let Ok(f_ow) = fext_key.open_subkey_with_flags("OpenWithProgids", KEY_WRITE) {
                            let _ = f_ow.delete_value(target_prog_id);
                        }
                    }
                }

                if let Ok(caps_assoc) = hkcu.open_subkey_with_flags("Software\\MaobuFetch\\Capabilities\\FileAssociations", KEY_WRITE) {
                    let _ = caps_assoc.delete_value(&dot_ext);
                }

                if let Ok(supp_types) = classes.open_subkey_with_flags("Applications\\maobu-fetch.exe\\SupportedTypes", KEY_WRITE) {
                    let _ = supp_types.delete_value(&dot_ext);
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

/// 确保应用基础能力（Applications 与 RegisteredApplications）已在注册表登记
pub fn ensure_registered_applications() -> Result<(), String> {
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

        if let Ok((app_key, _)) = classes.create_subkey("Applications\\maobu-fetch.exe") {
            let _ = app_key.set_value("FriendlyAppName", &"猫步下载器");
            let _ = app_key.set_value("ApplicationCompany", &"Maobu Fetch");
            if let Ok((app_icon, _)) = app_key.create_subkey("DefaultIcon") {
                let _ = app_icon.set_value("", &format!("\"{exe_path_str}\",0"));
            }
            if let Ok((shell_cmd, _)) = app_key.create_subkey("shell\\open\\command") {
                let _ = shell_cmd.set_value("", &format!("\"{exe_path_str}\" \"%1\""));
            }
            if let Ok((supp_types, _)) = app_key.create_subkey("SupportedTypes") {
                for ext in SUPPORTED_IMAGE_EXTS.iter().chain(SUPPORTED_VIDEO_EXTS.iter()) {
                    let _ = supp_types.set_value(&format!(".{ext}"), &"");
                }
            }
        }

        if let Ok((reg_apps, _)) = hkcu.create_subkey("Software\\RegisteredApplications") {
            let _ = reg_apps.set_value("MaobuFetch", &"Software\\MaobuFetch\\Capabilities");
        }
        if let Ok((caps, _)) = hkcu.create_subkey("Software\\MaobuFetch\\Capabilities") {
            let _ = caps.set_value("ApplicationName", &"猫步下载器");
            let _ = caps.set_value("ApplicationDescription", &"猫步下载器与极速媒体看图器");
            let _ = caps.set_value("ApplicationIcon", &format!("\"{exe_path_str}\",0"));
        }

        Ok(())
    }
    #[cfg(not(windows))]
    {
        Ok(())
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
        assert!(SUPPORTED_IMAGE_EXTS.contains(&"png"));
        assert!(SUPPORTED_IMAGE_EXTS.contains(&"jpg"));
        assert!(SUPPORTED_IMAGE_EXTS.contains(&"webp"));
    }

    #[test]
    fn test_get_file_associations_returns_all_supported() {
        let assocs = get_file_associations().unwrap();
        let expected_len = SUPPORTED_VIDEO_EXTS.len() + SUPPORTED_IMAGE_EXTS.len();
        assert_eq!(assocs.len(), expected_len);
        for item in assocs {
            if item.category == "image" {
                assert!(SUPPORTED_IMAGE_EXTS.contains(&item.extension.as_str()));
            } else {
                assert!(SUPPORTED_VIDEO_EXTS.contains(&item.extension.as_str()));
            }
        }
    }
}
