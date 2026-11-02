use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Threading::{CreateEventW, CreateMutexW, OpenEventW, SetEvent, EVENT_MODIFY_STATE};

const MUTEX_NAME: &str = "Local\\Sidewise.Singleton";
const EVENT_NAME: &str = "Local\\Sidewise.ShowFlyout";

fn to_wide_null(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

pub struct SingleInstance {
    mutex: HANDLE,
    event: HANDLE,
}

impl SingleInstance {
    pub fn try_acquire() -> Option<Self> {
        unsafe {
            let wide_mutex = to_wide_null(MUTEX_NAME);
            let mutex = match CreateMutexW(None, true, PCWSTR(wide_mutex.as_ptr())) {
                Ok(m) => m,
                Err(_) => return None,
            };

            if GetLastError() == ERROR_ALREADY_EXISTS {
                let _ = CloseHandle(mutex);
                return None;
            }

            let wide_event = to_wide_null(EVENT_NAME);
            let event = CreateEventW(None, false, false, PCWSTR(wide_event.as_ptr())).unwrap_or_default();

            Some(Self { mutex, event })
        }
    }

    pub fn signal_existing() {
        unsafe {
            let wide_event = to_wide_null(EVENT_NAME);
            if let Ok(event) = OpenEventW(EVENT_MODIFY_STATE, false, PCWSTR(wide_event.as_ptr())) {
                let _ = SetEvent(event);
                let _ = CloseHandle(event);
            }
        }
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        unsafe {
            if !self.mutex.is_invalid() {
                let _ = CloseHandle(self.mutex);
            }
            if !self.event.is_invalid() {
                let _ = CloseHandle(self.event);
            }
        }
    }
}
