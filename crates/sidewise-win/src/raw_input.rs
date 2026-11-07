use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};
use crossbeam_channel::{bounded, Receiver, Sender};
use sidewise_core::profile::{DeviceIdentity, DeviceProfile};
use sidewise_core::types::{Modifiers, TouchFrame};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VK_CONTROL, VK_MENU, VK_SHIFT,
};
use windows::Win32::UI::Input::{
    GetRawInputData, RegisterRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE,
    RAWINPUTHEADER, RIDEV_INPUTSINK, RID_INPUT, RIM_TYPEHID,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    RegisterClassExW, TranslateMessage, CS_HREDRAW, CS_VREDRAW, HICON, MSG,
    WNDCLASSEXW, WM_INPUT, HWND_MESSAGE, WINDOW_STYLE,
};
use crate::hid::HidParser;
use crate::mouse_hook::CursorFreezer;

pub static LATEST_ACTIVE_DEVICE: RwLock<Option<(DeviceIdentity, DeviceProfile)>> = RwLock::new(None);

pub fn get_latest_device_info() -> Option<(DeviceIdentity, DeviceProfile)> {
    LATEST_ACTIVE_DEVICE.read().ok().and_then(|guard| guard.clone())
}

pub struct RawInputListener {
    frame_rx: Receiver<(TouchFrame, Modifiers)>,
    running: Arc<AtomicBool>,
}

impl RawInputListener {
    pub fn new() -> (Self, Receiver<(TouchFrame, Modifiers)>) {
        let (tx, rx) = bounded(64);
        let running = Arc::new(AtomicBool::new(true));

        let running_clone = Arc::clone(&running);
        let tx_clone = tx.clone();

        thread::Builder::new()
            .name("raw-input-thread".into())
            .spawn(move || {
                run_input_thread(tx_clone, running_clone);
            })
            .expect("Failed to start raw input thread");

        (
            Self {
                frame_rx: rx.clone(),
                running,
            },
            rx,
        )
    }

    pub fn receiver(&self) -> &Receiver<(TouchFrame, Modifiers)> {
        &self.frame_rx
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }
}

fn query_current_modifiers() -> Modifiers {
    unsafe {
        let alt = (GetAsyncKeyState(VK_MENU.0 as i32) as u16 & 0x8000) != 0;
        let ctrl = (GetAsyncKeyState(VK_CONTROL.0 as i32) as u16 & 0x8000) != 0;
        let shift = (GetAsyncKeyState(VK_SHIFT.0 as i32) as u16 & 0x8000) != 0;
        Modifiers { alt, ctrl, shift }
    }
}

fn run_input_thread(
    frame_tx: Sender<(TouchFrame, Modifiers)>,
    running: Arc<AtomicBool>,
) {
    unsafe {
        // Install mouse hook on this message pump thread for Cursor Freeze
        let _ = CursorFreezer::install_hook();

        let class_name = w!("SidewiseRawInputWindow");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(input_wnd_proc),
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
            w!("Sidewise Hidden Input"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            None,
            windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default(),
            None,
        );

        let hwnd = match hwnd {
            Ok(h) if !h.0.is_null() => h,
            _ => return,
        };

        // Register for Precision Touchpad: UsagePage 0x0D (Digitizer), Usage 0x05 (Touch Pad)
        let rid = RAWINPUTDEVICE {
            usUsagePage: 0x0D,
            usUsage: 0x05,
            dwFlags: RIDEV_INPUTSINK,
            hwndTarget: hwnd,
        };

        let _ = RegisterRawInputDevices(&[rid], std::mem::size_of::<RAWINPUTDEVICE>() as u32);

        let mut parser = HidParser::new();
        let mut raw_buf = vec![0u8; 1024];

        let mut msg = MSG::default();
        loop {
            if !running.load(Ordering::Relaxed) {
                break;
            }
            let res = GetMessageW(&mut msg, None, 0, 0);
            if res.0 <= 0 {
                break;
            }
            if msg.message == WM_INPUT {
                let hraw = HRAWINPUT(msg.lParam.0 as *mut _);
                let mut size = 0u32;
                let _ = GetRawInputData(
                    hraw,
                    RID_INPUT,
                    None,
                    &mut size,
                    std::mem::size_of::<RAWINPUTHEADER>() as u32,
                );

                if size as usize > raw_buf.len() {
                    raw_buf.resize(size as usize, 0);
                }

                if size > 0 {
                    let copied = GetRawInputData(
                        hraw,
                        RID_INPUT,
                        Some(raw_buf.as_mut_ptr() as *mut _),
                        &mut size,
                        std::mem::size_of::<RAWINPUTHEADER>() as u32,
                    );

                    if copied != u32::MAX {
                        let raw = &*(raw_buf.as_ptr() as *const RAWINPUT);
                        if raw.header.dwType == RIM_TYPEHID.0 {
                            let hdev = raw.header.hDevice;
                            let hid_report_data = std::slice::from_raw_parts(
                                raw_buf.as_ptr().add(std::mem::size_of::<RAWINPUTHEADER>() + 8),
                                (raw.data.hid.dwSizeHid * raw.data.hid.dwCount) as usize,
                            );

                            let t_ms = SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis() as u64;

                            if let Some(frame) = parser.parse_report(hdev, hid_report_data, t_ms) {
                                if let Some(info) = parser.get_device_info(hdev) {
                                    if let Ok(mut lock) = LATEST_ACTIVE_DEVICE.write() {
                                        *lock = Some((info.identity.clone(), info.profile.clone()));
                                    }
                                }
                                let mods = query_current_modifiers();
                                let _ = frame_tx.try_send((frame, mods));
                            }
                        }
                    }
                }
            }

            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        CursorFreezer::uninstall_hook();
        let _ = DestroyWindow(hwnd);
    }
}

unsafe extern "system" fn input_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    DefWindowProcW(hwnd, msg, wparam, lparam)
}
