//! 仅测试：对私有测试键设置可恢复 DACL，产生真实 Windows 权限拒绝。
use std::{io, ptr};
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::{Authorization::*, *},
};
use winreg::RegKey;

pub(super) struct DenyAccess<'a> {
    key: &'a RegKey,
    original: *mut ACL,
    descriptor: PSECURITY_DESCRIPTOR,
    modified: *mut ACL,
    restored: bool,
}

impl<'a> DenyAccess<'a> {
    pub(super) fn new(key: &'a RegKey, denied: u32) -> io::Result<Self> {
        let mut original = ptr::null_mut();
        let mut descriptor = ptr::null_mut();
        // SAFETY: the injected key is owned by the test; output memory is kept
        // until after restoration and released using LocalFree below.
        let status = unsafe {
            GetSecurityInfo(
                key.raw_handle() as _,
                SE_REGISTRY_KEY,
                DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                &mut original,
                ptr::null_mut(),
                &mut descriptor,
            )
        };
        check(status)?;
        let mut guard = Self {
            key,
            original,
            descriptor,
            modified: ptr::null_mut(),
            restored: true,
        };
        let mut sid = [0usize; 16];
        let mut size = std::mem::size_of_val(&sid) as u32;
        if unsafe {
            CreateWellKnownSid(
                WinWorldSid,
                ptr::null_mut(),
                sid.as_mut_ptr().cast(),
                &mut size,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let entry = EXPLICIT_ACCESS_W {
            grfAccessPermissions: denied,
            grfAccessMode: DENY_ACCESS,
            grfInheritance: NO_INHERITANCE,
            Trustee: TRUSTEE_W {
                pMultipleTrustee: ptr::null_mut(),
                MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_WELL_KNOWN_GROUP,
                ptstrName: sid.as_mut_ptr().cast(),
            },
        };
        check(unsafe { SetEntriesInAclW(1, &entry, original, &mut guard.modified) })?;
        check(unsafe {
            SetSecurityInfo(
                key.raw_handle() as _,
                SE_REGISTRY_KEY,
                DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                guard.modified,
                ptr::null_mut(),
            )
        })?;
        guard.restored = false;
        Ok(guard)
    }

    pub(super) fn restore(&mut self) -> io::Result<()> {
        if !self.restored {
            check(unsafe {
                SetSecurityInfo(
                    self.key.raw_handle() as _,
                    SE_REGISTRY_KEY,
                    DACL_SECURITY_INFORMATION,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    self.original,
                    ptr::null_mut(),
                )
            })?;
            self.restored = true;
        }
        Ok(())
    }
}

impl Drop for DenyAccess<'_> {
    fn drop(&mut self) {
        if let Err(error) = self.restore() {
            eprintln!("failed to restore private file-association test ACL: {error}");
        }
        unsafe {
            if !self.modified.is_null() {
                LocalFree(self.modified.cast());
            }
            if !self.descriptor.is_null() {
                LocalFree(self.descriptor);
            }
        }
    }
}

fn check(status: u32) -> io::Result<()> {
    if status == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(status as i32))
    }
}
