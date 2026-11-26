//! A small HTTPS GET over WinHTTP, which uses the system proxy settings and
//! certificate store, so Wisp needs no TLS library of its own.

use crate::wide;
use std::ffi::c_void;
use std::io;
use windows_sys::Win32::Networking::WinHttp::{
    WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReadData,
    WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetTimeouts, INTERNET_DEFAULT_HTTPS_PORT,
    WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
};

/// Largest response body accepted.
const MAX_BODY: usize = 1 << 20;

struct Handle(*mut c_void);

impl Handle {
    fn new(h: *mut c_void) -> io::Result<Self> {
        if h.is_null() { Err(io::Error::last_os_error()) } else { Ok(Self(h)) }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: a handle returned by WinHttpOpen/Connect/OpenRequest, closed once.
        unsafe { WinHttpCloseHandle(self.0) };
    }
}

fn check(ok: i32) -> io::Result<()> {
    if ok != 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

/// GET `https://{host}{path}` and return the status code and body.
pub fn get(host: &str, path: &str, user_agent: &str, extra_headers: &str) -> io::Result<(u32, Vec<u8>)> {
    let agent = wide(user_agent);
    let host = wide(host);
    let path = wide(path);
    let verb = wide("GET");
    let headers = wide(extra_headers);
    unsafe {
        let session = Handle::new(WinHttpOpen(
            agent.as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            std::ptr::null(),
            std::ptr::null(),
            0,
        ))?;
        // Resolve, connect, send, receive: 10 s each.
        WinHttpSetTimeouts(session.0, 10_000, 10_000, 10_000, 10_000);
        let connect = Handle::new(WinHttpConnect(session.0, host.as_ptr(), INTERNET_DEFAULT_HTTPS_PORT, 0))?;
        let request = Handle::new(WinHttpOpenRequest(
            connect.0,
            verb.as_ptr(),
            path.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        ))?;
        let header_len = if extra_headers.is_empty() { 0 } else { u32::MAX }; // -1: null-terminated
        let header_ptr = if extra_headers.is_empty() { std::ptr::null() } else { headers.as_ptr() };
        check(WinHttpSendRequest(request.0, header_ptr, header_len, std::ptr::null(), 0, 0, 0))?;
        check(WinHttpReceiveResponse(request.0, std::ptr::null_mut()))?;

        let mut status: u32 = 0;
        let mut size = 4u32;
        check(WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            std::ptr::null(),
            (&mut status as *mut u32).cast(),
            &mut size,
            std::ptr::null_mut(),
        ))?;

        let mut body = Vec::new();
        let mut chunk = vec![0u8; 16 * 1024];
        loop {
            let mut read = 0u32;
            check(WinHttpReadData(request.0, chunk.as_mut_ptr().cast(), chunk.len() as u32, &mut read))?;
            if read == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..read as usize]);
            if body.len() > MAX_BODY {
                return Err(io::Error::other("response too large"));
            }
        }
        Ok((status, body))
    }
}

/// Open an https link in the default browser.
pub fn open_in_browser(url: &str) {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    if !url.starts_with("https://") {
        return;
    }
    let verb = wide("open");
    let target = wide(url);
    // SAFETY: valid null-terminated strings; no parent window.
    unsafe {
        ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), target.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL)
    };
}
