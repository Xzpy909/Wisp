//! Clipboard listener. A hidden message-only window on its own thread gets
//! `WM_CLIPBOARDUPDATE` on every copy; nothing polls.

use crate::wide;
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::DataExchange::{
    AddClipboardFormatListener, CloseClipboard, GetClipboardData, GetClipboardOwner, IsClipboardFormatAvailable,
    OpenClipboard, RegisterClipboardFormatW,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows_sys::Win32::System::Ole::{CF_DIB, CF_DIBV5, CF_HDROP, CF_UNICODETEXT};
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetForegroundWindow, GetMessageW, GetWindowThreadProcessId,
    RegisterClassExW, TranslateMessage, HWND_MESSAGE, MSG, WM_CLIPBOARDUPDATE, WNDCLASSEXW,
};
use wisp_core::{images, Captured, ClipContent};

/// Text larger than this is not recorded.
pub const MAX_TEXT_BYTES: usize = 1 << 20;
/// Images with more pixels than this (about 8K × 5K) are not recorded.
pub const MAX_IMAGE_PIXELS: u64 = 40_000_000;

/// Switches the UI can flip while the listener runs.
#[derive(Default)]
pub struct CaptureOptions {
    pub paused: AtomicBool,
    pub images: AtomicBool,
    pub files: AtomicBool,
}

type Callback = Box<dyn Fn(Captured)>;

thread_local! {
    static STATE: RefCell<Option<(Arc<CaptureOptions>, Callback)>> = const { RefCell::new(None) };
}

/// Start listening on a background thread. `on_capture` runs on that thread.
pub fn start(options: Arc<CaptureOptions>, on_capture: impl Fn(Captured) + Send + 'static) {
    std::thread::Builder::new()
        .name("wisp-clipboard".into())
        .spawn(move || {
            STATE.with(|s| *s.borrow_mut() = Some((options, Box::new(on_capture))));
            unsafe { run_listener() };
        })
        .expect("spawn clipboard thread");
}

unsafe fn run_listener() {
    let class = wide("WispClipboardListener");
    let hinstance = GetModuleHandleW(std::ptr::null());
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(wnd_proc),
        hInstance: hinstance,
        lpszClassName: class.as_ptr(),
        ..std::mem::zeroed()
    };
    RegisterClassExW(&wc);
    let hwnd = CreateWindowExW(
        0,
        class.as_ptr(),
        std::ptr::null(),
        0,
        0,
        0,
        0,
        0,
        HWND_MESSAGE,
        std::ptr::null_mut(),
        hinstance,
        std::ptr::null(),
    );
    if hwnd.is_null() || AddClipboardFormatListener(hwnd) == 0 {
        return;
    }
    let mut msg: MSG = std::mem::zeroed();
    while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_CLIPBOARDUPDATE {
        STATE.with(|s| {
            if let Some((opts, cb)) = s.borrow().as_ref() {
                if !opts.paused.load(Ordering::Relaxed) {
                    if let Some(cap) = read_clipboard(hwnd, opts) {
                        cb(cap);
                    }
                }
            }
        });
        return 0;
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

fn format(name: &str) -> u32 {
    let w = wide(name);
    unsafe { RegisterClipboardFormatW(w.as_ptr()) }
}

/// Closes the clipboard when dropped.
struct Open;
impl Drop for Open {
    fn drop(&mut self) {
        unsafe { CloseClipboard() };
    }
}

fn open(hwnd: HWND) -> Option<Open> {
    // Another app may still hold the clipboard right after writing it.
    for _ in 0..10 {
        if unsafe { OpenClipboard(hwnd) } != 0 {
            return Some(Open);
        }
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
    None
}

fn available(fmt: u32) -> bool {
    fmt != 0 && unsafe { IsClipboardFormatAvailable(fmt) } != 0
}

/// Copy a clipboard format's global memory out as bytes.
fn global_bytes(fmt: u32) -> Option<Vec<u8>> {
    unsafe {
        let h = GetClipboardData(fmt);
        if h.is_null() {
            return None;
        }
        let size = GlobalSize(h);
        let p = GlobalLock(h) as *const u8;
        if p.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(p, size).to_vec();
        GlobalUnlock(h);
        Some(bytes)
    }
}

fn read_clipboard(hwnd: HWND, opts: &CaptureOptions) -> Option<Captured> {
    // Password managers and other apps mark private content with these.
    let exclude = format("ExcludeClipboardContentFromMonitorProcessing");
    let viewer_ignore = format("Clipboard Viewer Ignore");
    let can_include = format("CanIncludeInClipboardHistory");
    let png_fmt = format("PNG");

    let _guard = open(hwnd)?;
    if available(exclude) || available(viewer_ignore) {
        return None;
    }
    if available(can_include) {
        if let Some(b) = global_bytes(can_include) {
            if b.len() >= 4 && u32::from_le_bytes([b[0], b[1], b[2], b[3]]) == 0 {
                return None;
            }
        }
    }

    let content = if available(CF_HDROP as u32) {
        if !opts.files.load(Ordering::Relaxed) {
            return None;
        }
        ClipContent::Files(read_files()?)
    } else if available(CF_UNICODETEXT as u32) {
        let text = read_text()?;
        if text.trim().is_empty() || text.len() > MAX_TEXT_BYTES {
            return None;
        }
        ClipContent::Text(text)
    } else if available(png_fmt) || available(CF_DIBV5 as u32) || available(CF_DIB as u32) {
        if !opts.images.load(Ordering::Relaxed) {
            return None;
        }
        read_image(png_fmt)?
    } else {
        return None;
    };
    Some(Captured { content, source_app: source_app() })
}

fn read_text() -> Option<String> {
    let bytes = global_bytes(CF_UNICODETEXT as u32)?;
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|&c| u16::from_le_bytes(c)).collect();
    let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
    Some(String::from_utf16_lossy(&units[..end]))
}

fn read_files() -> Option<Vec<PathBuf>> {
    unsafe {
        let h = GetClipboardData(CF_HDROP as u32) as HDROP;
        if h.is_null() {
            return None;
        }
        let count = DragQueryFileW(h, u32::MAX, std::ptr::null_mut(), 0);
        let mut paths = Vec::with_capacity(count as usize);
        for i in 0..count {
            let len = DragQueryFileW(h, i, std::ptr::null_mut(), 0) as usize;
            let mut buf = vec![0u16; len + 1];
            DragQueryFileW(h, i, buf.as_mut_ptr(), buf.len() as u32);
            paths.push(PathBuf::from(String::from_utf16_lossy(&buf[..len])));
        }
        (!paths.is_empty()).then_some(paths)
    }
}

fn read_image(png_fmt: u32) -> Option<ClipContent> {
    // Browsers and modern apps offer PNG, which keeps transparency.
    if available(png_fmt) {
        if let Some(png) = global_bytes(png_fmt) {
            if let Some((width, height)) = images::png_size(&png) {
                if (width as u64 * height as u64) <= MAX_IMAGE_PIXELS {
                    return Some(ClipContent::Image { png, width, height });
                }
                return None;
            }
        }
    }
    let fmt = if available(CF_DIBV5 as u32) { CF_DIBV5 } else { CF_DIB } as u32;
    let dib = global_bytes(fmt)?;
    let rgba = images::from_dib(&dib)?;
    if rgba.width as u64 * rgba.height as u64 > MAX_IMAGE_PIXELS {
        return None;
    }
    let png = images::encode_png(&rgba).ok()?;
    Some(ClipContent::Image { png, width: rgba.width, height: rgba.height })
}

/// Name of the app that put the content on the clipboard, e.g. "notepad".
fn source_app() -> Option<String> {
    unsafe {
        let mut owner = GetClipboardOwner();
        if owner.is_null() {
            owner = GetForegroundWindow();
        }
        if owner.is_null() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(owner, &mut pid);
        if pid == 0 {
            return None;
        }
        let process: HANDLE = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return None;
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len) != 0;
        CloseHandle(process);
        if !ok {
            return None;
        }
        let path = PathBuf::from(String::from_utf16_lossy(&buf[..len as usize]));
        path.file_stem().map(|s| s.to_string_lossy().into_owned())
    }
}
