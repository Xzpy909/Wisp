//! Win32 helpers for Wisp. Every function is a thin, safe wrapper over one
//! Windows API concern; the app crate never calls Windows directly.
#![cfg(windows)]

pub mod autostart;
pub mod clipboard;
pub mod instance;
pub mod paste;
pub mod screen;
pub mod window;

/// A window handle passed around as an integer, so it is `Send` and `Copy`.
pub type Hwnd = isize;

pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
