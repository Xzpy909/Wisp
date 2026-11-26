//! Monitors, DPI, mouse pointer and text caret positions.

use crate::Hwnd;
use wisp_core::{Point, Rect};
use windows_sys::Win32::Foundation::{POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    ClientToScreen, GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, HMONITOR, MONITORINFO,
    MONITOR_DEFAULTTONEAREST,
};
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO,
};


#[derive(Clone, Copy, Debug)]
pub struct Monitor {
    /// Full monitor bounds, physical pixels.
    pub bounds: Rect,
    /// Work area (excludes the taskbar), physical pixels.
    pub work: Rect,
    /// DPI scale, 1.0 at 96 DPI.
    pub scale: f32,
}

fn bounds(r: RECT) -> Rect {
    Rect { x: r.left, y: r.top, w: r.right - r.left, h: r.bottom - r.top }
}

fn monitor_info(h: HMONITOR) -> Option<Monitor> {
    if h.is_null() {
        return None;
    }
    let mut mi: MONITORINFO = unsafe { std::mem::zeroed() };
    mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    if unsafe { GetMonitorInfoW(h, &mut mi) } == 0 {
        return None;
    }
    let (mut dx, mut dy) = (96u32, 96u32);
    let scale = if unsafe { GetDpiForMonitor(h, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) } == 0 {
        dx as f32 / 96.0
    } else {
        1.0
    };
    Some(Monitor { bounds: bounds(mi.rcMonitor), work: bounds(mi.rcWork), scale })
}

pub fn monitor_at(p: Point) -> Option<Monitor> {
    monitor_info(unsafe { MonitorFromPoint(POINT { x: p.x, y: p.y }, MONITOR_DEFAULTTONEAREST) })
}

pub fn monitor_of_window(hwnd: Hwnd) -> Option<Monitor> {
    if hwnd == 0 {
        return None;
    }
    monitor_info(unsafe { MonitorFromWindow(hwnd as _, MONITOR_DEFAULTTONEAREST) })
}

pub fn cursor_pos() -> Option<Point> {
    let mut p = POINT { x: 0, y: 0 };
    (unsafe { GetCursorPos(&mut p) } != 0).then_some(Point { x: p.x, y: p.y })
}

/// Bottom-left of the text caret in the foreground app, in screen pixels.
/// Only apps that use the system caret report it (classic Win32 edit
/// controls, Notepad, many IDEs); browsers and Electron apps usually don't.
pub fn caret_pos() -> Option<Point> {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_null() {
            return None;
        }
        let tid = GetWindowThreadProcessId(fg, std::ptr::null_mut());
        let mut info: GUITHREADINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
        if GetGUIThreadInfo(tid, &mut info) == 0 || info.hwndCaret.is_null() {
            return None;
        }
        let r = info.rcCaret;
        if r.right == r.left && r.bottom == r.top {
            return None;
        }
        let mut p = POINT { x: r.left, y: r.bottom };
        if ClientToScreen(info.hwndCaret, &mut p) == 0 {
            return None;
        }
        Some(Point { x: p.x, y: p.y })
    }
}

/// Size of a small icon (tray, title bar) in pixels at the current display
/// scale: 16 at 100%, 24 at 150%, 32 at 200%.
pub fn small_icon_size() -> u32 {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSMICON};
    let s = unsafe { GetSystemMetrics(SM_CXSMICON) };
    if s > 0 {
        s as u32
    } else {
        16
    }
}
