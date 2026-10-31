//! 开机自启模块（Windows 注册表同步）
//! 遵循 AGENTS.md 强约束：使用保留标识 `app.lumaget.desktop` 作为注册表键名。

#[cfg(all(windows, not(test)))]
pub fn sync_autostart(enabled: bool) -> Result<(), String> {
    if !crate::system_integration::allowed() { return Ok(()); }
    use winreg::enums::*;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let run_path = r"Software\Microsoft\Windows\CurrentVersion\Run";
    let (key, _) = hkcu
        .create_subkey(run_path)
        .map_err(|e| format!("打开注册表 Run 键失败: {e}"))?;

    let exe=std::env::current_exe().map_err(|e|format!("读取程序路径以同步开机启动失败：{e}"))?;
    sync_value(&key,enabled,&exe)
}

// 单元/原生集成测试不能触碰真实用户的 Run 项。
#[cfg(any(not(windows), test))]
pub fn sync_autostart(_enabled: bool) -> Result<(), String> {
    Ok(())
}

#[cfg(windows)]
fn sync_value(key:&winreg::RegKey,enabled:bool,exe:&std::path::Path)->Result<(),String>{
    let app_name="app.lumaget.desktop";
    if enabled {
        let command=format!("\"{}\" --autostart",exe.to_string_lossy());
        key.set_value(app_name,&command).map_err(|e|format!("写入注册表开机自启失败：{e}"))
    } else {
        match key.delete_value(app_name) {
            Ok(())=>Ok(()),
            Err(error) if error.kind()==std::io::ErrorKind::NotFound=>Ok(()),
            Err(error)=>Err(format!("移除开机自启失败，请检查注册表权限：{error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_autostart_does_not_write_to_real_user_startup() {
        assert!(sync_autostart(false).is_ok());
        assert!(sync_autostart(true).is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn actual_registry_behavior_is_confined_to_a_unique_test_key() {
        use winreg::{enums::HKEY_CURRENT_USER,RegKey};
        let root=RegKey::predef(HKEY_CURRENT_USER);
        let path=format!("Software\\MaobuFetchTests\\autostart-{}",uuid::Uuid::new_v4());
        let (key,_)=root.create_subkey(&path).unwrap();
        sync_value(&key,true,std::path::Path::new("C:\\Synthetic\\maobu-fetch.exe")).unwrap();
        assert_eq!(key.get_value::<String,_>("app.lumaget.desktop").unwrap(),"\"C:\\Synthetic\\maobu-fetch.exe\" --autostart");
        sync_value(&key,false,std::path::Path::new("unused")).unwrap();
        sync_value(&key,false,std::path::Path::new("unused")).unwrap();
        assert!(key.get_value::<String,_>("app.lumaget.desktop").is_err());
        drop(key);
        assert!(path.starts_with("Software\\MaobuFetchTests\\autostart-"));
        root.delete_subkey(path).unwrap();
    }
}
