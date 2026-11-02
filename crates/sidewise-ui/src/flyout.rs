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
    DT_CENTER, DT_LEFT, DT_RIGHT, DT_SINGLELINE, DT_VCENTER, FW_NORMAL, FW_SEMIBOLD,
    HDC, PAINTSTRUCT, PS_SOLID,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetCursorPos, PostQuitMessage,
    RegisterClassExW, SetForegroundWindow,
    SetWindowPos, ShowWindow, CS_HREDRAW, CS_VREDRAW, HICON,
    SWP_SHOWWINDOW, SW_HIDE,
    WA_INACTIVE, WM_ACTIVATE, WM_KEYDOWN, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WNDCLASSEXW, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};
use crate::theme::{
    create_ui_font, draw_segmented_control, draw_slider_bar, draw_switch, draw_text,
    fill_round_rect, get_window_scale, COLOR_BG, COLOR_BORDER,
    COLOR_BORDER_CARD, COLOR_CARD, COLOR_TEXT_MUTED,
    COLOR_TEXT_PRIMARY,
};

pub struct FlyoutState {
    pub config: Config,
    pub volume: f32,
    pub brightness: f32,
    pub is_dragging_sensitivity: bool,
    pub is_dragging_edge_width: bool,
    pub on_config_change: Option<Box<dyn Fn(Config) + Send + Sync>>,
    pub on_open_setup: Option<Box<dyn Fn() + Send + Sync>>,
}

pub struct FlyoutWindow {
    hwnd: HWND,
    state: Arc<Mutex<FlyoutState>>,
}

unsafe impl Send for FlyoutWindow {}
unsafe impl Sync for FlyoutWindow {}

static mut GLOBAL_FLYOUT_STATE: Option<Arc<Mutex<FlyoutState>>> = None;

// Logical Base Dimensions (Flyout: 360 x 600 at 100% DPI)
const BASE_FLYOUT_WIDTH: i32 = 360;
const BASE_FLYOUT_HEIGHT: i32 = 600;

impl FlyoutWindow {
    pub fn new(
        initial_config: Config,
        on_config_change: impl Fn(Config) + Send + Sync + 'static,
        on_open_setup: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let state = Arc::new(Mutex::new(FlyoutState {
            config: initial_config,
            volume: 0.5,
            brightness: 0.5,
            is_dragging_sensitivity: false,
            is_dragging_edge_width: false,
            on_config_change: Some(Box::new(on_config_change)),
            on_open_setup: Some(Box::new(on_open_setup)),
        }));

        unsafe {
            GLOBAL_FLYOUT_STATE = Some(Arc::clone(&state));
        }
        let hwnd = unsafe { create_flyout_hwnd() };

        Self { hwnd, state }
    }

    pub fn update_levels(&self, volume: f32, brightness: f32) {
        if let Ok(mut st) = self.state.lock() {
            st.volume = volume;
            st.brightness = brightness;
        }
        unsafe {
            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(self.hwnd, None, false);
        }
    }

    pub fn toggle(&self) {
        unsafe {
            let is_visible = windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(self.hwnd).as_bool();
            if is_visible {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            } else {
                self.show_anchored();
            }
        }
    }

    pub fn show_anchored(&self) {
        unsafe {
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);

            let scale = get_window_scale(self.hwnd);
            let width = (BASE_FLYOUT_WIDTH as f32 * scale).round() as i32;
            let height = (BASE_FLYOUT_HEIGHT as f32 * scale).round() as i32;

            // Anchor above the taskbar / cursor
            let x = (pt.x - width / 2).max(10);
            let y = (pt.y - height - (12.0 * scale) as i32).max(10);

            let _ = SetWindowPos(
                self.hwnd,
                windows::Win32::UI::WindowsAndMessaging::HWND_TOPMOST,
                x,
                y,
                width,
                height,
                SWP_SHOWWINDOW,
            );
            let _ = SetForegroundWindow(self.hwnd);
            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(self.hwnd, None, false);
        }
    }

    pub fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }
}

unsafe fn create_flyout_hwnd() -> HWND {
    let class_name = w!("SidewiseFlyoutWindow");
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(flyout_wnd_proc),
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

    CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
        class_name,
        w!("Sidewise Flyout"),
        WS_POPUP,
        0,
        0,
        BASE_FLYOUT_WIDTH,
        BASE_FLYOUT_HEIGHT,
        HWND::default(),
        None,
        windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default(),
        None,
    )
    .unwrap_or_default()
}

unsafe extern "system" fn flyout_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_ACTIVATE => {
            if (wparam.0 as u32 & 0xFFFF) == WA_INACTIVE {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if wparam.0 == windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE.0 as usize {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let raw_x = (lparam.0 as i16) as i32;
            let raw_y = ((lparam.0 >> 16) as i16) as i32;
            let scale = get_window_scale(hwnd);
            let lx = (raw_x as f32 / scale).round() as i32;
            let ly = (raw_y as f32 / scale).round() as i32;
            handle_mouse_click(hwnd, lx, ly);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if (wparam.0 & 0x0001) != 0 { // MK_LBUTTON
                let raw_x = (lparam.0 as i16) as i32;
                let scale = get_window_scale(hwnd);
                let lx = (raw_x as f32 / scale).round() as i32;
                handle_mouse_drag(hwnd, lx);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            if let Some(ref st_lock) = GLOBAL_FLYOUT_STATE {
                if let Ok(mut st) = st_lock.lock() {
                    st.is_dragging_sensitivity = false;
                    st.is_dragging_edge_width = false;
                }
            }
            LRESULT(0)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            paint_flyout(hwnd, hdc);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn handle_mouse_click(hwnd: HWND, x: i32, y: i32) {
    if let Some(ref st_lock) = GLOBAL_FLYOUT_STATE {
        let mut notify_cfg: Option<Config> = None;
        let mut do_quit = false;
        let mut do_hide = false;
        let mut login_toggle: Option<bool> = None;

        if let Ok(mut st) = st_lock.lock() {
            // Every inline settings row is 30px tall, starting at Y = 204.
            let row_hit = |index: i32| -> bool {
                let top = 204 + index * 30;
                y >= top && y < top + 30 && x >= 24 && x <= 336
            };

            // Header close button (X ~ 304..336, Y ~ 24..48)
            if y >= 18 && y <= 54 && x >= 296 && x <= 340 {
                do_hide = true;
            }
            // Row 1: Swap sides
            else if row_hit(0) {
                st.config.swap_sides = !st.config.swap_sides;
                notify_cfg = Some(st.config.clone());
            }
            // Row 2: Bottom quarter only
            else if row_hit(1) {
                st.config.bottom_quarter_only = !st.config.bottom_quarter_only;
                notify_cfg = Some(st.config.clone());
            }
            // Row 3: Freeze cursor
            else if row_hit(2) {
                st.config.freeze_cursor = !st.config.freeze_cursor;
                notify_cfg = Some(st.config.clone());
            }
            // Row 4: Fine control
            else if row_hit(3) {
                st.config.fine_control = !st.config.fine_control;
                notify_cfg = Some(st.config.clone());
            }
            // Row 5: Tap to play or pause
            else if row_hit(4) {
                st.config.tap_to_play_pause = !st.config.tap_to_play_pause;
                notify_cfg = Some(st.config.clone());
            }
            // Row 6: Require modifier (segmented control on the right)
            else if row_hit(5) {
                if x >= 210 {
                    let seg_w = (336 - 210) / 3;
                    let idx = ((x - 210) / seg_w).clamp(0, 2);
                    st.config.require_modifier = match idx {
                        0 => ModifierRequirement::None,
                        1 => ModifierRequirement::Alt,
                        _ => ModifierRequirement::Ctrl,
                    };
                } else {
                    // Clicking the label cycles through the options.
                    st.config.require_modifier = match st.config.require_modifier {
                        ModifierRequirement::None => ModifierRequirement::Alt,
                        ModifierRequirement::Alt => ModifierRequirement::Ctrl,
                        ModifierRequirement::Ctrl => ModifierRequirement::None,
                    };
                }
                notify_cfg = Some(st.config.clone());
            }
            // Row 7: Quick mute
            else if row_hit(6) {
                st.config.quick_mute = !st.config.quick_mute;
                notify_cfg = Some(st.config.clone());
            }
            // Row 8: Media strip
            else if row_hit(7) {
                st.config.media_strip = !st.config.media_strip;
                notify_cfg = Some(st.config.clone());
            }
            // Row 9: Open at login
            else if row_hit(8) {
                st.config.open_at_login = !st.config.open_at_login;
                login_toggle = Some(st.config.open_at_login);
                notify_cfg = Some(st.config.clone());
            }
            // Sensitivity slider (Y ~ 486..520)
            else if y >= 486 && y <= 520 && x >= 24 && x <= 336 {
                st.is_dragging_sensitivity = true;
                let ratio = ((x - 24) as f32 / 312.0).clamp(0.0, 1.0);
                st.config.sensitivity = 0.5 + ratio * 1.5;
                notify_cfg = Some(st.config.clone());
            }
            // Edge width slider (Y ~ 524..558)
            else if y >= 524 && y <= 558 && x >= 24 && x <= 336 {
                st.is_dragging_edge_width = true;
                let ratio = ((x - 24) as f32 / 312.0).clamp(0.0, 1.0);
                st.config.edge_width = 0.04 + ratio * 0.16;
                notify_cfg = Some(st.config.clone());
            }
            // Footer: Quit button (Y ~ 560..592, X ~ 260..336)
            else if y >= 560 && y <= 592 && x >= 260 && x <= 336 {
                do_quit = true;
            }
        }

        if let Some(cfg) = notify_cfg {
            if let Ok(st) = st_lock.lock() {
                if let Some(ref cb) = st.on_config_change {
                    cb(cfg);
                }
            }
            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(hwnd, None, false);
        }

        if let Some(enabled) = login_toggle {
            let _ = StartupManager::set_enabled(enabled);
        }

        if do_hide {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }

        if do_quit {
            PostQuitMessage(0);
        }
    }
}

unsafe fn handle_mouse_drag(hwnd: HWND, x: i32) {
    if let Some(ref st_lock) = GLOBAL_FLYOUT_STATE {
        let mut notify_cfg: Option<Config> = None;
        if let Ok(mut st) = st_lock.lock() {
            if st.is_dragging_sensitivity {
                let ratio = ((x - 24) as f32 / 312.0).clamp(0.0, 1.0);
                st.config.sensitivity = 0.5 + ratio * 1.5;
                notify_cfg = Some(st.config.clone());
            } else if st.is_dragging_edge_width {
                let ratio = ((x - 24) as f32 / 312.0).clamp(0.0, 1.0);
                st.config.edge_width = 0.04 + ratio * 0.16;
                notify_cfg = Some(st.config.clone());
            }
        }

        if let Some(cfg) = notify_cfg {
            if let Ok(st) = st_lock.lock() {
                if let Some(ref cb) = st.on_config_change {
                    cb(cfg);
                }
            }
            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(hwnd, None, false);
        }
    }
}

unsafe fn paint_flyout(hwnd: HWND, hdc: HDC) {
    let mut client_rect = RECT::default();
    let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut client_rect);

    let scale = get_window_scale(hwnd);
    let s = |v: i32| -> i32 { (v as f32 * scale).round() as i32 };

    let mem_dc = CreateCompatibleDC(hdc);
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: client_rect.right,
            biHeight: -client_rect.bottom, // top-down
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
    FillRect(mem_dc, &client_rect, bg_brush);
    let _ = DeleteObject(bg_brush);

    // Fonts (DPI scaled)
    let title_font = create_ui_font(12, FW_SEMIBOLD.0 as i32, scale, false); // 15px
    let ready_font = create_ui_font(9, FW_NORMAL.0 as i32, scale, false);   // 12px
    let prompt_font = create_ui_font(8, FW_NORMAL.0 as i32, scale, false);  // 11px
    let label_font = create_ui_font(8, FW_SEMIBOLD.0 as i32, scale, false); // 10px uppercase
    let row_font = create_ui_font(11, FW_NORMAL.0 as i32, scale, false);   // 14px
    let mono_font = create_ui_font(9, FW_NORMAL.0 as i32, scale, true);    // 12px mono
    let footer_font = create_ui_font(10, FW_SEMIBOLD.0 as i32, scale, false); // 13px

    let mut st_snap = FlyoutState {
        config: Config::default(),
        volume: 0.5,
        brightness: 0.5,
        is_dragging_sensitivity: false,
        is_dragging_edge_width: false,
        on_config_change: None,
        on_open_setup: None,
    };
    if let Some(ref st_lock) = GLOBAL_FLYOUT_STATE {
        if let Ok(st) = st_lock.lock() {
            st_snap.config = st.config.clone();
            st_snap.volume = st.volume;
            st_snap.brightness = st.brightness;
        }
    }

    // 1. Header (padding: 24px left/right, 24px top)
    let badge_sz = s(22);
    let badge_y = s(24) + (s(24) - badge_sz) / 2;
    crate::theme::draw_logo_badge(mem_dc, s(24), badge_y, badge_sz);

    let mut title_rect = RECT { left: s(24) + badge_sz + s(8), top: s(24), right: s(200), bottom: s(48) };
    draw_text(mem_dc, "Sidewise", &mut title_rect, COLOR_TEXT_PRIMARY, title_font, DT_LEFT | DT_VCENTER | DT_SINGLELINE);

    let mut status_rect = RECT { left: s(180), top: s(24), right: s(300), bottom: s(48) };
    draw_text(mem_dc, "Touchpad ready", &mut status_rect, COLOR_TEXT_MUTED, ready_font, DT_RIGHT | DT_VCENTER | DT_SINGLELINE);

    // Close button (hides the flyout)
    let mut close_rect = RECT { left: s(304), top: s(24), right: s(336), bottom: s(48) };
    draw_text(mem_dc, "\u{00D7}", &mut close_rect, COLOR_TEXT_MUTED, title_font, DT_CENTER | DT_VCENTER | DT_SINGLELINE);

    // 2. Touchpad Card Container (Y ~ 62..170, Height: 108px, Width: 312px)
    let pad_rect = RECT { left: s(24), top: s(62), right: s(336), bottom: s(170) };
    fill_round_rect(mem_dc, &pad_rect, COLOR_CARD, COLOR_BORDER_CARD, s(18));

    let (left_val, right_val, left_label, right_label) = if !st_snap.config.swap_sides {
        (st_snap.brightness, st_snap.volume, "BRIGHTNESS", "VOLUME")
    } else {
        (st_snap.volume, st_snap.brightness, "VOLUME", "BRIGHTNESS")
    };

    // Static diagram: the edge zones no longer track the edge-width slider.
    // A fixed 64px zone (the old default) keeps the preview balanced and stable.
    let zone_w = s(64);

    let pad_total_h = pad_rect.bottom - pad_rect.top;

    // --- Render pixel-level anti-aliased overlay directly in 32-bit DIB buffer ---
    // This gives smooth shadows, translucent gradient fill, active level bar, and illuminated dot glow
    let width = client_rect.right as usize;
    let height = client_rect.bottom as usize;
    if !bits.is_null() {
        // Distinct accents: brightness is warm amber (#B07A2E),
        // volume stays pine teal (#2D6A5F).
        let brightness_rgb = (176.0f32, 122.0f32, 46.0f32);
        let volume_rgb = (45.0f32, 106.0f32, 95.0f32);
        let (left_rgb, right_rgb) = if !st_snap.config.swap_sides {
            (brightness_rgb, volume_rgb)
        } else {
            (volume_rgb, brightness_rgb)
        };

        let slice = std::slice::from_raw_parts_mut(bits as *mut u32, width * height);

        let left_fill_h = ((pad_total_h as f32) * left_val.clamp(0.0, 1.0)).round() as i32;
        let right_fill_h = ((pad_total_h as f32) * right_val.clamp(0.0, 1.0)).round() as i32;

        let left_bar_y = pad_rect.bottom - left_fill_h;
        let right_bar_y = pad_rect.bottom - right_fill_h;

        let dot_diam = s(12) as f32;
        let dot_rad = dot_diam / 2.0;
        let left_dot_cx = (pad_rect.left + zone_w / 2) as f32;
        let left_dot_cy = (pad_rect.bottom - left_fill_h) as f32;

        let right_dot_cx = (pad_rect.right - zone_w / 2) as f32;
        let right_dot_cy = (pad_rect.bottom - right_fill_h) as f32;

        // Clip scanning to the bounding box of pad_rect
        let py_start = pad_rect.top.max(0) as usize;
        let py_end = (pad_rect.bottom as usize).min(height);
        let px_start = pad_rect.left.max(0) as usize;
        let px_end = (pad_rect.right as usize).min(width);

        for py in py_start..py_end {
            for px in px_start..px_end {
                let idx = py * width + px;
                let cur_pixel = slice[idx];
                let cur_b = (cur_pixel & 0xFF) as f32;
                let cur_g = ((cur_pixel >> 8) & 0xFF) as f32;
                let cur_r = ((cur_pixel >> 16) & 0xFF) as f32;

                let fx = px as f32;
                let fy = py as f32;

                let mut r = cur_r;
                let mut g = cur_g;
                let mut b = cur_b;

                // 1. Subtle top-left highlight blur (Main.dc.html light reflection)
                let hl_cx = (pad_rect.left + s(60)) as f32;
                let hl_cy = (pad_rect.top + s(30)) as f32;
                let hl_dist = ((fx - hl_cx).powi(2) + (fy - hl_cy).powi(2)).sqrt();
                if hl_dist < (s(90) as f32) {
                    let hl_factor = ((1.0 - (hl_dist / (s(90) as f32))) * 0.18).clamp(0.0, 1.0);
                    r = r * (1.0 - hl_factor) + 255.0 * hl_factor;
                    g = g * (1.0 - hl_factor) + 255.0 * hl_factor;
                    b = b * (1.0 - hl_factor) + 255.0 * hl_factor;
                }

                // 2. Left Zone Fill (rgba(45, 106, 95, 0.16) below level)
                if px < (pad_rect.left + zone_w) as usize {
                    if (py as i32) >= left_bar_y {
                        let alpha = 0.18;
                        r = r * (1.0 - alpha) + left_rgb.0 * alpha;
                        g = g * (1.0 - alpha) + left_rgb.1 * alpha;
                        b = b * (1.0 - alpha) + left_rgb.2 * alpha;
                    }

                    // Level line highlight (2px bar at level height)
                    let dist_to_bar = ((py as i32) - left_bar_y).abs();
                    if dist_to_bar <= s(1) {
                        let bar_alpha = 0.85;
                        r = r * (1.0 - bar_alpha) + left_rgb.0 * bar_alpha;
                        g = g * (1.0 - bar_alpha) + left_rgb.1 * bar_alpha;
                        b = b * (1.0 - bar_alpha) + left_rgb.2 * bar_alpha;
                    } else if dist_to_bar <= s(5) {
                        // Subtle glow around the line
                        let glow_a = (1.0 - dist_to_bar as f32 / s(5) as f32) * 0.25;
                        r = r * (1.0 - glow_a) + left_rgb.0 * glow_a;
                        g = g * (1.0 - glow_a) + left_rgb.1 * glow_a;
                        b = b * (1.0 - glow_a) + left_rgb.2 * glow_a;
                    }

                    // Zone right border (rgba(0,0,0,0.06))
                    if px == (pad_rect.left + zone_w - s(1)) as usize {
                        let sep_a = 0.08;
                        r *= 1.0 - sep_a;
                        g *= 1.0 - sep_a;
                        b *= 1.0 - sep_a;
                    }
                }

                // 3. Right Zone Fill (rgba(45, 106, 95, 0.16) below level)
                if px >= (pad_rect.right - zone_w) as usize {
                    if (py as i32) >= right_bar_y {
                        let alpha = 0.18;
                        r = r * (1.0 - alpha) + right_rgb.0 * alpha;
                        g = g * (1.0 - alpha) + right_rgb.1 * alpha;
                        b = b * (1.0 - alpha) + right_rgb.2 * alpha;
                    }

                    // Level line highlight (2px bar at level height)
                    let dist_to_bar = ((py as i32) - right_bar_y).abs();
                    if dist_to_bar <= s(1) {
                        let bar_alpha = 0.85;
                        r = r * (1.0 - bar_alpha) + right_rgb.0 * bar_alpha;
                        g = g * (1.0 - bar_alpha) + right_rgb.1 * bar_alpha;
                        b = b * (1.0 - bar_alpha) + right_rgb.2 * bar_alpha;
                    } else if dist_to_bar <= s(5) {
                        // Subtle glow
                        let glow_a = (1.0 - dist_to_bar as f32 / s(5) as f32) * 0.25;
                        r = r * (1.0 - glow_a) + right_rgb.0 * glow_a;
                        g = g * (1.0 - glow_a) + right_rgb.1 * glow_a;
                        b = b * (1.0 - glow_a) + right_rgb.2 * glow_a;
                    }

                    // Zone left border (rgba(0,0,0,0.06))
                    if px == (pad_rect.right - zone_w) as usize {
                        let sep_a = 0.08;
                        r *= 1.0 - sep_a;
                        g *= 1.0 - sep_a;
                        b *= 1.0 - sep_a;
                    }
                }

                // 3b. Minimal Media Strip Indicator at bottom of touchpad
                if st_snap.config.media_strip {
                    let strip_h = s(5) as f32;
                    let strip_margin = (zone_w + s(14)) as f32;
                    let strip_x1 = pad_rect.left as f32 + strip_margin;
                    let strip_x2 = pad_rect.right as f32 - strip_margin;
                    let strip_y = (pad_rect.bottom - s(8)) as f32;

                    if fx >= strip_x1 && fx <= strip_x2 && fy >= strip_y && fy <= strip_y + strip_h {
                        let pill_r = strip_h / 2.0;
                        let pdx = (fx - (strip_x1 + strip_x2) / 2.0).abs() - ((strip_x2 - strip_x1) / 2.0 - pill_r);
                        let pdy = (fy - (strip_y + pill_r)).abs();
                        let dist_pill = (pdx.max(0.0).powi(2) + pdy.max(0.0).powi(2)).sqrt().max(pdx.max(pdy)) - pill_r;

                        if dist_pill <= 0.0 {
                            let strip_a = 0.35;
                            r = r * (1.0 - strip_a) + 45.0 * strip_a;
                            g = g * (1.0 - strip_a) + 106.0 * strip_a;
                            b = b * (1.0 - strip_a) + 95.0 * strip_a;
                        } else if dist_pill <= 1.0 {
                            let aa = (1.0 - dist_pill) * 0.35;
                            r = r * (1.0 - aa) + 45.0 * aa;
                            g = g * (1.0 - aa) + 106.0 * aa;
                            b = b * (1.0 - aa) + 95.0 * aa;
                        }
                    }
                }

                // 4. Left Dot Indicator with outer halo and antialiasing
                let dist_l = ((fx - left_dot_cx).powi(2) + (fy - left_dot_cy).powi(2)).sqrt();
                if dist_l <= dot_rad + (s(4) as f32) {
                    if dist_l <= dot_rad {
                        // Inner solid accent circle
                        let aa = (dot_rad - dist_l).clamp(0.0, 1.0);
                        r = r * (1.0 - aa) + left_rgb.0 * aa;
                        g = g * (1.0 - aa) + left_rgb.1 * aa;
                        b = b * (1.0 - aa) + left_rgb.2 * aa;
                    } else {
                        // Outer halo ring
                        let ring_d = dist_l - dot_rad;
                        let ring_a = ((1.0 - (ring_d / (s(4) as f32))) * 0.25).clamp(0.0, 1.0);
                        r = r * (1.0 - ring_a) + left_rgb.0 * ring_a;
                        g = g * (1.0 - ring_a) + left_rgb.1 * ring_a;
                        b = b * (1.0 - ring_a) + left_rgb.2 * ring_a;
                    }
                }

                // 5. Right Dot Indicator with outer halo and antialiasing
                let dist_r = ((fx - right_dot_cx).powi(2) + (fy - right_dot_cy).powi(2)).sqrt();
                if dist_r <= dot_rad + (s(4) as f32) {
                    if dist_r <= dot_rad {
                        let aa = (dot_rad - dist_r).clamp(0.0, 1.0);
                        r = r * (1.0 - aa) + right_rgb.0 * aa;
                        g = g * (1.0 - aa) + right_rgb.1 * aa;
                        b = b * (1.0 - aa) + right_rgb.2 * aa;
                    } else {
                        let ring_d = dist_r - dot_rad;
                        let ring_a = ((1.0 - (ring_d / (s(4) as f32))) * 0.25).clamp(0.0, 1.0);
                        r = r * (1.0 - ring_a) + right_rgb.0 * ring_a;
                        g = g * (1.0 - ring_a) + right_rgb.1 * ring_a;
                        b = b * (1.0 - ring_a) + right_rgb.2 * ring_a;
                    }
                }

                slice[idx] = (cur_pixel & 0xFF000000)
                    | ((r.round() as u32) << 16)
                    | ((g.round() as u32) << 8)
                    | (b.round() as u32);
            }
        }
    }

    // Percentage Labels at top of zones (centered horizontally in zone)
    let mut left_pct_rect = RECT {
        left: pad_rect.left,
        top: pad_rect.top + s(12),
        right: pad_rect.left + zone_w,
        bottom: pad_rect.top + s(30),
    };
    let left_pct = format!("{}%", (left_val * 100.0).round() as u32);
    draw_text(mem_dc, &left_pct, &mut left_pct_rect, COLOR_TEXT_PRIMARY, mono_font, DT_CENTER | DT_VCENTER | DT_SINGLELINE);

    let mut right_pct_rect = RECT {
        left: pad_rect.right - zone_w,
        top: pad_rect.top + s(12),
        right: pad_rect.right,
        bottom: pad_rect.top + s(30),
    };
    let right_pct = format!("{}%", (right_val * 100.0).round() as u32);
    draw_text(mem_dc, &right_pct, &mut right_pct_rect, COLOR_TEXT_PRIMARY, mono_font, DT_CENTER | DT_VCENTER | DT_SINGLELINE);

    // Center Prompt in Trackpad: "Slide an edge" (and minimal media hint if enabled)
    if st_snap.config.media_strip {
        let mut prompt_rect = RECT {
            left: pad_rect.left + zone_w,
            top: pad_rect.top + s(28),
            right: pad_rect.right - zone_w,
            bottom: pad_rect.top + s(52),
        };
        draw_text(mem_dc, "Slide an edge", &mut prompt_rect, COLOR_TEXT_MUTED, prompt_font, DT_CENTER | DT_VCENTER | DT_SINGLELINE);

        let mut media_hint_rect = RECT {
            left: pad_rect.left + zone_w,
            top: pad_rect.bottom - s(24),
            right: pad_rect.right - zone_w,
            bottom: pad_rect.bottom - s(10),
        };
        draw_text(mem_dc, "◂ swipe media ▸", &mut media_hint_rect, COLOR_TEXT_MUTED, prompt_font, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    } else {
        let mut prompt_rect = RECT {
            left: pad_rect.left + zone_w,
            top: pad_rect.top + s(36),
            right: pad_rect.right - zone_w,
            bottom: pad_rect.bottom - s(36),
        };
        draw_text(mem_dc, "Slide an edge", &mut prompt_rect, COLOR_TEXT_MUTED, prompt_font, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    }

    // Uppercase Zone Labels below Card (Y ~ 178..192)
    let mut l_lbl = RECT { left: s(24), top: s(178), right: s(150), bottom: s(192) };
    draw_text(mem_dc, left_label, &mut l_lbl, COLOR_TEXT_MUTED, label_font, DT_LEFT | DT_VCENTER | DT_SINGLELINE);

    let mut r_lbl = RECT { left: s(210), top: s(178), right: s(336), bottom: s(192) };
    draw_text(mem_dc, right_label, &mut r_lbl, COLOR_TEXT_MUTED, label_font, DT_RIGHT | DT_VCENTER | DT_SINGLELINE);

    // Divider before the inline settings rows (Y = 200)
    draw_divider(mem_dc, s(24), s(336), s(200));

    // 3. Inline settings rows (Y = 204 + index * 30, height 30px)
    let row_top = |index: i32| s(204 + index * 30);
    let row_bottom = |index: i32| s(204 + index * 30 + 30);
    let switch_rect = |index: i32| RECT {
        left: s(300),
        top: s(204 + index * 30 + 6),
        right: s(336),
        bottom: s(204 + index * 30 + 24),
    };

    let draw_row_label = |index: i32, text: &str| {
        let mut rc = RECT {
            left: s(24),
            top: row_top(index),
            right: s(280),
            bottom: row_bottom(index),
        };
        draw_text(mem_dc, text, &mut rc, COLOR_TEXT_PRIMARY, row_font, DT_LEFT | DT_VCENTER | DT_SINGLELINE);
    };

    // Row 1: Swap sides
    draw_row_label(0, "Swap sides");
    draw_switch(mem_dc, &switch_rect(0), st_snap.config.swap_sides);

    // Row 2: Bottom quarter only
    draw_row_label(1, "Bottom quarter only");
    draw_switch(mem_dc, &switch_rect(1), st_snap.config.bottom_quarter_only);

    // Row 3: Freeze cursor
    draw_row_label(2, "Freeze cursor");
    draw_switch(mem_dc, &switch_rect(2), st_snap.config.freeze_cursor);

    // Row 4: Fine control
    draw_row_label(3, "Fine control");
    draw_switch(mem_dc, &switch_rect(3), st_snap.config.fine_control);

    // Row 5: Tap to play or pause
    draw_row_label(4, "Tap to play or pause");
    draw_switch(mem_dc, &switch_rect(4), st_snap.config.tap_to_play_pause);

    // Row 6: Require modifier (segmented control)
    draw_row_label(5, "Require modifier");
    let mod_sel = match st_snap.config.require_modifier {
        ModifierRequirement::None => 0,
        ModifierRequirement::Alt => 1,
        ModifierRequirement::Ctrl => 2,
    };
    let mod_seg_rc = RECT {
        left: s(210),
        top: s(204 + 5 * 30 + 4),
        right: s(336),
        bottom: s(204 + 5 * 30 + 24),
    };
    draw_segmented_control(mem_dc, &mod_seg_rc, &["None", "Alt", "Ctrl"], mod_sel, s(6), label_font);

    // Row 7: Quick mute
    draw_row_label(6, "Quick mute");
    draw_switch(mem_dc, &switch_rect(6), st_snap.config.quick_mute);

    // Row 8: Media strip
    draw_row_label(7, "Media strip");
    draw_switch(mem_dc, &switch_rect(7), st_snap.config.media_strip);

    // Row 9: Open at login
    draw_row_label(8, "Open at login");
    draw_switch(mem_dc, &switch_rect(8), st_snap.config.open_at_login);

    draw_divider(mem_dc, s(24), s(336), s(482));

    // 4. Sliders: Sensitivity and Edge width (Y ~ 486..558)
    // Slider 1: Sensitivity
    let mut sens_lbl = RECT { left: s(24), top: s(486), right: s(200), bottom: s(502) };
    draw_text(mem_dc, "Sensitivity", &mut sens_lbl, COLOR_TEXT_PRIMARY, row_font, DT_LEFT | DT_VCENTER | DT_SINGLELINE);

    let sens_num = format!("{:.1}x", st_snap.config.sensitivity);
    let mut sens_val = RECT { left: s(250), top: s(486), right: s(336), bottom: s(502) };
    draw_text(mem_dc, &sens_num, &mut sens_val, COLOR_TEXT_MUTED, mono_font, DT_RIGHT | DT_VCENTER | DT_SINGLELINE);

    draw_slider_bar(mem_dc, s(24), s(510), s(312), s(4), s(7), (st_snap.config.sensitivity - 0.5) / 1.5);

    // Slider 2: Edge width
    let mut edge_lbl = RECT { left: s(24), top: s(524), right: s(200), bottom: s(540) };
    draw_text(mem_dc, "Edge width", &mut edge_lbl, COLOR_TEXT_PRIMARY, row_font, DT_LEFT | DT_VCENTER | DT_SINGLELINE);

    let edge_num = format!("{}%", (st_snap.config.edge_width * 100.0).round() as u32);
    let mut edge_val = RECT { left: s(250), top: s(524), right: s(336), bottom: s(540) };
    draw_text(mem_dc, &edge_num, &mut edge_val, COLOR_TEXT_MUTED, mono_font, DT_RIGHT | DT_VCENTER | DT_SINGLELINE);

    draw_slider_bar(mem_dc, s(24), s(548), s(312), s(4), s(7), (st_snap.config.edge_width - 0.04) / 0.16);

    // 5. Footer: Quit (Y ~ 564..590)
    let mut quit_btn = RECT { left: s(260), top: s(564), right: s(336), bottom: s(590) };
    draw_text(mem_dc, "Quit", &mut quit_btn, COLOR_TEXT_MUTED, footer_font, DT_RIGHT | DT_VCENTER | DT_SINGLELINE);

    let _ = windows::Win32::Graphics::Gdi::BitBlt(
        hdc,
        0,
        0,
        client_rect.right,
        client_rect.bottom,
        mem_dc,
        0,
        0,
        windows::Win32::Graphics::Gdi::SRCCOPY,
    );

    let _ = SelectObject(mem_dc, old_bm);
    let _ = DeleteObject(hbm);
    let _ = DeleteDC(mem_dc);
    let _ = DeleteObject(title_font);
    let _ = DeleteObject(ready_font);
    let _ = DeleteObject(prompt_font);
    let _ = DeleteObject(label_font);
    let _ = DeleteObject(row_font);
    let _ = DeleteObject(mono_font);
    let _ = DeleteObject(footer_font);
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
