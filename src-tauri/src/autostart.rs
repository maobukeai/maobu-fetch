//! 开机自启模块（Windows 注册表同步）
//! 遵循 AGENTS.md 强约束：使用保留标识 `app.lumaget.desktop` 作为注册表键名。

#[cfg(windows)]
pub fn sync_autostart(enabled: bool) -> Result<(), String> {
    use winreg::enums::*;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let run_path = r"Software\Microsoft\Windows\CurrentVersion\Run";
    let (key, _) = hkcu
        .create_subkey(run_path)
        .map_err(|e| format!("打开注册表 Run 键失败: {e}"))?;

    let app_name = "app.lumaget.desktop";

    if enabled {
        if let Ok(exe_path) = std::env::current_exe() {
            let exe_str = format!("\"{}\"", exe_path.to_string_lossy());
            key.set_value(app_name, &exe_str)
                .map_err(|e| format!("写入注册表开机自启失败: {e}"))?;
        }
    } else {
        let _ = key.delete_value(app_name);
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn sync_autostart(_enabled: bool) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_autostart_non_failing() {
        let res = sync_autostart(false);
        assert!(res.is_ok());
    }
}
