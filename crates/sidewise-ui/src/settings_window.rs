use std::sync::{Arc, Mutex};
use sidewise_core::config::Config;
use sidewise_core::types::ModifierRequirement;
use sidewise_win::StartupManager;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateCompatibleDC, CreateDIBSection, CreatePen, CreateSolidBrush,
    DeleteDC, DeleteObject, EndPaint, FillRect, MoveToEx, LineTo, SelectObject,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    DT_CENTER, DT_LEFT, DT_SINGLELINE, DT_VCENTER, FW_NORMAL, FW_SEMIBOLD,
    HDC, PAINTSTRUCT, PS_SOLID,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, RegisterClassExW, ShowWindow,
    CS_HREDRAW, CS_VREDRAW, HICON,
    SW_SHOW, WM_DESTROY, WM_LBUTTONDOWN, WM_PAINT, WNDCLASSEXW, WS_CAPTION,
    WS_OVERLAPPED, WS_SYSMENU, WS_VISIBLE,
};
use crate::theme::{
    create_ui_font, draw_segmented_control, draw_switch, draw_text,
    fill_round_rect, get_window_scale, COLOR_ACCENT_GREEN, COLOR_BG, COLOR_BORDER,
    COLOR_CARD, COLOR_TEXT_LABEL, COLOR_TEXT_MUTED, COLOR_TEXT_PRIMARY,
};

type SettingsCallback = Arc<Box<dyn Fn(Config) + Send + Sync>>;
static mut GLOBAL_SETTINGS_CONFIG: Option<Arc<Mutex<Config>>> = None;
static mut GLOBAL_SETTINGS_CALLBACK: Option<SettingsCallback> = None;
static mut GLOBAL_DIAGNOSTICS_MSG: Option<String> = None;

const BASE_SETTINGS_CLIENT_WIDTH: i32 = 640;
const BASE_SETTINGS_CLIENT_HEIGHT: i32 = 620;

pub struct SettingsWindow;

impl SettingsWindow {
    pub fn open(
        config: Config,
        on_change: impl Fn(Config) + Send + Sync + 'static,
    ) {
        let config_arc = Arc::new(Mutex::new(config));
        let cb_arc: Arc<Box<dyn Fn(Config) + Send + Sync>> = Arc::new(Box::new(on_change));

        unsafe {
            GLOBAL_SETTINGS_CONFIG = Some(Arc::clone(&config_arc));
            GLOBAL_SETTINGS_CALLBACK = Some(Arc::clone(&cb_arc));

            let class_name = w!("SidewiseSettingsWindow");
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(settings_wnd_proc),
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

            // Create temporary test HWND to read the monitor DPI
            let hdc = windows::Win32::Graphics::Gdi::GetDC(HWND::default());
            let log_dpi = windows::Win32::Graphics::Gdi::GetDeviceCaps(hdc, windows::Win32::Graphics::Gdi::LOGPIXELSX);
            let _ = windows::Win32::Graphics::Gdi::ReleaseDC(HWND::default(), hdc);
            let scale = if log_dpi > 0 { log_dpi as f32 / 96.0 } else { 1.5 };

            let client_w = (BASE_SETTINGS_CLIENT_WIDTH as f32 * scale).round() as i32;
            let client_h = (BASE_SETTINGS_CLIENT_HEIGHT as f32 * scale).round() as i32;

            // Compute window size including borders and caption
            let mut rc = RECT {
                left: 0,
                top: 0,
                right: client_w,
                bottom: client_h,
            };
            let _ = windows::Win32::UI::WindowsAndMessaging::AdjustWindowRectEx(
                &mut rc,
                WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
                false,
                windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE(0),
            );

            let hwnd = CreateWindowExW(
                windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE(0),
                class_name,
                w!("Sidewise Settings"),
                WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
                windows::Win32::UI::WindowsAndMessaging::CW_USEDEFAULT,
                windows::Win32::UI::WindowsAndMessaging::CW_USEDEFAULT,
                rc.right - rc.left,
                rc.bottom - rc.top,
                HWND::default(),
                None,
                windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default(),
                None,
            );

            if let Ok(h) = hwnd {
                if !h.0.is_null() {
                    let _ = ShowWindow(h, SW_SHOW);
                }
            }
        }
    }
}

unsafe extern "system" fn settings_wnd_proc(
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
            handle_settings_click(hwnd, lx, ly);
            LRESULT(0)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            paint_settings(hwnd, hdc);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_DESTROY => {
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn handle_settings_click(hwnd: HWND, x: i32, y: i32) {
    if let Some(ref cfg_lock) = GLOBAL_SETTINGS_CONFIG {
        let mut new_cfg: Option<Config> = None;
        if let Ok(mut cfg) = cfg_lock.lock() {
            // Section: GESTURES
            // Row 1: Swap sides (Y ~ 138..174)
            if y >= 138 && y <= 174 && x >= 40 && x <= 600 {
                cfg.swap_sides = !cfg.swap_sides;
                new_cfg = Some(cfg.clone());
            }
            // Row 2: Bottom quarter only (Y ~ 176..212)
            else if y >= 176 && y <= 212 && x >= 40 && x <= 600 {
                cfg.bottom_quarter_only = !cfg.bottom_quarter_only;
                new_cfg = Some(cfg.clone());
            }
            // Row 3: Freeze cursor (Y ~ 214..250)
            else if y >= 214 && y <= 250 && x >= 40 && x <= 600 {
                cfg.freeze_cursor = !cfg.freeze_cursor;
                new_cfg = Some(cfg.clone());
            }
            // Row 4: Fine control (Y ~ 252..288)
            else if y >= 252 && y <= 288 && x >= 40 && x <= 600 {
                cfg.fine_control = !cfg.fine_control;
                new_cfg = Some(cfg.clone());
            }
            // Row 5: Tap to play or pause (Y ~ 290..326)
            else if y >= 290 && y <= 326 && x >= 40 && x <= 600 {
                cfg.tap_to_play_pause = !cfg.tap_to_play_pause;
                new_cfg = Some(cfg.clone());
            }
            // Row 6: Require modifier (Y ~ 328..364)
            // Segmented control: [ None | Alt | Ctrl ] from X ~ 440..600
            else if y >= 328 && y <= 364 && x >= 440 && x <= 600 {
                let seg_w = (600 - 440) / 3;
                let idx = ((x - 440) / seg_w).clamp(0, 2);
                cfg.require_modifier = match idx {
                    0 => ModifierRequirement::None,
                    1 => ModifierRequirement::Alt,
                    _ => ModifierRequirement::Ctrl,
                };
                new_cfg = Some(cfg.clone());
            }
            // Section: STARTUP
            // Row 1: Open at login (Y ~ 428..464)
            else if y >= 428 && y <= 464 && x >= 40 && x <= 600 {
                cfg.open_at_login = !cfg.open_at_login;
                let _ = StartupManager::set_enabled(cfg.open_at_login);
                new_cfg = Some(cfg.clone());
            }
            // Button: Send Touchpad Diagnostics (Y ~ 500..536, X ~ 40..250)
            else if y >= 500 && y <= 536 && x >= 40 && x <= 250 {
                export_touchpad_diagnostics();
                let _ = windows::Win32::Graphics::Gdi::InvalidateRect(hwnd, None, false);
                return;
            }
        }

        if let Some(cfg) = new_cfg {
            if let Some(ref cb) = GLOBAL_SETTINGS_CALLBACK {
                cb(cfg);
            }
            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(hwnd, None, false);
        }
    }
}

unsafe fn paint_settings(hwnd: HWND, hdc: HDC) {
    let mut rc = RECT::default();
    let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut rc);

    let scale = get_window_scale(hwnd);
    let s = |v: i32| -> i32 { (v as f32 * scale).round() as i32 };

    let mem_dc = CreateCompatibleDC(hdc);
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: rc.right,
            biHeight: -rc.bottom,
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

    // Scaled ClearType Fonts
    let title_font = create_ui_font(21, FW_SEMIBOLD.0 as i32, scale, false);
    let subtitle_font = create_ui_font(11, FW_NORMAL.0 as i32, scale, false);
    let section_font = create_ui_font(9, FW_SEMIBOLD.0 as i32, scale, false);
    let row_font = create_ui_font(11, FW_NORMAL.0 as i32, scale, false);
    let button_font = create_ui_font(10, FW_SEMIBOLD.0 as i32, scale, false);
    let seg_font = create_ui_font(9, FW_NORMAL.0 as i32, scale, false);
    let footer_font = create_ui_font(9, FW_NORMAL.0 as i32, scale, false);

    let mut cfg = Config::default();
    if let Some(ref cfg_lock) = GLOBAL_SETTINGS_CONFIG {
        if let Ok(c) = cfg_lock.lock() {
            cfg = c.clone();
        }
    }

    // 1. Header (Y ~ 32..88)
    let badge_sz = s(34);
    let badge_y = s(32) + (s(32) - badge_sz) / 2;
    crate::theme::draw_logo_badge(mem_dc, s(40), badge_y, badge_sz);

    let mut title_rc = RECT { left: s(40) + badge_sz + s(10), top: s(32), right: s(600), bottom: s(64) };
    draw_text(mem_dc, "Settings", &mut title_rc, COLOR_TEXT_PRIMARY, title_font, DT_LEFT | DT_VCENTER | DT_SINGLELINE);

    let mut sub_rc = RECT { left: s(40), top: s(68), right: s(600), bottom: s(90) };
    draw_text(mem_dc, "Changes take effect as soon as you make them.", &mut sub_rc, COLOR_TEXT_MUTED, subtitle_font, DT_LEFT | DT_VCENTER | DT_SINGLELINE);

    // 2. Section: GESTURES
    let mut sec1_rc = RECT { left: s(40), top: s(112), right: s(600), bottom: s(130) };
    draw_text(mem_dc, "GESTURES", &mut sec1_rc, COLOR_TEXT_LABEL, section_font, DT_LEFT | DT_VCENTER | DT_SINGLELINE);

    // Row 1: Swap sides (Y: 138)
    draw_settings_row_switch(mem_dc, "Swap sides", cfg.swap_sides, s(40), s(600), s(138), s(22), row_font);
    draw_divider(mem_dc, s(40), s(600), s(174));

    // Row 2: Bottom quarter only (Y: 176)
    draw_settings_row_switch(mem_dc, "Bottom quarter only", cfg.bottom_quarter_only, s(40), s(600), s(176), s(22), row_font);
    draw_divider(mem_dc, s(40), s(600), s(212));

    // Row 3: Freeze cursor (Y: 214)
    draw_settings_row_switch(mem_dc, "Freeze cursor", cfg.freeze_cursor, s(40), s(600), s(214), s(22), row_font);
    draw_divider(mem_dc, s(40), s(600), s(250));

    // Row 4: Fine control (Y: 252)
    draw_settings_row_switch(mem_dc, "Fine control", cfg.fine_control, s(40), s(600), s(252), s(22), row_font);
    draw_divider(mem_dc, s(40), s(600), s(288));

    // Row 5: Tap to play or pause (Y: 290)
    draw_settings_row_switch(mem_dc, "Tap to play or pause", cfg.tap_to_play_pause, s(40), s(600), s(290), s(22), row_font);
    draw_divider(mem_dc, s(40), s(600), s(326));

    // Row 6: Require modifier (Y: 328)
    let mut mod_lbl = RECT { left: s(40), top: s(334), right: s(300), bottom: s(356) };
    draw_text(mem_dc, "Require modifier", &mut mod_lbl, COLOR_TEXT_PRIMARY, row_font, DT_LEFT | DT_VCENTER | DT_SINGLELINE);

    let mod_sel = match cfg.require_modifier {
        ModifierRequirement::None => 0,
        ModifierRequirement::Alt => 1,
        ModifierRequirement::Ctrl => 2,
    };
    let mod_seg_rc = RECT { left: s(440), top: s(330), right: s(600), bottom: s(358) };
    draw_segmented_control(mem_dc, &mod_seg_rc, &["None", "Alt", "Ctrl"], mod_sel, s(10), seg_font);

    // 3. Section: STARTUP
    let mut sec3_rc = RECT { left: s(40), top: s(402), right: s(600), bottom: s(420) };
    draw_text(mem_dc, "STARTUP", &mut sec3_rc, COLOR_TEXT_LABEL, section_font, DT_LEFT | DT_VCENTER | DT_SINGLELINE);

    // Row 1: Open at login (Y: 428)
    draw_settings_row_switch(mem_dc, "Open at login", cfg.open_at_login, s(40), s(600), s(428), s(22), row_font);
    draw_divider(mem_dc, s(40), s(600), s(464));

    // 4. Diagnostics Button (Y ~ 500..534)
    let mut diag_rc = RECT { left: s(40), top: s(500), right: s(250), bottom: s(534) };
    fill_round_rect(mem_dc, &diag_rc, COLOR_CARD, COLOR_BORDER, s(8));
    draw_text(mem_dc, "Send Touchpad Diagnostics", &mut diag_rc, COLOR_TEXT_PRIMARY, button_font, DT_CENTER | DT_VCENTER | DT_SINGLELINE);

    // Feedback status under diagnostics button
    if let Some(ref msg) = GLOBAL_DIAGNOSTICS_MSG {
        let mut msg_rc = RECT { left: s(265), top: s(506), right: s(600), bottom: s(528) };
        draw_text(mem_dc, msg, &mut msg_rc, COLOR_ACCENT_GREEN, subtitle_font, DT_LEFT | DT_VCENTER | DT_SINGLELINE);
    }

    // 5. Footer (Y ~ 580..602)
    let mut footer_rc = RECT { left: s(40), top: s(580), right: s(600), bottom: s(602) };
    draw_text(
        mem_dc,
        "Sidewise 1.0 · Changes are saved automatically.",
        &mut footer_rc,
        COLOR_TEXT_MUTED,
        footer_font,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );

    let _ = windows::Win32::Graphics::Gdi::BitBlt(
        hdc,
        0,
        0,
        rc.right,
        rc.bottom,
        mem_dc,
        0,
        0,
        windows::Win32::Graphics::Gdi::SRCCOPY,
    );

    let _ = SelectObject(mem_dc, old_bm);
    let _ = DeleteObject(hbm);
    let _ = DeleteDC(mem_dc);
    let _ = DeleteObject(title_font);
    let _ = DeleteObject(subtitle_font);
    let _ = DeleteObject(section_font);
    let _ = DeleteObject(row_font);
    let _ = DeleteObject(button_font);
    let _ = DeleteObject(seg_font);
    let _ = DeleteObject(footer_font);
}

#[allow(clippy::too_many_arguments)]
unsafe fn draw_settings_row_switch(
    hdc: HDC,
    label: &str,
    active: bool,
    x_left: i32,
    x_right: i32,
    y: i32,
    h: i32,
    font: windows::Win32::Graphics::Gdi::HFONT,
) {
    let mut lbl_rc = RECT { left: x_left, top: y + 2, right: x_right - 50, bottom: y + h };
    draw_text(hdc, label, &mut lbl_rc, COLOR_TEXT_PRIMARY, font, DT_LEFT | DT_VCENTER | DT_SINGLELINE);

    let sw_w = (h as f32 * 1.9) as i32;
    let sw_rc = RECT { left: x_right - sw_w, top: y, right: x_right, bottom: y + h };
    draw_switch(hdc, &sw_rc, active);
}

unsafe fn draw_divider(hdc: HDC, x1: i32, x2: i32, y: i32) {
    let pen = CreatePen(PS_SOLID, 1, COLOR_BORDER);
    let old_pen = SelectObject(hdc, pen);
    let mut pt = POINT::default();
    let _ = MoveToEx(hdc, x1, y, Some(&mut pt));
    let _ = LineTo(hdc, x2, y);
    let _ = SelectObject(hdc, old_pen);
    let _ = DeleteObject(pen);
}

fn export_touchpad_diagnostics() {
    let dev_info = sidewise_win::get_latest_device_info();
    let (ident_opt, prof_opt) = match dev_info {
        Some((i, p)) => (Some(i), Some(p)),
        None => (None, None),
    };

    let vid_str = ident_opt.as_ref().map(|i| format!("\"0x{:04X}\"", i.vid)).unwrap_or_else(|| "\"unknown\"".into());
    let pid_str = ident_opt.as_ref().map(|i| format!("\"0x{:04X}\"", i.pid)).unwrap_or_else(|| "\"unknown\"".into());
    let name_str = ident_opt.as_ref().map(|i| format!("\"{}\"", i.name.replace('\\', "\\\\"))).unwrap_or_else(|| "\"unknown\"".into());
    let hash_str = ident_opt.as_ref().map(|i| format!("\"{}\"", i.descriptor_hash)).unwrap_or_else(|| "\"unknown\"".into());
    let prof_id = prof_opt.as_ref().map(|p| format!("\"{}\"", p.id)).unwrap_or_else(|| "\"generic\"".into());
    let prof_source = prof_opt.as_ref().map(|p| format!("\"{}\"", p.source)).unwrap_or_else(|| "\"none\"".into());

    let json = format!(
r#"{{
  "identity": {{
    "name": {},
    "vendor_id": {},
    "product_id": {},
    "descriptor_hash": {}
  }},
  "profile": {{
    "id": {},
    "source": {}
  }},
  "exported_at_utc": "{}",
  "privacy_note": "User-reviewed diagnostic export. Touchpad coordinates are never stored or transmitted."
}}"#,
        name_str, vid_str, pid_str, hash_str, prof_id, prof_source,
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()
    );

    let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
    let dir = std::path::PathBuf::from(appdata).join("Sidewise");
    let _ = std::fs::create_dir_all(&dir);
    let out_path = dir.join("touchpad_diagnostics.json");
    if std::fs::write(&out_path, json).is_ok() {
        unsafe {
            GLOBAL_DIAGNOSTICS_MSG = Some("Diagnostics saved to %APPDATA%\\Sidewise".into());
        }
    }
}
