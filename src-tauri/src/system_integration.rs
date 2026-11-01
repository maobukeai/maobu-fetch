//! 测试和显式受限的调试启动不能修改真实自启/文件关联等全局用户状态。
use std::ffi::OsStr;

pub(crate) fn allowed() -> bool {
    integration_allowed(
        cfg!(test),
        std::env::var_os("MAOBU_FETCH_ISOLATED_TEST").as_deref(),
        std::env::var_os("MAOBU_FETCH_SKIP_SYSTEM_INTEGRATION").as_deref(),
    )
}

fn integration_allowed(is_test: bool, isolated: Option<&OsStr>, skip: Option<&OsStr>) -> bool {
    !is_test && isolated != Some(OsStr::new("1")) && skip != Some(OsStr::new("1"))
}

#[cfg(test)]
mod tests {
    use super::integration_allowed;
    use std::ffi::OsStr;

    #[test]
    fn native_tests_never_enable_global_user_integration() {
        assert!(!super::allowed());
    }

    #[test]
    fn normal_startup_keeps_system_integration() {
        assert!(integration_allowed(false, None, None));
        assert!(integration_allowed(
            false,
            Some(OsStr::new("0")),
            Some(OsStr::new("0")),
        ));
    }

    #[test]
    fn debug_can_skip_registration_without_enabling_isolated_mode() {
        assert!(!integration_allowed(false, None, Some(OsStr::new("1"))));
        assert!(!integration_allowed(true, None, None));
    }

    #[test]
    fn isolated_startup_still_prevents_global_registration() {
        assert!(!integration_allowed(false, Some(OsStr::new("1")), None));
    }
}
