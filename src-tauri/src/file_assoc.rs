//! Windows 媒体与图片文件关联管理模块（file_assoc）。
//!
//! 在 Windows 注册表中注册与查询 `MaobuFetch.Video` 与 `MaobuFetch.Image` ProgID，
//! 支持将视频（.mp4 / .webm / .mkv 等）与图片（.png / .jpg / .webp 等）注册关联到猫步下载器内置播放器/看图器。
//!
//! 设计要点（AGENTS.md §7 & §8）：
//! - 仅登记当前用户的候选应用能力，不修改默认应用或 UserChoice。
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

#[cfg(all(windows, test))]
mod permission_fixture;
#[cfg(windows)]
mod registry;
#[cfg(all(windows, test))]
mod registry_tests;

fn is_image(extension: &str) -> bool {
    SUPPORTED_IMAGE_EXTS.contains(&extension)
}

fn normalize_extensions(extensions: &[String]) -> Result<Vec<String>, String> {
    let mut result = Vec::new();
    for extension in extensions {
        let extension = extension
            .trim()
            .trim_start_matches('.')
            .to_ascii_lowercase();
        if !SUPPORTED_VIDEO_EXTS.contains(&extension.as_str()) && !is_image(&extension) {
            return Err("包含不支持的文件扩展名，未修改文件关联".into());
        }
        if !result.contains(&extension) {
            result.push(extension);
        }
    }
    Ok(result)
}

fn unassociated() -> Vec<FileAssocInfo> {
    SUPPORTED_VIDEO_EXTS
        .iter()
        .chain(SUPPORTED_IMAGE_EXTS)
        .map(|extension| FileAssocInfo {
            extension: extension.to_string(),
            is_associated: false,
            category: if is_image(extension) {
                "image"
            } else {
                "video"
            }
            .into(),
        })
        .collect()
}

/// 查询候选应用注册；隔离验证不读取真实用户注册表。
pub fn get_file_associations() -> Result<Vec<FileAssocInfo>, String> {
    if !crate::system_integration::allowed() {
        return Ok(unassociated());
    }
    #[cfg(windows)]
    {
        registry::list(&winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER))
            .map_err(|e| format!("读取文件关联失败，请检查当前用户注册表权限：{e}"))
    }
    #[cfg(not(windows))]
    {
        Ok(unassociated())
    }
}

/// 注册/撤销候选应用，不写扩展名默认值或 Windows UserChoice。
pub fn set_file_associations(exts: Vec<String>, enable: bool) -> Result<(), String> {
    if !crate::system_integration::allowed() {
        return Err("当前为隔离验证模式，未修改真实用户的文件关联".into());
    }
    #[cfg(windows)]
    {
        let exe = std::env::current_exe().map_err(|e| format!("无法获取当前程序路径：{e}"))?;
        registry::set(
            &winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER),
            &exe,
            &exts,
            enable,
        )
        .map_err(|e| format!("更新文件关联失败，请检查当前用户注册表权限后重试：{e}"))?;
        notify_shell_assoc_changed();
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (normalize_extensions(&exts)?, enable);
        Ok(())
    }
}

pub fn open_default_apps_settings() -> Result<(), String> {
    if !crate::system_integration::allowed() {
        return Err("当前为隔离验证模式，未打开真实用户的默认应用设置".into());
    }
    #[cfg(windows)]
    {
        open::that("ms-settings:defaultapps")
            .map_err(|e| format!("无法打开 Windows 默认应用设置：{e}"))
    }
    #[cfg(not(windows))]
    {
        Ok(())
    }
}

pub fn ensure_registered_applications() -> Result<(), String> {
    if !crate::system_integration::allowed() {
        return Ok(());
    }
    #[cfg(windows)]
    {
        let exe = std::env::current_exe().map_err(|e| format!("无法获取当前程序路径：{e}"))?;
        registry::ensure(
            &winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER),
            &exe,
        )
        .map_err(|e| format!("登记应用能力失败，请检查当前用户注册表权限：{e}"))
    }
    #[cfg(not(windows))]
    {
        Ok(())
    }
}

#[cfg(windows)]
fn notify_shell_assoc_changed() {
    use windows_sys::Win32::UI::Shell::SHChangeNotify;
    const SHCNE_ASSOCCHANGED: i32 = 0x08000000;
    unsafe {
        SHChangeNotify(SHCNE_ASSOCCHANGED, 0, std::ptr::null(), std::ptr::null());
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
        let list = get_file_associations().unwrap();
        assert_eq!(
            list.len(),
            SUPPORTED_VIDEO_EXTS.len() + SUPPORTED_IMAGE_EXTS.len()
        );
        assert!(list.iter().all(|entry| !entry.is_associated));
    }
    #[test]
    fn isolated_entry_points_never_register_or_open_global_settings() {
        assert!(ensure_registered_applications().is_ok());
        assert!(set_file_associations(vec!["mp4".into()], true)
            .unwrap_err()
            .contains("隔离"));
        assert!(open_default_apps_settings().unwrap_err().contains("隔离"));
    }
}
