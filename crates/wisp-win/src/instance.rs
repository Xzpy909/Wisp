//! One running copy of Wisp. A second launch signals the first to open the
//! popup, then exits.

use crate::wide;
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::{
    CreateEventW, CreateMutexW, OpenEventW, SetEvent, WaitForSingleObject, EVENT_MODIFY_STATE, INFINITE,
};

const MUTEX_NAME: &str = "Local\\Wisp.SingleInstance";
const EVENT_NAME: &str = "Local\\Wisp.ShowPopup";

/// Held by the primary instance for its whole lifetime.
pub struct SingleInstance {
    mutex: usize,
    event: usize,
}

pub enum Instance {
    Primary(SingleInstance),
    /// Another Wisp is running and has been asked to open its popup.
    Secondary,
}

pub fn acquire() -> Instance {
    let name = wide(MUTEX_NAME);
    // SAFETY: valid null-terminated name; null security attributes.
    let mutex = unsafe { CreateMutexW(std::ptr::null(), 1, name.as_ptr()) };
    let already = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    if already {
        unsafe { CloseHandle(mutex) };
        signal_primary();
        return Instance::Secondary;
    }
    let ev = wide(EVENT_NAME);
    // Auto-reset event, initially unsignalled.
    let event = unsafe { CreateEventW(std::ptr::null(), 0, 0, ev.as_ptr()) };
    Instance::Primary(SingleInstance { mutex: mutex as usize, event: event as usize })
}

fn signal_primary() {
    let ev = wide(EVENT_NAME);
    unsafe {
        let h = OpenEventW(EVENT_MODIFY_STATE, 0, ev.as_ptr());
        if !h.is_null() {
            SetEvent(h);
            CloseHandle(h);
        }
    }
}

impl SingleInstance {
    /// Call `f` on a background thread each time another launch signals us.
    pub fn on_activate(&self, f: impl Fn() + Send + 'static) {
        let event = self.event;
        if event == 0 {
            return;
        }
        std::thread::Builder::new()
            .name("wisp-activate".into())
            .spawn(move || loop {
                // SAFETY: the handle lives as long as the process (see Drop note).
                let r = unsafe { WaitForSingleObject(event as HANDLE, INFINITE) };
                if r != WAIT_OBJECT_0 {
                    break;
                }
                f();
            })
            .ok();
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        // The event handle stays open for the waiting thread; the OS closes it
        // at process exit. Releasing the mutex is what matters for relaunch.
        unsafe { CloseHandle(self.mutex as HANDLE) };
    }
}
