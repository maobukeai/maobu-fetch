//! 系统目录探测模块（跨平台支持 Win11 / Win10 / macOS / Linux）。
//!
//! 核心目标：
//! 准确获取用户在操作系统中真正配置的“下载”目录（例如用户将下载目录重定向到 D:\Downloads）。
//! 严禁无脑硬编码 USERPROFILE\Downloads，防止强行在 C 盘创建下载目录。

use std::path::{Path, PathBuf};

#[cfg(windows)]
mod win_impl {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::path::PathBuf;
    use winreg::enums::*;
    use winreg::RegKey;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    // FOLDERID_Downloads: {374DE290-123F-4565-9164-39C4925E467B}
    const FOLDERID_DOWNLOADS: Guid = Guid {
        data1: 0x374de290,
        data2: 0x123f,
        data3: 0x4565,
        data4: [0x91, 0x64, 0x39, 0xc4, 0x92, 0x5e, 0x46, 0x7b],
    };

    extern "system" {
        fn SHGetKnownFolderPath(
            rfid: *const Guid,
            dwFlags: u32,
            hToken: *mut std::ffi::c_void,
            ppszPath: *mut *mut u16,
        ) -> i32;
        fn CoTaskMemFree(pv: *mut std::ffi::c_void);
        fn ExpandEnvironmentStringsW(
            lpSrc: *const u16,
            lpDst: *mut u16,
            nSize: u32,
        ) -> u32;
    }

    /// 通过 Win32 SHGetKnownFolderPath API 获取系统 KnownFolder Downloads。
    /// 无论用户是将下载目录移动到了 D:\、E:\ 还是网络驱动器，此 API 都能权威返回。
    pub fn get_known_folder_downloads() -> Option<PathBuf> {
        unsafe {
            let mut path_ptr: *mut u16 = std::ptr::null_mut();
            let hr = SHGetKnownFolderPath(
                &FOLDERID_DOWNLOADS,
                0,
                std::ptr::null_mut(),
                &mut path_ptr,
            );
            if hr == 0 && !path_ptr.is_null() {
                let mut len = 0;
                while *path_ptr.add(len) != 0 {
                    len += 1;
                }
                let slice = std::slice::from_raw_parts(path_ptr, len);
                let os_str = OsString::from_wide(slice);
                CoTaskMemFree(path_ptr as *mut _);
                let path = PathBuf::from(os_str);
                if !path.as_os_str().is_empty() {
                    return Some(path);
                }
            }
        }
        None
    }

    /// 展开字符串中的环境变量（如 %USERPROFILE% 或 %HOMEDRIVE%）。
    fn expand_env_string(raw: &str) -> String {
        if !raw.contains('%') {
            return raw.to_string();
        }
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = std::ffi::OsStr::new(raw)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        unsafe {
            let needed = ExpandEnvironmentStringsW(wide.as_ptr(), std::ptr::null_mut(), 0);
            if needed > 0 {
                let mut buf = vec![0u16; needed as usize];
                let written = ExpandEnvironmentStringsW(wide.as_ptr(), buf.as_mut_ptr(), needed);
                if written > 0 {
                    // 去除末尾 null
                    let len = if buf.ends_with(&[0]) { buf.len() - 1 } else { buf.len() };
                    return OsString::from_wide(&buf[..len]).to_string_lossy().to_string();
                }
            }
        }
        raw.to_string()
    }

    /// 从 Windows 注册表读取用户自定义的下载目录。
    pub fn get_registry_downloads() -> Option<PathBuf> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        // 1. 优先读取 User Shell Folders（存储重定向/移动后的实际路径）
        if let Ok(key) = hkcu.open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\User Shell Folders") {
            // FOLDERID_Downloads GUID
            if let Ok(val) = key.get_value::<String, _>("{374DE290-123F-4565-9164-39C4925E467B}") {
                let expanded = expand_env_string(&val);
                if !expanded.trim().is_empty() {
                    return Some(PathBuf::from(expanded.trim()));
                }
            }
            // Win10/Win11 备用 Shell GUID
            if let Ok(val) = key.get_value::<String, _>("{7D83EE9B-2244-4E70-B1F5-5393042AF1E4}") {
                let expanded = expand_env_string(&val);
                if !expanded.trim().is_empty() {
                    return Some(PathBuf::from(expanded.trim()));
                }
            }
            // Downloads 名称键
            if let Ok(val) = key.get_value::<String, _>("Downloads") {
                let expanded = expand_env_string(&val);
                if !expanded.trim().is_empty() {
                    return Some(PathBuf::from(expanded.trim()));
                }
            }
        }

        // 2. 备用读取 Shell Folders
        if let Ok(key) = hkcu.open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Shell Folders") {
            if let Ok(val) = key.get_value::<String, _>("{374DE290-123F-4565-9164-39C4925E467B}") {
                let expanded = expand_env_string(&val);
                if !expanded.trim().is_empty() {
                    return Some(PathBuf::from(expanded.trim()));
                }
            }
            if let Ok(val) = key.get_value::<String, _>("Downloads") {
                let expanded = expand_env_string(&val);
                if !expanded.trim().is_empty() {
                    return Some(PathBuf::from(expanded.trim()));
                }
            }
        }

        None
    }
}

/// 获取当前操作系统真正配置的下载目录。
///
/// 具备多级回退保证：
/// - Windows: SHGetKnownFolderPath (权威) -> Registry User Shell Folders -> dirs -> USERPROFILE/Downloads
/// - macOS / Linux: dirs -> XDG_DOWNLOAD_DIR / user-dirs.dirs -> HOME/Downloads
pub fn system_download_dir() -> PathBuf {
    #[cfg(windows)]
    {
        // 1. 尝试 Win32 Shell API: SHGetKnownFolderPath
        if let Some(path) = win_impl::get_known_folder_downloads() {
            if !path.as_os_str().is_empty() {
                return path;
            }
        }

        // 2. 尝试 Windows 注册表 User Shell Folders
        if let Some(path) = win_impl::get_registry_downloads() {
            if !path.as_os_str().is_empty() {
                return path;
            }
        }

        // 3. 回退到 USERPROFILE\Downloads
        if let Some(profile) = std::env::var_os("USERPROFILE") {
            return PathBuf::from(profile).join("Downloads");
        }
    }

    #[cfg(not(windows))]
    {
        // 检查 XDG 环境变量或 user-dirs.dirs
        if let Ok(val) = std::env::var("XDG_DOWNLOAD_DIR") {
            if !val.trim().is_empty() {
                return PathBuf::from(val.trim());
            }
        }
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            let xdg_file = home.join(".config").join("user-dirs.dirs");
            if let Ok(content) = std::fs::read_to_string(&xdg_file) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with("XDG_DOWNLOAD_DIR=") {
                        let raw = trimmed.trim_start_matches("XDG_DOWNLOAD_DIR=").trim_matches('"');
                        let expanded = raw.replace("$HOME", &home.to_string_lossy());
                        return PathBuf::from(expanded);
                    }
                }
            }
            return home.join("Downloads");
        }
    }

    PathBuf::from("Downloads")
}

/// 旧版硬编码的默认下载目录（基于 USERPROFILE / HOME 直接拼接 Downloads）。
pub fn legacy_hardcoded_userprofile_downloads() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("Downloads"))
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Downloads"))
    }
}

/// 路径规范化比较（Windows 下不区分大小写，去除尾部反斜杠/斜杠）。
pub fn paths_equal(a: &Path, b: &Path) -> bool {
    let clean_a = a.to_string_lossy();
    let clean_b = b.to_string_lossy();
    let norm_a = clean_a.trim_end_matches(['\\', '/']).replace('/', "\\");
    let norm_b = clean_b.trim_end_matches(['\\', '/']).replace('/', "\\");

    #[cfg(windows)]
    {
        norm_a.eq_ignore_ascii_case(&norm_b)
    }
    #[cfg(not(windows))]
    {
        norm_a == norm_b
    }
}

/// 判断保存的下载目录是否需要自动迁移到系统真实的下载目录。
///
/// 判定逻辑：
/// 1. 若当前设置为空或纯空白字符：需要迁移。
/// 2. 若当前设置与旧版硬编码的 `USERPROFILE\Downloads` 一致，但系统真实的下载目录不同（如用户重定向到了 `D:\Downloads`）：
///    自动平滑迁移为系统真实的下载目录。
/// 3. 若用户是手动设置的自定义目录（既不是旧版硬编码，也不为空）：尊重用户设置，不触发自动迁移。
pub fn migrate_download_dir_if_needed(current_saved_dir: &str) -> Option<String> {
    let trimmed = current_saved_dir.trim();
    let sys_dir = system_download_dir();

    if trimmed.is_empty() {
        return Some(sys_dir.to_string_lossy().to_string());
    }

    let saved_path = PathBuf::from(trimmed);

    if let Some(legacy_default) = legacy_hardcoded_userprofile_downloads() {
        // 如果保存的路径与旧版硬编码的默认路径一致，但与系统真实路径不一致
        if paths_equal(&saved_path, &legacy_default) && !paths_equal(&legacy_default, &sys_dir) {
            return Some(sys_dir.to_string_lossy().to_string());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_download_dir_returns_non_empty() {
        let dir = system_download_dir();
        assert!(!dir.as_os_str().is_empty());
        println!("Detected system download dir: {}", dir.display());
    }

    #[test]
    fn test_paths_equal() {
        assert!(paths_equal(Path::new("C:\\Users\\Test\\Downloads"), Path::new("c:/users/test/downloads/")));
        assert!(!paths_equal(Path::new("C:\\Users\\Test\\Downloads"), Path::new("D:\\Downloads")));
    }

    #[test]
    fn test_migrate_logic() {
        if let Some(legacy) = legacy_hardcoded_userprofile_downloads() {
            let sys = system_download_dir();
            if !paths_equal(&legacy, &sys) {
                // 如果当前系统下载目录与硬编码不同（如用户是 D:\Downloads），测试必须成功判定需要迁移
                let result = migrate_download_dir_if_needed(&legacy.to_string_lossy());
                assert_eq!(result, Some(sys.to_string_lossy().to_string()));
            }
        }

        // 手动自定义的目录绝对不能被篡改
        assert_eq!(migrate_download_dir_if_needed("E:\\MyCustomFolder"), None);
        assert_eq!(migrate_download_dir_if_needed("D:\\SpecialDownload"), None);
    }
}
