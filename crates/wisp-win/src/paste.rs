//! Put a clip back on the clipboard and paste it into the active app.

use crate::wide;
use std::path::Path;
use windows_sys::Win32::Foundation::{GlobalFree, HANDLE, POINT};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows_sys::Win32::System::Ole::{CF_DIB, CF_HDROP, CF_UNICODETEXT};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY,
    VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN,
};
use windows_sys::Win32::UI::Shell::DROPFILES;
use wisp_core::{images, ClipContent};

/// Clipboard format Wisp adds to what it writes, so its own listener skips it.
pub const OWN_FORMAT: &str = "Wisp.OwnClip";

/// Closes the clipboard when dropped.
struct Open;
impl Drop for Open {
    fn drop(&mut self) {
        unsafe { CloseClipboard() };
    }
}

fn format(name: &str) -> u32 {
    let w = wide(name);
    unsafe { RegisterClipboardFormatW(w.as_ptr()) }
}

/// Hand `bytes` to the clipboard as format `fmt`.
unsafe fn set(fmt: u32, bytes: &[u8]) -> bool {
    let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1));
    if h.is_null() {
        return false;
    }
    let p = GlobalLock(h) as *mut u8;
    if p.is_null() {
        GlobalFree(h);
        return false;
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
    GlobalUnlock(h);
    // On success the clipboard owns the memory.
    if SetClipboardData(fmt, h as HANDLE).is_null() {
        GlobalFree(h);
        return false;
    }
    true
}

fn utf16_bytes(text: &str) -> Vec<u8> {
    text.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect()
}

/// `CF_HDROP` data: a DROPFILES header, then the paths as wide strings, each
/// ending in a zero, with one more zero at the end.
fn hdrop_bytes(paths: &[impl AsRef<Path>]) -> Vec<u8> {
    let header = std::mem::size_of::<DROPFILES>();
    let mut out = vec![0u8; header];
    let df = DROPFILES { pFiles: header as u32, pt: POINT { x: 0, y: 0 }, fNC: 0, fWide: 1 };
    unsafe { std::ptr::copy_nonoverlapping((&df as *const DROPFILES).cast::<u8>(), out.as_mut_ptr(), header) };
    for p in paths {
        out.extend(utf16_bytes(&p.as_ref().to_string_lossy()));
    }
    out.extend([0, 0]);
    out
}

/// Write a clip to the clipboard. With `plain`, files go as their paths in
/// text instead of as files.
pub fn write(content: &ClipContent, plain: bool) -> Result<(), String> {
    let own = format(OWN_FORMAT);
    let mut opened = false;
    for _ in 0..10 {
        if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
            opened = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
    if !opened {
        return Err("another app is using the clipboard".into());
    }
    let _guard = Open;
    unsafe {
        EmptyClipboard();
        set(own, &[1]);
        let ok = match content {
            ClipContent::Text(t) => set(CF_UNICODETEXT as u32, &utf16_bytes(t)),
            ClipContent::Files(paths) if plain => {
                let text = paths.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>().join("\r\n");
                set(CF_UNICODETEXT as u32, &utf16_bytes(&text))
            }
            ClipContent::Files(paths) => {
                // Paste as a copy, not a move.
                let effect = format("Preferred DropEffect");
                set(effect, &1u32.to_le_bytes());
                set(CF_HDROP as u32, &hdrop_bytes(paths))
            }
            ClipContent::Image { png, .. } => {
                // PNG keeps transparency for apps that read it; the DIB is
                // what every other app pastes.
                let png_ok = set(format("PNG"), png);
                let dib_ok = images::decode_png(png).is_some_and(|img| set(CF_DIB as u32, &images::to_dib(&img)));
                png_ok || dib_ok
            }
        };
        if ok {
            Ok(())
        } else {
            Err("Windows did not accept the clip".into())
        }
    }
}

fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// Press Ctrl+V in whatever window has focus. Modifier keys still held down
/// (Shift from Shift+Enter, say) are released first, so the app sees a plain
/// Ctrl+V.
pub fn send_ctrl_v() {
    let mut inputs = Vec::new();
    for vk in [VK_LSHIFT, VK_RSHIFT, VK_LMENU, VK_RMENU, VK_LWIN, VK_RWIN, VK_LCONTROL, VK_RCONTROL] {
        if unsafe { GetAsyncKeyState(vk as i32) } < 0 {
            inputs.push(key(vk, true));
        }
    }
    const VK_V: VIRTUAL_KEY = b'V' as VIRTUAL_KEY;
    inputs.extend([key(VK_CONTROL, false), key(VK_V, false), key(VK_V, true), key(VK_CONTROL, true)]);
    unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
}

/// Open File Explorer with `path` selected.
pub fn reveal(path: &Path) {
    use std::os::windows::process::CommandExt;
    let _ = std::process::Command::new("explorer")
        .raw_arg(format!("/select,\"{}\"", path.display()))
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hdrop_layout() {
        let b = hdrop_bytes(&["C:\\a", "D:\\b"]);
        assert_eq!(u32::from_le_bytes([b[0], b[1], b[2], b[3]]), 20);
        assert_eq!(u32::from_le_bytes([b[16], b[17], b[18], b[19]]), 1);
        // 2 paths × (4 chars + zero) + final zero, 2 bytes each.
        assert_eq!(b.len(), 20 + (5 + 5 + 1) * 2);
        assert!(b.ends_with(&[0, 0, 0, 0]));
    }
}
