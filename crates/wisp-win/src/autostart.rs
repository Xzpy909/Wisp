//! Start with Windows via the per-user Run key (no admin rights needed).

use crate::wide;
use std::io;
use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegGetValueW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
    KEY_QUERY_VALUE, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ, RRF_RT_REG_DWORD,
};

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const VALUE_NAME: &str = "Wisp";

fn err(code: u32) -> io::Error {
    io::Error::from_raw_os_error(code as i32)
}

fn open_run_key() -> io::Result<HKEY> {
    let path = wide(RUN_KEY);
    let mut key: HKEY = std::ptr::null_mut();
    let r = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            path.as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE | KEY_QUERY_VALUE,
            std::ptr::null(),
            &mut key,
            std::ptr::null_mut(),
        )
    };
    if r != ERROR_SUCCESS {
        return Err(err(r));
    }
    Ok(key)
}

/// The command Windows runs at login: this executable, started hidden.
pub fn command_for_current_exe() -> io::Result<String> {
    let exe = std::env::current_exe()?;
    Ok(format!("\"{}\" --background", exe.display()))
}

/// Register (`Some(command)`) or remove (`None`) the login entry.
pub fn set(command: Option<&str>) -> io::Result<()> {
    let key = open_run_key()?;
    let name = wide(VALUE_NAME);
    let r = match command {
        Some(cmd) => {
            let data = wide(cmd);
            unsafe {
                RegSetValueExW(
                    key,
                    name.as_ptr(),
                    0,
                    REG_SZ,
                    data.as_ptr().cast(),
                    (data.len() * 2) as u32,
                )
            }
        }
        None => match unsafe { RegDeleteValueW(key, name.as_ptr()) } {
            ERROR_FILE_NOT_FOUND => ERROR_SUCCESS,
            other => other,
        },
    };
    unsafe { RegCloseKey(key) };
    if r == ERROR_SUCCESS { Ok(()) } else { Err(err(r)) }
}

/// The currently registered command, if any.
pub fn current() -> Option<String> {
    read_string(HKEY_CURRENT_USER, RUN_KEY, VALUE_NAME)
}

pub(crate) fn read_string(root: HKEY, subkey: &str, value: &str) -> Option<String> {
    let sub = wide(subkey);
    let name = wide(value);
    let mut buf = vec![0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    let r = unsafe {
        RegGetValueW(root, sub.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut size)
    };
    if r != ERROR_SUCCESS {
        return None;
    }
    let len = (size as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buf[..len]))
}

pub(crate) fn read_dword(root: HKEY, subkey: &str, value: &str) -> Option<u32> {
    let sub = wide(subkey);
    let name = wide(value);
    let mut out: u32 = 0;
    let mut size = 4u32;
    let r = unsafe {
        RegGetValueW(root, sub.as_ptr(), name.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), (&mut out as *mut u32).cast(), &mut size)
    };
    (r == ERROR_SUCCESS).then_some(out)
}
