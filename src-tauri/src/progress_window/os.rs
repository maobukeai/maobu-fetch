//! All visibility/topmost changes for this HWND go through this module on Windows.
//! Never mix these operations with Tao's cached VISIBLE/ALWAYS_ON_TOP mutations.
use super::layout::Placement;
use tauri::WebviewWindow;

#[cfg(windows)]
fn hwnd(window: &WebviewWindow) -> Result<windows_sys::Win32::Foundation::HWND, String> {
    window
        .hwnd()
        .map(|handle| handle.0 as _)
        .map_err(|error| format!("无法取得进度窗口句柄：{error}"))
}

#[cfg(windows)]
pub(super) fn show(window: &WebviewWindow, visible: bool) -> Result<(), String> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        IsWindowVisible, ShowWindow, SW_HIDE, SW_SHOWNOACTIVATE,
    };
    let handle = hwnd(window)?;
    if (unsafe { IsWindowVisible(handle) } != 0) == visible {
        return Ok(());
    }
    unsafe {
        ShowWindow(handle, if visible { SW_SHOWNOACTIVATE } else { SW_HIDE });
    }
    if (unsafe { IsWindowVisible(handle) } != 0) == visible {
        Ok(())
    } else {
        Err("无法切换下载进度悬浮窗的显示状态，请在主窗口查看下载".into())
    }
}

#[cfg(windows)]
pub(super) fn pin(window: &WebviewWindow, pinned: bool) -> Result<(), String> {
    use windows_sys::Win32::{
        Foundation::GetLastError,
        UI::WindowsAndMessaging::{
            GetWindowLongW, SetWindowPos, GWL_EXSTYLE, HWND_NOTOPMOST, HWND_TOPMOST,
            SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, WS_EX_TOPMOST,
        },
    };
    let handle = hwnd(window)?;
    if (unsafe { GetWindowLongW(handle, GWL_EXSTYLE) } as u32 & WS_EX_TOPMOST != 0) == pinned {
        return Ok(());
    }
    let result = unsafe {
        SetWindowPos(
            handle,
            if pinned { HWND_TOPMOST } else { HWND_NOTOPMOST },
            0,
            0,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
        )
    };
    if result != 0 {
        Ok(())
    } else {
        Err(format!(
            "无法切换进度悬浮窗置顶状态（Windows错误码{}）",
            unsafe { GetLastError() }
        ))
    }
}

#[cfg(windows)]
pub(super) fn place(window: &WebviewWindow, placement: &Placement) -> Result<(), String> {
    use windows_sys::Win32::{
        Foundation::{GetLastError, RECT},
        UI::WindowsAndMessaging::{GetWindowRect, SetWindowPos, SWP_NOACTIVATE, SWP_NOZORDER},
    };
    let handle = hwnd(window)?;
    let mut current = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    if unsafe { GetWindowRect(handle, &mut current) } != 0
        && current.left == placement.x
        && current.top == placement.y
        && i64::from(current.right) - i64::from(current.left) == i64::from(placement.width)
        && i64::from(current.bottom) - i64::from(current.top) == i64::from(placement.height)
    {
        return Ok(());
    }
    let result = unsafe {
        SetWindowPos(
            handle,
            std::ptr::null_mut(),
            placement.x,
            placement.y,
            placement.width.min(i32::MAX as u32) as i32,
            placement.height.min(i32::MAX as u32) as i32,
            SWP_NOACTIVATE | SWP_NOZORDER,
        )
    };
    if result != 0 {
        Ok(())
    } else {
        Err(format!(
            "无法定位下载进度悬浮窗（Windows错误码{}）",
            unsafe { GetLastError() }
        ))
    }
}

#[cfg(not(windows))]
pub(super) fn show(window: &WebviewWindow, visible: bool) -> Result<(), String> {
    (if visible {
        window.show()
    } else {
        window.hide()
    })
    .map_err(|error| error.to_string())
}
#[cfg(not(windows))]
pub(super) fn pin(window: &WebviewWindow, pinned: bool) -> Result<(), String> {
    window
        .set_always_on_top(pinned)
        .map_err(|error| error.to_string())
}
#[cfg(not(windows))]
pub(super) fn place(window: &WebviewWindow, placement: &Placement) -> Result<(), String> {
    window
        .set_position(tauri::PhysicalPosition::new(placement.x, placement.y))
        .map_err(|error| error.to_string())?;
    window
        .set_size(tauri::PhysicalSize::new(placement.width, placement.height))
        .map_err(|error| error.to_string())
}
