use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use windows::Win32::Foundation::{LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, ClipCursor, GetCursorPos, SetWindowsHookExW, UnhookWindowsHookEx,
    HHOOK, MSLLHOOKSTRUCT, WH_MOUSE_LL, WM_MOUSEMOVE,
};

static IS_SLIDING: AtomicBool = AtomicBool::new(false);
static HOOK_HANDLE: AtomicIsize = AtomicIsize::new(0);

pub struct CursorFreezer;

impl CursorFreezer {
    pub fn set_sliding(sliding: bool) {
        let was_sliding = IS_SLIDING.swap(sliding, Ordering::SeqCst);
        if sliding && !was_sliding {
            // Backup ClipCursor to 1x1 rect around current pointer
            unsafe {
                let mut pt = POINT::default();
                if GetCursorPos(&mut pt).is_ok() {
                    let rect = RECT {
                        left: pt.x,
                        top: pt.y,
                        right: pt.x + 1,
                        bottom: pt.y + 1,
                    };
                    let _ = ClipCursor(Some(&rect));
                }
            }
        } else if !sliding && was_sliding {
            // Release cursor clip
            unsafe {
                let _ = ClipCursor(None);
            }
        }
    }

    pub fn is_sliding() -> bool {
        IS_SLIDING.load(Ordering::Relaxed)
    }

    pub fn install_hook() -> bool {
        if HOOK_HANDLE.load(Ordering::SeqCst) != 0 {
            return true;
        }

        unsafe {
            let hook = SetWindowsHookExW(
                WH_MOUSE_LL,
                Some(mouse_ll_proc),
                None,
                0,
            );

            match hook {
                Ok(h) => {
                    HOOK_HANDLE.store(h.0 as isize, Ordering::SeqCst);
                    true
                }
                Err(_) => false,
            }
        }
    }

    pub fn uninstall_hook() {
        let handle = HOOK_HANDLE.swap(0, Ordering::SeqCst);
        if handle != 0 {
            unsafe {
                let _ = UnhookWindowsHookEx(HHOOK(handle as *mut _));
                let _ = ClipCursor(None);
            }
        }
    }
}

unsafe extern "system" fn mouse_ll_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code >= 0 && wparam.0 as u32 == WM_MOUSEMOVE && IS_SLIDING.load(Ordering::Relaxed) {
        if let Some(ms) = (lparam.0 as *const MSLLHOOKSTRUCT).as_ref() {
            // Ignore injected mouse events, only suppress physical mouse move during slide
            if (ms.flags & 1) == 0 {
                return LRESULT(1);
            }
        }
    }

    CallNextHookEx(None, code, wparam, lparam)
}
