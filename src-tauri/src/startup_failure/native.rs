use super::FailureSink;
pub(crate) struct NativeSink;

impl FailureSink for NativeSink {
    fn stderr(&self, message: &str) {
        // Uses the existing redacting log subscriber when setup initialized it; no new
        // file, repair, database or settings write is performed by this reporter.
        tracing::error!(reason = %message, "应用启动失败");
        eprintln!("{message}");
    }
    fn dialog(&self, title: &str, message: &str) -> Result<(), String> {
        show(title, message)
    }
}

pub(super) fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16()
        .filter(|character| *character != 0)
        .chain([0])
        .collect()
}

#[cfg(windows)]
fn show(title: &str, message: &str) -> Result<(), String> {
    use windows_sys::Win32::{
        Foundation::GetLastError,
        UI::WindowsAndMessaging::{
            MessageBoxW, MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MB_TASKMODAL,
        },
    };
    let title = wide(title);
    let message = wide(message);
    // Both buffers remain alive and NUL terminated until the modal call returns.
    let result = unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR | MB_SETFOREGROUND | MB_TASKMODAL,
        )
    };
    if result == 0 {
        Err(format!("Windows 系统错误码 {}", unsafe {
            GetLastError()
        }))
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn show(_title: &str, _message: &str) -> Result<(), String> {
    Err("当前平台无法显示 Windows 启动错误窗口，请查看终端中的错误信息".into())
}
