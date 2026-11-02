use std::sync::{Arc, Mutex};
use sidewise_core::config::Config;
use sidewise_core::types::{Contact, SliderKind};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateCompatibleDC, CreateDIBSection, CreatePen, CreateSolidBrush,
    DeleteDC, DeleteObject, EndPaint, FillRect, GetMonitorInfoW, MonitorFromPoint,
    SelectObject,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    DT_CENTER, DT_LEFT, DT_RIGHT, DT_SINGLELINE, DT_VCENTER, FW_NORMAL, FW_SEMIBOLD,
    HDC, MONITORINFO, MONITOR_DEFAULTTONEAREST, PAINTSTRUCT, PS_SOLID,
};
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, BringWindowToTop, CreateWindowExW, DefWindowProcW, DestroyWindow,
    GetClientRect, GetCursorPos, RegisterClassExW, SetForegroundWindow, SetWindowPos,
    ShowWindow, CS_HREDRAW, CS_VREDRAW, HICON, HWND_TOPMOST,
    SWP_SHOWWINDOW, SW_SHOWNORMAL, WINDOW_EX_STYLE, WM_DESTROY, WM_LBUTTONDOWN, WM_PAINT,
    WNDCLASSEXW, WS_CAPTION, WS_EX_APPWINDOW, WS_EX_TOPMOST, WS_OVERLAPPED,
    WS_SYSMENU, WS_VISIBLE,
};
use crate::theme::{
    create_ui_font, draw_text, fill_round_rect, get_window_scale,
    COLOR_ACCENT_GREEN, COLOR_ACCENT_TEAL, COLOR_BG, COLOR_BORDER, COLOR_BORDER_CARD,
    COLOR_CARD, COLOR_TEXT_LABEL, COLOR_TEXT_MUTED, COLOR_TEXT_PRIMARY, COLOR_TRACK,
    COLOR_WHITE,
};

/// Brightness accent used by the touchpad diagram (#B07A2E).
const COLOR_BRIGHTNESS: COLORREF = COLORREF(0x002E7AB0);

const BASE_SETUP_WIDTH: i32 = 600;
const BASE_SETUP_HEIGHT: i32 = 560;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SetupStep {
    DetectPad,
    ProbeSlideRight,
    ProbeSlideLeft,
    ProbeTap,
    PracticeVolume,
    PracticeBrightness,
    Complete,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SetupGuideState {
    pub step: SetupStep,
    pub latest_contact: Option<Contact>,
    pub contact_trail: Vec<(f32, f32)>,
    pub pad_detected: bool,
    pub volume_practiced: bool,
    pub brightness_practiced: bool,
    pub current_volume: f32,
    pub current_brightness: f32,
    pub active_slider: Option<SliderKind>,
    pub probe_right_done: bool,
    pub probe_left_done: bool,
    pub probe_tap_done: bool,
    pub probe_min_x: f32,
    pub probe_max_x: f32,
    pub probe_min_y: f32,
    pub probe_max_y: f32,
    pub probe_observed_tip: bool,
    pub diagnostic_alert: Option<String>,
    pub probe_status: Option<String>,
    /// Mirrors the user's "Swap sides" setting so the diagram labels match
    /// what the gesture engine actually does.
    pub swap_sides: bool,
}

impl Default for SetupGuideState {
    fn default() -> Self {
        Self {
            step: SetupStep::DetectPad,
            latest_contact: None,
            contact_trail: Vec::with_capacity(20),
            pad_detected: false,
            volume_practiced: false,
            brightness_practiced: false,
            current_volume: 0.5,
            current_brightness: 0.5,
            active_slider: None,
            probe_right_done: false,
            probe_left_done: false,
            probe_tap_done: false,
            probe_min_x: 1.0,
            probe_max_x: 0.0,
            probe_min_y: 1.0,
            probe_max_y: 0.0,
            probe_observed_tip: false,
            diagnostic_alert: None,
            probe_status: None,
            swap_sides: false,
        }
    }
}

static mut GLOBAL_SETUP_STATE: Option<Arc<Mutex<SetupGuideState>>> = None;
static mut GLOBAL_SETUP_HWND: HWND = HWND(std::ptr::null_mut());

pub struct SetupGuideWindow;

impl SetupGuideWindow {
    pub fn open() {
        let initial = SetupGuideState {
            swap_sides: configured_swap_sides(),
            ..Default::default()
        };
        let state = Arc::new(Mutex::new(initial));

        unsafe {
            GLOBAL_SETUP_STATE = Some(Arc::clone(&state));

            let class_name = w!("SidewiseSetupGuideWindow");
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(setup_wnd_proc),
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
                hbrBackground: CreateSolidBrush(COLOR_BG),
                lpszMenuName: PCWSTR::null(),
                lpszClassName: class_name,
                hIconSm: HICON::default(),
            };
            let _ = RegisterClassExW(&wc);

            // The process is per-monitor DPI aware, so size the client area in
            // physical pixels and let the paint code scale logical units.
            let dpi = GetDpiForSystem().max(96) as f32;
            let scale = dpi / 96.0;
            let client_w = (BASE_SETUP_WIDTH as f32 * scale).round() as i32;
            let client_h = (BASE_SETUP_HEIGHT as f32 * scale).round() as i32;

            let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_VISIBLE;
            let mut frame = RECT {
                left: 0,
                top: 0,
                right: client_w,
                bottom: client_h,
            };
            let _ = AdjustWindowRectEx(&mut frame, style, false, WINDOW_EX_STYLE(0));
            let win_w = frame.right - frame.left;
            let win_h = frame.bottom - frame.top;

            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            let hmon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
            let mut mi = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let (x, y) = if GetMonitorInfoW(hmon, &mut mi).as_bool() {
                (
                    mi.rcWork.left + ((mi.rcWork.right - mi.rcWork.left) - win_w) / 2,
                    mi.rcWork.top + ((mi.rcWork.bottom - mi.rcWork.top) - win_h) / 2,
                )
            } else {
                (80, 80)
            };

            let hwnd = CreateWindowExW(
                WS_EX_APPWINDOW | WS_EX_TOPMOST,
                class_name,
                w!("Sidewise Setup"),
                style,
                x.max(0),
                y.max(0),
                win_w,
                win_h,
                HWND::default(),
                None,
                windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default(),
                None,
            );

            if let Ok(h) = hwnd {
                GLOBAL_SETUP_HWND = h;
                let _ = SetWindowPos(h, HWND_TOPMOST, x, y, win_w, win_h, SWP_SHOWWINDOW);
                let _ = ShowWindow(h, SW_SHOWNORMAL);
                let _ = SetForegroundWindow(h);
                let _ = BringWindowToTop(h);
            }
        }
    }

    pub fn record_contact(contact: Contact) {
        unsafe {
            let mut should_repaint = false;
            if let Some(ref st_lock) = GLOBAL_SETUP_STATE {
                if let Ok(mut st) = st_lock.lock() {
                    st.latest_contact = Some(contact);
                    if contact.tip {
                        st.pad_detected = true;
                        st.contact_trail.push((contact.x, contact.y));
                        if st.contact_trail.len() > 14 {
                            st.contact_trail.remove(0);
                        }

                        // Which physical edge owns volume depends on "Swap sides".
                        let volume_on_right = !st.swap_sides;

                        match st.step {
                            SetupStep::ProbeSlideRight => {
                                st.probe_min_y = st.probe_min_y.min(contact.y);
                                st.probe_max_y = st.probe_max_y.max(contact.y);
                                if contact.x >= 0.70 && (st.probe_max_y - st.probe_min_y) >= 0.12 {
                                    st.probe_right_done = true;
                                    st.step = SetupStep::ProbeSlideLeft;
                                    st.probe_min_y = 1.0;
                                    st.probe_max_y = 0.0;
                                    st.probe_status = Some("Right edge captured. Now slide along the left edge.".into());
                                    st.contact_trail.clear();
                                }
                            }
                            SetupStep::ProbeSlideLeft => {
                                st.probe_min_y = st.probe_min_y.min(contact.y);
                                st.probe_max_y = st.probe_max_y.max(contact.y);
                                if contact.x <= 0.30 && (st.probe_max_y - st.probe_min_y) >= 0.12 {
                                    st.probe_left_done = true;
                                    st.step = SetupStep::ProbeTap;
                                    st.probe_status = Some("Left edge captured. Tap once to finish.".into());
                                    st.contact_trail.clear();
                                }
                            }
                            SetupStep::ProbeTap => {
                                st.probe_tap_done = true;
                                save_probed_profile();
                                st.step = SetupStep::PracticeVolume;
                                st.probe_status = Some("Calibration complete. Profile saved.".into());
                                st.contact_trail.clear();
                            }
                            SetupStep::PracticeVolume => {
                                let on_volume_edge = if volume_on_right {
                                    contact.x >= 0.85
                                } else {
                                    contact.x <= 0.15
                                };
                                if on_volume_edge {
                                    st.volume_practiced = true;
                                }
                            }
                            SetupStep::PracticeBrightness => {
                                let on_brightness_edge = if volume_on_right {
                                    contact.x <= 0.15
                                } else {
                                    contact.x >= 0.85
                                };
                                if on_brightness_edge {
                                    st.brightness_practiced = true;
                                }
                            }
                            _ => {}
                        }
                    } else {
                        st.contact_trail.clear();
                    }
                    should_repaint = true;
                }
            }

            if should_repaint {
                request_repaint();
            }
        }
    }

    pub fn surface_diagnostic_check(msg: &str) {
        unsafe {
            if let Some(ref st_lock) = GLOBAL_SETUP_STATE {
                if let Ok(mut st) = st_lock.lock() {
                    st.diagnostic_alert = Some(msg.to_string());
                }
                request_repaint();
            }
        }
    }

    pub fn start_probe() {
        unsafe {
            if let Some(ref st_lock) = GLOBAL_SETUP_STATE {
                if let Ok(mut st) = st_lock.lock() {
                    st.step = SetupStep::ProbeSlideRight;
                    st.probe_right_done = false;
                    st.probe_left_done = false;
                    st.probe_tap_done = false;
                    st.probe_min_y = 1.0;
                    st.probe_max_y = 0.0;
                    st.diagnostic_alert = None;
                    st.probe_status = Some("Slide your finger firmly along the right edge.".into());
                    st.contact_trail.clear();
                }
                request_repaint();
            }
        }
    }

    pub fn record_glide(kind: SliderKind, value: f32) {
        unsafe {
            if let Some(ref st_lock) = GLOBAL_SETUP_STATE {
                if let Ok(mut st) = st_lock.lock() {
                    match kind {
                        SliderKind::Volume => {
                            st.current_volume = value;
                            st.volume_practiced = true;
                        }
                        SliderKind::Brightness => {
                            st.current_brightness = value;
                            st.brightness_practiced = true;
                        }
                    }
                    st.active_slider = Some(kind);
                }
                request_repaint();
            }
        }
    }

    pub fn record_slide_start(kind: SliderKind) {
        unsafe {
            if let Some(ref st_lock) = GLOBAL_SETUP_STATE {
                if let Ok(mut st) = st_lock.lock() {
                    st.active_slider = Some(kind);
                    match kind {
                        SliderKind::Volume => st.volume_practiced = true,
                        SliderKind::Brightness => st.brightness_practiced = true,
                    }
                }
                request_repaint();
            }
        }
    }

    pub fn record_slide_end(kind: SliderKind) {
        unsafe {
            if let Some(ref st_lock) = GLOBAL_SETUP_STATE {
                if let Ok(mut st) = st_lock.lock() {
                    if st.active_slider == Some(kind) {
                        st.active_slider = None;
                    }
                }
                request_repaint();
            }
        }
    }
}

fn request_repaint() {
    unsafe {
        if !GLOBAL_SETUP_HWND.0.is_null() {
            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(GLOBAL_SETUP_HWND, None, false);
        }
    }
}

fn configured_swap_sides() -> bool {
    let Ok(appdata) = std::env::var("APPDATA") else {
        return false;
    };
    let path = std::path::PathBuf::from(appdata)
        .join("Sidewise")
        .join("config.toml");
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| Config::from_toml(&text).ok())
        .map(|cfg| cfg.swap_sides)
        .unwrap_or(false)
}

unsafe extern "system" fn setup_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_LBUTTONDOWN => {
            let raw_x = (lparam.0 as i16) as i32;
            let raw_y = ((lparam.0 >> 16) as i16) as i32;
            let scale = get_window_scale(hwnd);
            let lx = (raw_x as f32 / scale).round() as i32;
            let ly = (raw_y as f32 / scale).round() as i32;
            handle_setup_click(hwnd, lx, ly);
            LRESULT(0)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            paint_setup(hwnd, hdc);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_DESTROY => {
            GLOBAL_SETUP_HWND = HWND(std::ptr::null_mut());
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// Logical hit areas (scaled at paint time, un-scaled before arriving here).
const BTN_ROW_TOP: i32 = 468;
const BTN_ROW_BOTTOM: i32 = 508;
const SKIP_LEFT: i32 = 40;
const SKIP_RIGHT: i32 = 120;
const MID_LEFT: i32 = 124;
const MID_RIGHT: i32 = 330;
const PRIMARY_LEFT: i32 = 346;
const PRIMARY_RIGHT: i32 = 560;

unsafe fn handle_setup_click(hwnd: HWND, x: i32, y: i32) {
    if let Some(ref st_lock) = GLOBAL_SETUP_STATE {
        if let Ok(mut st) = st_lock.lock() {
            let in_button_row = y >= BTN_ROW_TOP && y <= BTN_ROW_BOTTOM;

            // Skip / Close
            if in_button_row && x >= SKIP_LEFT && x <= SKIP_RIGHT {
                let _ = DestroyWindow(hwnd);
            }
            // Run touchpad check (calibration)
            else if in_button_row && x >= MID_LEFT && x <= MID_RIGHT {
                st.step = SetupStep::ProbeSlideRight;
                st.probe_right_done = false;
                st.probe_left_done = false;
                st.probe_tap_done = false;
                st.probe_min_y = 1.0;
                st.probe_max_y = 0.0;
                st.diagnostic_alert = None;
                st.probe_status = Some("Calibration started. Slide along the right edge.".into());
                st.contact_trail.clear();
                let _ = windows::Win32::Graphics::Gdi::InvalidateRect(hwnd, None, false);
            }
            // Primary action
            else if in_button_row && x >= PRIMARY_LEFT && x <= PRIMARY_RIGHT {
                match st.step {
                    SetupStep::DetectPad => st.step = SetupStep::PracticeVolume,
                    SetupStep::ProbeSlideRight => st.step = SetupStep::ProbeSlideLeft,
                    SetupStep::ProbeSlideLeft => st.step = SetupStep::ProbeTap,
                    SetupStep::ProbeTap => {
                        save_probed_profile();
                        st.step = SetupStep::PracticeVolume;
                    }
                    SetupStep::PracticeVolume => st.step = SetupStep::PracticeBrightness,
                    SetupStep::PracticeBrightness => st.step = SetupStep::Complete,
                    SetupStep::Complete => {
                        let _ = DestroyWindow(hwnd);
                        return;
                    }
                }
                st.contact_trail.clear();
                let _ = windows::Win32::Graphics::Gdi::InvalidateRect(hwnd, None, false);
            }
        }
    }
}

fn save_probed_profile() {
    let dev_info = sidewise_win::get_latest_device_info();
    let (vid, pid, name) = match dev_info {
        Some((id, _)) => (id.vid, id.pid, id.name),
        None => (0x045E, 0x096F, "Calibrated Touchpad".into()),
    };

    let toml = format!(
        r#"# Provisional Touchpad Profile generated by Sidewise Setup Probe
[[profile]]
id = "probed-{:04x}-{:04x}"
source = "probe"

[profile.match]
vid = 0x{:04X}
pid = 0x{:04X}
name = "{}"

[profile.fields.x]
type = "value"
usage_page = 0x01
usage = 0x30

[profile.fields.y]
type = "value"
usage_page = 0x01
usage = 0x31

[profile.fields.contact_id]
type = "value"
usage_page = 0x0D
usage = 0x51

[profile.fields.tip_switch]
type = "button"
usage_page = 0x0D
usage = 0x42

[profile.defaults]
confidence = true
tip_switch = true

[profile.axis]
flip_y = false
flip_x = false

[profile.tuning]
edge_width_scale = 1.0
sensitivity_scale = 1.0
"#,
        vid,
        pid,
        vid,
        pid,
        name.replace('\\', "\\\\")
    );

    let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
    let dir = std::path::PathBuf::from(appdata)
        .join("Sidewise")
        .join("profiles");
    let _ = std::fs::create_dir_all(&dir);
    let out_file = dir.join(format!("probed_{:04x}_{:04x}.toml", vid, pid));
    let _ = std::fs::write(out_file, toml);
}

unsafe fn paint_setup(hwnd: HWND, hdc: HDC) {
    let mut rc = RECT::default();
    let _ = GetClientRect(hwnd, &mut rc);
    let width = rc.right;
    let height = rc.bottom;

    if width <= 0 || height <= 0 {
        return;
    }

    let scale = get_window_scale(hwnd);
    let s = |v: i32| -> i32 { (v as f32 * scale).round() as i32 };

    // Double-buffered 32-bit surface: no flicker while touch frames stream in.
    let mem_dc = CreateCompatibleDC(hdc);
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut u8 = std::ptr::null_mut();
    let hbm = CreateDIBSection(
        hdc,
        &bmi,
        DIB_RGB_COLORS,
        &mut bits as *mut *mut u8 as *mut *mut std::ffi::c_void,
        windows::Win32::Foundation::HANDLE::default(),
        0,
    );
    if hbm.is_err() || bits.is_null() {
        return;
    }
    let hbm = hbm.unwrap();
    let old_bm = SelectObject(mem_dc, hbm);

    let bg_brush = CreateSolidBrush(COLOR_BG);
    FillRect(mem_dc, &rc, bg_brush);
    let _ = DeleteObject(bg_brush);

    // Type scale shared with the flyout/settings windows (IBM Plex Mono).
    let title_font = create_ui_font(17, FW_SEMIBOLD.0 as i32, scale, false);
    let sub_font = create_ui_font(10, FW_NORMAL.0 as i32, scale, false);
    let body_font = create_ui_font(11, FW_NORMAL.0 as i32, scale, false);
    let body_bold = create_ui_font(11, FW_SEMIBOLD.0 as i32, scale, false);
    let label_font = create_ui_font(8, FW_SEMIBOLD.0 as i32, scale, false);
    let mono_font = create_ui_font(9, FW_NORMAL.0 as i32, scale, false);
    let button_font = create_ui_font(10, FW_SEMIBOLD.0 as i32, scale, false);
    let footer_font = create_ui_font(9, FW_NORMAL.0 as i32, scale, false);

    let mut st = SetupGuideState::default();
    if let Some(ref st_lock) = GLOBAL_SETUP_STATE {
        if let Ok(snapshot) = st_lock.lock() {
            st = snapshot.clone();
        }
    }

    let (tag, progress) = match st.step {
        SetupStep::DetectPad => ("STEP 1 OF 3", 1),
        SetupStep::PracticeVolume => ("STEP 2 OF 3", 2),
        SetupStep::PracticeBrightness => ("STEP 3 OF 3", 3),
        SetupStep::Complete => ("COMPLETE", 3),
        SetupStep::ProbeSlideRight => ("CALIBRATION 1 OF 3", 1),
        SetupStep::ProbeSlideLeft => ("CALIBRATION 2 OF 3", 2),
        SetupStep::ProbeTap => ("CALIBRATION 3 OF 3", 3),
    };

    // Header
    let mut title_rc = RECT {
        left: s(40),
        top: s(26),
        right: s(360),
        bottom: s(54),
    };
    draw_text(
        mem_dc,
        "Set up Sidewise",
        &mut title_rc,
        COLOR_TEXT_PRIMARY,
        title_font,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );

    let mut tag_rc = RECT {
        left: s(340),
        top: s(30),
        right: s(560),
        bottom: s(50),
    };
    draw_text(
        mem_dc,
        tag,
        &mut tag_rc,
        COLOR_TEXT_LABEL,
        label_font,
        DT_RIGHT | DT_VCENTER | DT_SINGLELINE,
    );

    let mut sub_rc = RECT {
        left: s(40),
        top: s(54),
        right: s(560),
        bottom: s(72),
    };
    draw_text(
        mem_dc,
        "Three quick steps. You can rerun this from the tray any time.",
        &mut sub_rc,
        COLOR_TEXT_MUTED,
        sub_font,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );

    // Segmented progress indicator
    let seg_top = s(88);
    let seg_bottom = s(92);
    let seg_gap = s(6);
    let seg_left = s(40);
    let seg_total = s(520);
    let seg_w = (seg_total - seg_gap * 2) / 3;
    for i in 0..3 {
        let left = seg_left + i * (seg_w + seg_gap);
        let seg_rc = RECT {
            left,
            top: seg_top,
            right: left + seg_w,
            bottom: seg_bottom,
        };
        let color = if i < progress {
            COLOR_ACCENT_TEAL
        } else {
            COLOR_TRACK
        };
        fill_round_rect(mem_dc, &seg_rc, color, color, s(2));
    }

    // Touchpad card
    let pad_rect = RECT {
        left: s(40),
        top: s(112),
        right: s(560),
        bottom: s(400),
    };
    fill_round_rect(mem_dc, &pad_rect, COLOR_CARD, COLOR_BORDER_CARD, s(18));

    let inset = s(8);
    let zone_w = s(64);
    let left_zone = RECT {
        left: pad_rect.left + inset,
        top: pad_rect.top + inset,
        right: pad_rect.left + inset + zone_w,
        bottom: pad_rect.bottom - inset,
    };
    let right_zone = RECT {
        left: pad_rect.right - inset - zone_w,
        top: pad_rect.top + inset,
        right: pad_rect.right - inset,
        bottom: pad_rect.bottom - inset,
    };
    let zone_h = (left_zone.bottom - left_zone.top).max(1) as f32;

    let (left_val, right_val, left_label, right_label, left_rgb, right_rgb) =
        if st.swap_sides {
            (
                st.current_volume,
                st.current_brightness,
                "VOLUME",
                "BRIGHTNESS",
                (45.0f32, 106.0f32, 95.0f32),
                (176.0f32, 122.0f32, 46.0f32),
            )
        } else {
            (
                st.current_brightness,
                st.current_volume,
                "BRIGHTNESS",
                "VOLUME",
                (176.0f32, 122.0f32, 46.0f32),
                (45.0f32, 106.0f32, 95.0f32),
            )
        };

    let left_active = matches!(st.step, SetupStep::PracticeBrightness | SetupStep::ProbeSlideLeft)
        || (st.swap_sides && st.step == SetupStep::PracticeVolume);
    let right_active = matches!(st.step, SetupStep::PracticeVolume | SetupStep::ProbeSlideRight)
        || (st.swap_sides && st.step == SetupStep::PracticeBrightness)
        || st.step == SetupStep::Complete;

    let left_bar_y = left_zone.bottom - (zone_h * left_val.clamp(0.0, 1.0)).round() as i32;
    let right_bar_y = right_zone.bottom - (zone_h * right_val.clamp(0.0, 1.0)).round() as i32;

    let left_dot_cx = (left_zone.left + left_zone.right) as f32 / 2.0;
    let right_dot_cx = (right_zone.left + right_zone.right) as f32 / 2.0;
    let dot_rad = s(6) as f32;

    // Paint the level fills, level lines, dots and zone separators straight
    // into the DIB so everything stays crisply antialiased.
    {
        let px_start = pad_rect.left.max(0) as usize;
        let px_end = (pad_rect.right as usize).min(width as usize);
        let py_start = pad_rect.top.max(0) as usize;
        let py_end = (pad_rect.bottom as usize).min(height as usize);
        let slice = std::slice::from_raw_parts_mut(bits as *mut u32, (width * height) as usize);

        for py in py_start..py_end {
            for px in px_start..px_end {
                let idx = py * width as usize + px;
                let cur = slice[idx];
                let mut r = ((cur >> 16) & 0xFF) as f32;
                let mut g = ((cur >> 8) & 0xFF) as f32;
                let mut b = (cur & 0xFF) as f32;
                let fx = px as f32;
                let fy = py as f32;

                let in_left = px < left_zone.right as usize;
                let in_right = px >= right_zone.left as usize;

                if in_left {
                    let (ar, ag, ab) = left_rgb;
                    if (py as i32) >= left_bar_y {
                        let alpha = if left_active { 0.20 } else { 0.10 };
                        r = r * (1.0 - alpha) + ar * alpha;
                        g = g * (1.0 - alpha) + ag * alpha;
                        b = b * (1.0 - alpha) + ab * alpha;
                    }
                    let dist = ((py as i32) - left_bar_y).abs();
                    if dist <= s(1) {
                        let a = 0.9;
                        r = r * (1.0 - a) + ar * a;
                        g = g * (1.0 - a) + ag * a;
                        b = b * (1.0 - a) + ab * a;
                    } else if dist <= s(5) {
                        let a = (1.0 - dist as f32 / s(5) as f32) * 0.22;
                        r = r * (1.0 - a) + ar * a;
                        g = g * (1.0 - a) + ag * a;
                        b = b * (1.0 - a) + ab * a;
                    }
                }

                if in_right {
                    let (ar, ag, ab) = right_rgb;
                    if (py as i32) >= right_bar_y {
                        let alpha = if right_active { 0.20 } else { 0.10 };
                        r = r * (1.0 - alpha) + ar * alpha;
                        g = g * (1.0 - alpha) + ag * alpha;
                        b = b * (1.0 - alpha) + ab * alpha;
                    }
                    let dist = ((py as i32) - right_bar_y).abs();
                    if dist <= s(1) {
                        let a = 0.9;
                        r = r * (1.0 - a) + ar * a;
                        g = g * (1.0 - a) + ag * a;
                        b = b * (1.0 - a) + ab * a;
                    } else if dist <= s(5) {
                        let a = (1.0 - dist as f32 / s(5) as f32) * 0.22;
                        r = r * (1.0 - a) + ar * a;
                        g = g * (1.0 - a) + ag * a;
                        b = b * (1.0 - a) + ab * a;
                    }
                }

                // Level dots with a soft halo, same language as the flyout.
                for (dot_cx, bar_y, rgb) in [
                    (left_dot_cx, left_bar_y, left_rgb),
                    (right_dot_cx, right_bar_y, right_rgb),
                ] {
                    let dist = ((fx - dot_cx).powi(2) + (fy - bar_y as f32).powi(2)).sqrt();
                    if dist <= dot_rad {
                        let a = (dot_rad - dist).clamp(0.0, 1.0);
                        r = r * (1.0 - a) + rgb.0 * a;
                        g = g * (1.0 - a) + rgb.1 * a;
                        b = b * (1.0 - a) + rgb.2 * a;
                    } else if dist <= dot_rad + s(4) as f32 {
                        let a = (1.0 - (dist - dot_rad) / s(4) as f32) * 0.22;
                        r = r * (1.0 - a) + rgb.0 * a;
                        g = g * (1.0 - a) + rgb.1 * a;
                        b = b * (1.0 - a) + rgb.2 * a;
                    }
                }

                // Separators between the edge zones and the centre surface.
                if px == left_zone.right as usize || px == right_zone.left as usize {
                    r *= 0.90;
                    g *= 0.90;
                    b *= 0.90;
                }

                slice[idx] = (cur & 0xFF00_0000)
                    | ((r.round() as u32) << 16)
                    | ((g.round() as u32) << 8)
                    | (b.round() as u32);
            }
        }
    }

    // Percentage readouts inside each zone.
    let mut left_pct_rc = RECT {
        left: left_zone.left,
        top: left_zone.top + s(8),
        right: left_zone.right,
        bottom: left_zone.top + s(26),
    };
    let left_pct = format!("{}%", (left_val * 100.0).round() as u32);
    draw_text(
        mem_dc,
        &left_pct,
        &mut left_pct_rc,
        COLOR_TEXT_PRIMARY,
        mono_font,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );

    let mut right_pct_rc = RECT {
        left: right_zone.left,
        top: right_zone.top + s(8),
        right: right_zone.right,
        bottom: right_zone.top + s(26),
    };
    let right_pct = format!("{}%", (right_val * 100.0).round() as u32);
    draw_text(
        mem_dc,
        &right_pct,
        &mut right_pct_rc,
        COLOR_TEXT_PRIMARY,
        mono_font,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );

    // Centre prompt for the current step.
    let volume_side = if st.swap_sides { "left" } else { "right" };
    let brightness_side = if st.swap_sides { "right" } else { "left" };
    let (prompt, hint, prompt_color) = match st.step {
        SetupStep::DetectPad => {
            if st.pad_detected {
                (
                    "Touchpad detected",
                    "Live contact is coming through.",
                    COLOR_ACCENT_GREEN,
                )
            } else {
                (
                    "Touch your touchpad",
                    "A dot will follow your finger.",
                    COLOR_TEXT_MUTED,
                )
            }
        }
        SetupStep::ProbeSlideRight => (
            "Slide the right edge",
            "Glide firmly up and down.",
            COLOR_TEXT_PRIMARY,
        ),
        SetupStep::ProbeSlideLeft => (
            "Slide the left edge",
            "Glide firmly up and down.",
            COLOR_TEXT_PRIMARY,
        ),
        SetupStep::ProbeTap => (
            "Tap the touchpad",
            "Anywhere on the surface.",
            COLOR_TEXT_PRIMARY,
        ),
        SetupStep::PracticeVolume => {
            if st.volume_practiced {
                (
                    "Volume gesture is working",
                    "Keep gliding to adjust it.",
                    COLOR_ACCENT_GREEN,
                )
            } else {
                (
                    "Glide the volume edge",
                    match volume_side {
                        "left" => "Up and down on the left side.",
                        _ => "Up and down on the right side.",
                    },
                    COLOR_TEXT_PRIMARY,
                )
            }
        }
        SetupStep::PracticeBrightness => {
            if st.brightness_practiced {
                (
                    "Brightness gesture is working",
                    "Keep gliding to adjust it.",
                    COLOR_ACCENT_GREEN,
                )
            } else {
                (
                    "Glide the brightness edge",
                    match brightness_side {
                        "left" => "Up and down on the left side.",
                        _ => "Up and down on the right side.",
                    },
                    COLOR_TEXT_PRIMARY,
                )
            }
        }
        SetupStep::Complete => (
            "You're all set",
            "Both edge gestures are ready to use.",
            COLOR_ACCENT_GREEN,
        ),
    };

    let center_left = left_zone.right + s(16);
    let center_right = right_zone.left - s(16);
    let mut prompt_rc = RECT {
        left: center_left,
        top: pad_rect.top + s(96),
        right: center_right,
        bottom: pad_rect.top + s(128),
    };
    draw_text(
        mem_dc,
        prompt,
        &mut prompt_rc,
        prompt_color,
        body_bold,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );

    let mut hint_rc = RECT {
        left: center_left,
        top: pad_rect.top + s(132),
        right: center_right,
        bottom: pad_rect.top + s(152),
    };
    draw_text(
        mem_dc,
        hint,
        &mut hint_rc,
        COLOR_TEXT_MUTED,
        sub_font,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );

    // Contact trail (fading dots) and live lead contact.
    let pad_w = (pad_rect.right - pad_rect.left) as f32;
    let pad_h = (pad_rect.bottom - pad_rect.top) as f32;
    let trail_len = st.contact_trail.len();
    for (idx, &(tx, ty)) in st.contact_trail.iter().enumerate() {
        let cx = pad_rect.left + (tx * pad_w) as i32;
        let cy = pad_rect.top + (ty * pad_h) as i32;
        let radius = 2 + (idx as i32 * 4) / (trail_len.max(1) as i32);
        let t_brush = CreateSolidBrush(COLOR_ACCENT_TEAL);
        let old_b = SelectObject(mem_dc, t_brush);
        let _ = windows::Win32::Graphics::Gdi::RoundRect(
            mem_dc,
            cx - radius,
            cy - radius,
            cx + radius,
            cy + radius,
            radius * 2,
            radius * 2,
        );
        let _ = SelectObject(mem_dc, old_b);
        let _ = DeleteObject(t_brush);
    }

    if let Some(contact) = st.latest_contact {
        if contact.tip {
            let cx = pad_rect.left + (contact.x * pad_w) as i32;
            let cy = pad_rect.top + (contact.y * pad_h) as i32;

            let ring_pen = CreatePen(PS_SOLID, s(2).max(1), COLOR_TEXT_PRIMARY);
            let null_brush =
                windows::Win32::Graphics::Gdi::GetStockObject(windows::Win32::Graphics::Gdi::NULL_BRUSH);
            let old_pen = SelectObject(mem_dc, ring_pen);
            let old_b = SelectObject(mem_dc, null_brush);
            let _ = windows::Win32::Graphics::Gdi::Ellipse(
                mem_dc,
                cx - s(13),
                cy - s(13),
                cx + s(13),
                cy + s(13),
            );
            let _ = SelectObject(mem_dc, old_b);
            let _ = SelectObject(mem_dc, old_pen);
            let _ = DeleteObject(ring_pen);

            let dot_color = if contact.x >= 0.85 {
                if st.swap_sides { COLOR_BRIGHTNESS } else { COLOR_ACCENT_TEAL }
            } else if contact.x <= 0.15 {
                if st.swap_sides { COLOR_ACCENT_TEAL } else { COLOR_BRIGHTNESS }
            } else {
                COLOR_TEXT_PRIMARY
            };
            let dot_brush = CreateSolidBrush(dot_color);
            let old_b = SelectObject(mem_dc, dot_brush);
            let _ = windows::Win32::Graphics::Gdi::Ellipse(
                mem_dc,
                cx - s(6),
                cy - s(6),
                cx + s(6),
                cy + s(6),
            );
            let _ = SelectObject(mem_dc, old_b);
            let _ = DeleteObject(dot_brush);
        }
    }

    // Zone labels below the card.
    let mut left_label_rc = RECT {
        left: s(40),
        top: s(404),
        right: s(250),
        bottom: s(420),
    };
    draw_text(
        mem_dc,
        left_label,
        &mut left_label_rc,
        COLOR_TEXT_LABEL,
        label_font,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );

    let mut right_label_rc = RECT {
        left: s(350),
        top: s(404),
        right: s(560),
        bottom: s(420),
    };
    draw_text(
        mem_dc,
        right_label,
        &mut right_label_rc,
        COLOR_TEXT_LABEL,
        label_font,
        DT_RIGHT | DT_VCENTER | DT_SINGLELINE,
    );

    // Live status line.
    let (status_text, status_color) = if let Some(ref alert) = st.diagnostic_alert {
        (alert.as_str(), COLOR_BRIGHTNESS)
    } else if let Some(ref status) = st.probe_status {
        (status.as_str(), COLOR_ACCENT_TEAL)
    } else {
        match st.step {
            SetupStep::DetectPad => {
                if st.pad_detected {
                    (
                        "Precision touchpad is active and delivering frames.",
                        COLOR_ACCENT_GREEN,
                    )
                } else {
                    ("Waiting for your first touch...", COLOR_TEXT_MUTED)
                }
            }
            SetupStep::ProbeSlideRight => (
                "Slide along the right edge to calibrate range and tip switch.",
                COLOR_TEXT_MUTED,
            ),
            SetupStep::ProbeSlideLeft => (
                "Slide along the left edge to calibrate range and tip switch.",
                COLOR_TEXT_MUTED,
            ),
            SetupStep::ProbeTap => (
                "Tap once on the touchpad to verify tip switch.",
                COLOR_TEXT_MUTED,
            ),
            SetupStep::PracticeVolume => {
                if st.volume_practiced {
                    ("Volume glide detected and working.", COLOR_ACCENT_GREEN)
                } else {
                    ("Start on the volume edge and glide up or down.", COLOR_TEXT_MUTED)
                }
            }
            SetupStep::PracticeBrightness => {
                if st.brightness_practiced {
                    ("Brightness glide detected and working.", COLOR_ACCENT_GREEN)
                } else {
                    (
                        "Start on the brightness edge and glide up or down.",
                        COLOR_TEXT_MUTED,
                    )
                }
            }
            SetupStep::Complete => (
                "Setup complete. Volume and brightness edges are ready.",
                COLOR_ACCENT_GREEN,
            ),
        }
    };

    let mut status_rc = RECT {
        left: s(40),
        top: s(424),
        right: s(560),
        bottom: s(446),
    };
    draw_text(
        mem_dc,
        status_text,
        &mut status_rc,
        status_color,
        sub_font,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );

    // Buttons
    let btn_top = s(BTN_ROW_TOP);
    let btn_bottom = s(BTN_ROW_BOTTOM);

    let mut skip_rc = RECT {
        left: s(SKIP_LEFT),
        top: btn_top,
        right: s(SKIP_RIGHT),
        bottom: btn_bottom,
    };
    let skip_text = if st.step == SetupStep::Complete {
        "Close"
    } else {
        "Skip"
    };
    draw_text(
        mem_dc,
        skip_text,
        &mut skip_rc,
        COLOR_TEXT_MUTED,
        body_font,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );

    let mid_rc = RECT {
        left: s(MID_LEFT),
        top: btn_top,
        right: s(MID_RIGHT),
        bottom: btn_bottom,
    };
    let mid_border = if st.diagnostic_alert.is_some() {
        COLOR_BRIGHTNESS
    } else {
        COLOR_BORDER
    };
    fill_round_rect(mem_dc, &mid_rc, COLOR_WHITE, mid_border, s(10));
    let mut mid_text_rc = mid_rc;
    draw_text(
        mem_dc,
        "Run touchpad check",
        &mut mid_text_rc,
        COLOR_TEXT_PRIMARY,
        button_font,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );

    let primary_rc = RECT {
        left: s(PRIMARY_LEFT),
        top: btn_top,
        right: s(PRIMARY_RIGHT),
        bottom: btn_bottom,
    };
    let primary_text = match st.step {
        SetupStep::DetectPad => "Next: Volume",
        SetupStep::ProbeSlideRight => "Skip to Left",
        SetupStep::ProbeSlideLeft => "Skip to Tap",
        SetupStep::ProbeTap => "Save Profile",
        SetupStep::PracticeVolume => "Next: Brightness",
        SetupStep::PracticeBrightness => "Finish",
        SetupStep::Complete => "Done",
    };
    let primary_bg = match st.step {
        SetupStep::DetectPad if st.pad_detected => COLOR_ACCENT_GREEN,
        SetupStep::PracticeVolume if st.volume_practiced => COLOR_ACCENT_GREEN,
        SetupStep::PracticeBrightness if st.brightness_practiced => COLOR_ACCENT_GREEN,
        SetupStep::Complete => COLOR_ACCENT_GREEN,
        _ => COLOR_ACCENT_TEAL,
    };
    fill_round_rect(mem_dc, &primary_rc, primary_bg, primary_bg, s(10));
    let mut primary_text_rc = primary_rc;
    draw_text(
        mem_dc,
        primary_text,
        &mut primary_text_rc,
        COLOR_WHITE,
        button_font,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );

    // Footer
    let mut footer_rc = RECT {
        left: s(40),
        top: s(522),
        right: s(560),
        bottom: s(542),
    };
    draw_text(
        mem_dc,
        "Sidewise 1.0 \u{00B7} Calibration is optional.",
        &mut footer_rc,
        COLOR_TEXT_MUTED,
        footer_font,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );

    let _ = windows::Win32::Graphics::Gdi::BitBlt(
        hdc,
        0,
        0,
        width,
        height,
        mem_dc,
        0,
        0,
        windows::Win32::Graphics::Gdi::SRCCOPY,
    );

    let _ = SelectObject(mem_dc, old_bm);
    let _ = DeleteObject(hbm);
    let _ = DeleteDC(mem_dc);
    let _ = DeleteObject(title_font);
    let _ = DeleteObject(sub_font);
    let _ = DeleteObject(body_font);
    let _ = DeleteObject(body_bold);
    let _ = DeleteObject(label_font);
    let _ = DeleteObject(mono_font);
    let _ = DeleteObject(button_font);
    let _ = DeleteObject(footer_font);
}
