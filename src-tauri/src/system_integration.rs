//! 本机隔离验证不能修改真实自启/文件关联等全局用户状态。
pub(crate) fn allowed() -> bool {
    !cfg!(test) && std::env::var_os("MAOBU_FETCH_ISOLATED_TEST").as_deref() != Some(std::ffi::OsStr::new("1"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn native_tests_never_enable_global_user_integration() {
        assert!(!super::allowed());
    }
}
