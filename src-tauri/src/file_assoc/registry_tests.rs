use super::permission_fixture::DenyAccess;
use super::*;
use winreg::{enums::*, RegKey};

const TEST_PREFIX: &str = "Software\\MaobuFetchTests\\file-assoc-";
struct TestRoot {
    key: RegKey,
    path: String,
}
impl TestRoot {
    fn new() -> Self {
        let path = format!("{TEST_PREFIX}{}", uuid::Uuid::new_v4());
        let key = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey(&path)
            .unwrap()
            .0;
        Self { key, path }
    }
    fn exe(&self) -> std::path::PathBuf {
        std::path::PathBuf::from("C:\\Isolated Fixture\\猫步下载器.exe")
    }
    fn assert_cleanup(self) {
        let path = self.path.clone();
        drop(self);
        let error = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(path)
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    }
}
impl Drop for TestRoot {
    fn drop(&mut self) {
        assert!(self.path.starts_with(TEST_PREFIX));
        assert!(uuid::Uuid::parse_str(self.path.trim_start_matches(TEST_PREFIX)).is_ok());
        RegKey::predef(HKEY_CURRENT_USER)
            .delete_subkey_all(&self.path)
            .expect("clean private association test root");
    }
}

#[test]
fn injected_registry_enable_disable_is_idempotent_and_preserves_default_apps() {
    let root = TestRoot::new();
    let foreign = root.key.create_subkey("Software\\Classes\\.mp4").unwrap().0;
    foreign.set_value("", &"Other.Video.Player").unwrap();
    let user_choice = root
        .key
        .create_subkey(
            "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\FileExts\\.mp4\\UserChoice",
        )
        .unwrap()
        .0;
    user_choice
        .set_value("ProgId", &"Other.Video.Player")
        .unwrap();
    user_choice
        .set_value("Hash", &"unchanged-fixture-hash")
        .unwrap();
    let selected = vec![".MP4".into(), "png".into(), "mp4".into()];
    for _ in 0..2 {
        registry::set(&root.key, &root.exe(), &selected, true).unwrap();
    }
    let list = registry::list(&root.key).unwrap();
    assert!(list
        .iter()
        .filter(|item| ["mp4", "png"].contains(&item.extension.as_str()))
        .all(|item| item.is_associated));
    let command: String = root
        .key
        .open_subkey("Software\\Classes\\MaobuFetch.Video\\shell\\open\\command")
        .unwrap()
        .get_value("")
        .unwrap();
    assert_eq!(
        command,
        "\"C:\\Isolated Fixture\\猫步下载器.exe\" --play \"%1\""
    );
    let image: String = root
        .key
        .open_subkey("Software\\Classes\\MaobuFetch.Image\\shell\\open\\command")
        .unwrap()
        .get_value("")
        .unwrap();
    assert!(image.contains("--view-image"));
    for _ in 0..2 {
        registry::set(&root.key, &root.exe(), &selected, false).unwrap();
    }
    assert!(registry::list(&root.key)
        .unwrap()
        .iter()
        .all(|item| !item.is_associated));
    assert_eq!(
        foreign.get_value::<String, _>("").unwrap(),
        "Other.Video.Player"
    );
    assert_eq!(
        user_choice.get_value::<String, _>("ProgId").unwrap(),
        "Other.Video.Player"
    );
    assert_eq!(
        user_choice.get_value::<String, _>("Hash").unwrap(),
        "unchanged-fixture-hash"
    );
    assert!(root
        .key
        .open_subkey("Software\\Classes\\.mp4\\OpenWithList\\maobu-fetch.exe")
        .is_err());
}

#[test]
fn unknown_extensions_fail_before_any_registration_and_ensure_records_stable_ids() {
    let root = TestRoot::new();
    for input in [
        vec!["mp4".into(), "exe".into()],
        vec!["..\\external".into()],
        vec!["".into()],
    ] {
        assert!(registry::set(&root.key, &root.exe(), &input, true).is_err());
    }
    assert_eq!(root.key.enum_keys().count(), 0);
    registry::ensure(&root.key, &root.exe()).unwrap();
    let registered: String = root
        .key
        .open_subkey("Software\\RegisteredApplications")
        .unwrap()
        .get_value("MaobuFetch")
        .unwrap();
    assert_eq!(registered, "Software\\MaobuFetch\\Capabilities");
    let types = root
        .key
        .open_subkey("Software\\Classes\\Applications\\maobu-fetch.exe\\SupportedTypes")
        .unwrap();
    assert_eq!(
        types.enum_values().count(),
        SUPPORTED_VIDEO_EXTS.len() + SUPPORTED_IMAGE_EXTS.len()
    );
    let command: String = root
        .key
        .open_subkey("Software\\Classes\\Applications\\maobu-fetch.exe\\shell\\open\\command")
        .unwrap()
        .get_value("")
        .unwrap();
    assert_eq!(command, "\"C:\\Isolated Fixture\\猫步下载器.exe\" \"%1\"");
}

#[test]
fn real_denied_creation_does_not_report_ensure_or_enable_success() {
    let root = TestRoot::new();
    let mut permissions = DenyAccess::new(&root.key, KEY_CREATE_SUB_KEY | KEY_SET_VALUE).unwrap();
    let os_denial = root.key.create_subkey("NoPermission").unwrap_err();
    assert_eq!(os_denial.kind(), std::io::ErrorKind::PermissionDenied);
    let ensured = registry::ensure(&root.key, &root.exe());
    let enabled = registry::set(&root.key, &root.exe(), &["mp4".into()], true);
    permissions.restore().unwrap();
    assert!(ensured.is_err() && enabled.is_err());
    registry::ensure(&root.key, &root.exe()).unwrap();
    drop(permissions);
    root.assert_cleanup();
}

#[test]
fn real_denied_value_deletion_is_not_swallowed() {
    let root = TestRoot::new();
    registry::set(&root.key, &root.exe(), &["mp4".into()], true).unwrap();
    let key = root
        .key
        .open_subkey_with_flags("Software\\Classes\\.mp4\\OpenWithProgids", KEY_ALL_ACCESS)
        .unwrap();
    let mut permissions = DenyAccess::new(&key, KEY_SET_VALUE).unwrap();
    let result = registry::set(&root.key, &root.exe(), &["mp4".into()], false);
    permissions.restore().unwrap();
    assert_eq!(
        result.unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert!(
        registry::list(&root.key)
            .unwrap()
            .iter()
            .find(|item| item.extension == "mp4")
            .unwrap()
            .is_associated
    );
    registry::set(&root.key, &root.exe(), &["mp4".into()], false).unwrap();
    drop(permissions);
    drop(key);
    root.assert_cleanup();
}

#[test]
fn isolated_top_level_calls_leave_real_run_value_unchanged() {
    fn snapshot() -> std::io::Result<Option<(u32, Vec<u8>)>> {
        let root = match RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(
            "Software\\Microsoft\\Windows\\CurrentVersion\\Run",
            KEY_QUERY_VALUE,
        ) {
            Ok(key) => key,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        match root.get_raw_value("app.lumaget.desktop") {
            Ok(value) => Ok(Some((value.vtype as u32, value.bytes))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }
    let before = snapshot().unwrap();
    ensure_registered_applications().unwrap();
    assert!(set_file_associations(vec!["mp4".into()], true).is_err());
    assert!(set_file_associations(vec!["png".into()], false).is_err());
    assert!(open_default_apps_settings().is_err());
    assert!(
        before == snapshot().unwrap(),
        "isolated file-association calls must not change the real application Run value"
    );
}
