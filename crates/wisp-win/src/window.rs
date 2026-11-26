//! Popup window styling and focus handling.

use crate::{autostart, wide, Hwnd};
use windows_sys::Win32::Graphics::Dwm::{
    DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMSBT_TRANSIENTWINDOW, DWMWA_SYSTEMBACKDROP_TYPE,
    DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
use windows_sys::Win32::System::Registry::HKEY_CURRENT_USER;
use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows_sys::Win32::UI::Controls::MARGINS;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, GetForegroundWindow, GetWindowLongPtrW, GetWindowThreadProcessId,
    IsWindow, MessageBoxW, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, GWL_STYLE, HWND_TOPMOST,
    MB_ICONWARNING, MB_OK, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_FRAMECHANGED, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
    WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_SYSMENU,
};

pub fn foreground_window() -> Hwnd {
    unsafe { GetForegroundWindow() as Hwnd }
}

/// Keep the popup out of the taskbar and Alt+Tab, keep it on top, round its
/// corners and match the system light or dark theme.
pub fn prepare_popup(hwnd: Hwnd, dark: bool) {
    let h = hwnd as _;
    unsafe {
        let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
        let ex = (ex | WS_EX_TOOLWINDOW as isize) & !(WS_EX_APPWINDOW as isize);
        SetWindowLongPtrW(h, GWL_EXSTYLE, ex);
        // With the DWM frame extended over the whole window (for acrylic),
        // Windows would draw its own caption buttons in the corner. Without a
        // system menu it draws none; Wisp has its own close button.
        let st = GetWindowLongPtrW(h, GWL_STYLE);
        let st = st & !((WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX) as isize);
        SetWindowLongPtrW(h, GWL_STYLE, st);
        SetWindowPos(h, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED);
        let corner = DWMWCP_ROUND;
        DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE as u32, (&corner as *const i32).cast(), 4);
        let d: i32 = dark as i32;
        DwmSetWindowAttribute(h, DWMWA_USE_IMMERSIVE_DARK_MODE as u32, (&d as *const i32).cast(), 4);
    }
}

/// Ask Windows 11 to draw an acrylic backdrop behind the window. Returns false
/// where that isn't supported (Windows 10), in which case the UI stays opaque.
pub fn apply_acrylic(hwnd: Hwnd) -> bool {
    let h = hwnd as _;
    unsafe {
        let m = MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 };
        if DwmExtendFrameIntoClientArea(h, &m) != 0 {
            return false;
        }
        let kind = DWMSBT_TRANSIENTWINDOW;
        DwmSetWindowAttribute(h, DWMWA_SYSTEMBACKDROP_TYPE as u32, (&kind as *const i32).cast(), 4) == 0
    }
}

/// Bring `hwnd` to the front and give it keyboard focus. Windows restricts
/// focus stealing; attaching to the current foreground thread's input queue
/// is the standard, documented way for a hotkey-triggered window to get it.
pub fn bring_to_front(hwnd: Hwnd) {
    let h = hwnd as _;
    unsafe {
        let fg = GetForegroundWindow();
        let me = GetCurrentThreadId();
        let fg_tid = if fg.is_null() { 0 } else { GetWindowThreadProcessId(fg, std::ptr::null_mut()) };
        let attach = fg_tid != 0 && fg_tid != me;
        if attach {
            AttachThreadInput(me, fg_tid, 1);
        }
        BringWindowToTop(h);
        SetForegroundWindow(h);
        SetFocus(h);
        if attach {
            AttachThreadInput(me, fg_tid, 0);
        }
    }
}

/// True when `hwnd` belongs to this process, e.g. a menu Wisp opened.
pub fn is_own_window(hwnd: Hwnd) -> bool {
    if hwnd == 0 {
        return false;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd as _, &mut pid) };
    pid != 0 && pid == unsafe { windows_sys::Win32::System::Threading::GetCurrentProcessId() }
}

/// Give focus back to the window that was active before the popup opened.
pub fn restore_focus(hwnd: Hwnd) {
    if hwnd != 0 && unsafe { IsWindow(hwnd as _) } != 0 {
        unsafe { SetForegroundWindow(hwnd as _) };
    }
}

/// True when Windows apps are set to the dark theme.
pub fn apps_use_dark_theme() -> bool {
    autostart::read_dword(
        HKEY_CURRENT_USER,
        "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize",
        "AppsUseLightTheme",
    ) == Some(0)
}

pub fn warn(title: &str, text: &str) {
    let t = wide(title);
    let b = wide(text);
    unsafe { MessageBoxW(std::ptr::null_mut(), b.as_ptr(), t.as_ptr(), MB_OK | MB_ICONWARNING) };
}
