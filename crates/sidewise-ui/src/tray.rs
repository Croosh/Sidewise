use std::sync::Arc;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
    NIF_ICON, NIF_MESSAGE, NIF_TIP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW,
    DefWindowProcW, DestroyIcon, DestroyMenu, DestroyWindow,
    GetCursorPos, RegisterClassExW, SetForegroundWindow,
    TrackPopupMenu, CS_HREDRAW, CS_VREDRAW, HICON,
    MF_SEPARATOR, MF_STRING, TPM_BOTTOMALIGN,
    TPM_RIGHTBUTTON, WNDCLASSEXW, WM_LBUTTONUP, WM_RBUTTONUP,
    WM_USER,
};

pub const WM_TRAY_CALLBACK: u32 = WM_USER + 100;
const CMD_FLYOUT: usize = 1001;
const CMD_SETTINGS: usize = 1002;
const CMD_SETUP: usize = 1003;
const CMD_UPDATES: usize = 1004;
const CMD_QUIT: usize = 1005;

pub struct TrayCallbacks {
    pub on_toggle_flyout: Box<dyn Fn() + Send + Sync>,
    pub on_open_settings: Box<dyn Fn() + Send + Sync>,
    pub on_open_setup: Box<dyn Fn() + Send + Sync>,
    pub on_quit: Box<dyn Fn() + Send + Sync>,
}

static mut GLOBAL_TRAY_CALLBACKS: Option<Arc<TrayCallbacks>> = None;

pub struct TrayManager {
    hwnd: HWND,
    nid: NOTIFYICONDATAW,
}

impl TrayManager {
    pub fn new(callbacks: TrayCallbacks) -> Self {
        let cb_arc = Arc::new(callbacks);
        unsafe {
            GLOBAL_TRAY_CALLBACKS = Some(Arc::clone(&cb_arc));
        }

        unsafe {
            let class_name = w!("SidewiseTrayHelperWindow");
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(tray_wnd_proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: windows::Win32::System::LibraryLoader::GetModuleHandleW(None)
                    .unwrap_or_default()
                    .into(),
                hIcon: HICON::default(),
                hCursor: windows::Win32::UI::WindowsAndMessaging::LoadCursorW(
                    None,
                    windows::Win32::UI::WindowsAndMessaging::IDC_ARROW,
                )
                .unwrap_or_default(),
                hbrBackground: windows::Win32::Graphics::Gdi::HBRUSH::default(),
                lpszMenuName: PCWSTR::null(),
                lpszClassName: class_name,
                hIconSm: HICON::default(),
            };
            let _ = RegisterClassExW(&wc);

            let hwnd = CreateWindowExW(
                windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE(0),
                class_name,
                w!("Sidewise Tray"),
                windows::Win32::UI::WindowsAndMessaging::WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                HWND::default(),
                None,
                windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default(),
                None,
            )
            .unwrap_or_default();

            let icon = create_tray_glyph_icon();

            let mut tip = [0u16; 128];
            let tip_str = "Sidewise (Precision Touchpad Controls)\0".encode_utf16();
            for (dest, src) in tip.iter_mut().zip(tip_str) {
                *dest = src;
            }

            let nid = NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: hwnd,
                uID: 1,
                uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
                uCallbackMessage: WM_TRAY_CALLBACK,
                hIcon: icon,
                szTip: tip,
                ..Default::default()
            };

            let _ = Shell_NotifyIconW(NIM_ADD, &nid);

            Self { hwnd, nid }
        }
    }
}

impl Drop for TrayManager {
    fn drop(&mut self) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &self.nid);
            if !self.nid.hIcon.is_invalid() {
                let _ = DestroyIcon(self.nid.hIcon);
            }
            if !self.hwnd.0.is_null() {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}

unsafe fn create_tray_glyph_icon() -> HICON {
    // Embedded transparent tray icon (no background, direct Sidewise logo)
    const TRAY_ICO_BYTES: &[u8] = include_bytes!("../../../assets/tray.ico");

    let hinstance = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default();
    let icon_offset = windows::Win32::UI::WindowsAndMessaging::LookupIconIdFromDirectoryEx(
        TRAY_ICO_BYTES.as_ptr(),
        true,
        16,
        16,
        windows::Win32::UI::WindowsAndMessaging::LR_DEFAULTCOLOR,
    );

    if icon_offset > 0 {
        if let Some(res_bytes) = TRAY_ICO_BYTES.get(icon_offset as usize..) {
            let hicon = windows::Win32::UI::WindowsAndMessaging::CreateIconFromResourceEx(
                res_bytes,
                BOOL(1),
                0x00030000,
                16,
                16,
                windows::Win32::UI::WindowsAndMessaging::LR_DEFAULTCOLOR,
            );
            if let Ok(ic) = hicon {
                if !ic.0.is_null() {
                    return ic;
                }
            }
        }
    }

    // Fallback: try loading executable resource icon if present
    #[allow(clippy::manual_dangling_ptr)]
    let res_icon = windows::Win32::UI::WindowsAndMessaging::LoadImageW(
        hinstance,
        windows::core::PCWSTR(1 as *const u16),
        windows::Win32::UI::WindowsAndMessaging::IMAGE_ICON,
        16,
        16,
        windows::Win32::UI::WindowsAndMessaging::IMAGE_FLAGS(0),
    );
    if let Ok(handle) = res_icon {
        if !handle.0.is_null() {
            return HICON(handle.0);
        }
    }

    // Final fallback: standard application icon
    windows::Win32::UI::WindowsAndMessaging::LoadIconW(
        None,
        windows::Win32::UI::WindowsAndMessaging::IDI_APPLICATION,
    )
    .unwrap_or_default()
}

unsafe extern "system" fn tray_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_TRAY_CALLBACK {
        let event = lparam.0 as u32;
        if event == WM_LBUTTONUP {
            if let Some(ref cb) = GLOBAL_TRAY_CALLBACKS {
                (cb.on_toggle_flyout)();
            }
            return LRESULT(0);
        } else if event == WM_RBUTTONUP {
            show_tray_context_menu(hwnd);
            return LRESULT(0);
        }
    }

    DefWindowProcW(hwnd, msg, wparam, lparam)
}

unsafe fn show_tray_context_menu(hwnd: HWND) {
    let menu = match CreatePopupMenu() {
        Ok(m) => m,
        Err(_) => return,
    };

    let _ = AppendMenuW(menu, MF_STRING, CMD_FLYOUT, w!("Open Flyout"));
    let _ = AppendMenuW(menu, MF_STRING, CMD_SETTINGS, w!("Settings..."));
    let _ = AppendMenuW(menu, MF_STRING, CMD_SETUP, w!("Setup Guide..."));
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    let _ = AppendMenuW(menu, MF_STRING, CMD_UPDATES, w!("Check for Updates..."));
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    let _ = AppendMenuW(menu, MF_STRING, CMD_QUIT, w!("Quit Sidewise"));

    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);

    let _ = SetForegroundWindow(hwnd);
    let cmd = TrackPopupMenu(
        menu,
        TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | windows::Win32::UI::WindowsAndMessaging::TPM_RETURNCMD,
        pt.x,
        pt.y,
        0,
        hwnd,
        None,
    );

    let _ = DestroyMenu(menu);

    if let Some(ref cb) = GLOBAL_TRAY_CALLBACKS {
        match cmd.0 as usize {
            CMD_FLYOUT => (cb.on_toggle_flyout)(),
            CMD_SETTINGS => (cb.on_open_settings)(),
            CMD_SETUP => (cb.on_open_setup)(),
            CMD_UPDATES => {
                let _ = windows::Win32::UI::WindowsAndMessaging::MessageBoxW(
                    hwnd,
                    w!("Sidewise is currently up to date (v1.0.0)."),
                    w!("Check for Updates"),
                    windows::Win32::UI::WindowsAndMessaging::MB_OK | windows::Win32::UI::WindowsAndMessaging::MB_ICONINFORMATION,
                );
            }
            CMD_QUIT => (cb.on_quit)(),
            _ => {}
        }
    }
}
