use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use sidewise_core::types::SliderKind;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteObject, GetDC,
    GetMonitorInfoW, MonitorFromPoint, SelectObject,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, HDC,
    MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetCursorPos,
    PeekMessageW, RegisterClassExW, SetWindowPos, ShowWindow,
    TranslateMessage, CS_HREDRAW, CS_VREDRAW, HICON, MSG, PM_REMOVE,
    SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE,
    ULW_ALPHA, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_EX_TOOLWINDOW, WS_POPUP,
};

const BASE_WIDTH: i32 = 320;
const BASE_HEIGHT: i32 = 64;
const AC_SRC_OVER: u8 = 0;
const AC_SRC_ALPHA: u8 = 1;

/// How long the popup stays fully visible after the last value update.
const HOLD_MS: u64 = 1200;
/// Exponential time constants (seconds) for the fade and the bar glide.
const FADE_IN_TAU: f32 = 0.055;
const FADE_OUT_TAU: f32 = 0.085;
const VALUE_TAU: f32 = 0.06;

// ---------------------------------------------------------------------------
// Vector glyph helpers for the OSD icon.
//
// The OSD is painted into a raw 32-bit buffer by hand, so the icons are drawn
// with signed-distance functions instead of GDI/DirectWrite. Each helper
// returns a 0..=1 coverage value for the pixel at (px, py).
// ---------------------------------------------------------------------------

/// Antialias coverage from a signed distance (negative = inside).
fn coverage(d: f32) -> f32 {
    (0.5 - d).clamp(0.0, 1.0)
}

fn sd_round_rect(px: f32, py: f32, cx: f32, cy: f32, hw: f32, hh: f32, r: f32) -> f32 {
    let dx = (px - cx).abs() - (hw - r);
    let dy = (py - cy).abs() - (hh - r);
    let ax = dx.max(0.0);
    let ay = dy.max(0.0);
    ax.hypot(ay) + dx.max(dy).min(0.0) - r
}

fn segment_distance(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let ex = bx - ax;
    let ey = by - ay;
    let len2 = ex * ex + ey * ey;
    let t = if len2 > 0.0 {
        (((px - ax) * ex + (py - ay) * ey) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let qx = ax + ex * t;
    let qy = ay + ey * t;
    (px - qx).hypot(py - qy)
}

/// Signed distance to a convex polygon (negative inside).
fn sd_convex_polygon(px: f32, py: f32, verts: &[(f32, f32)]) -> f32 {
    let n = verts.len();
    if n < 3 {
        return f32::INFINITY;
    }
    let mut ccx = 0.0;
    let mut ccy = 0.0;
    for (vx, vy) in verts {
        ccx += vx;
        ccy += vy;
    }
    ccx /= n as f32;
    ccy /= n as f32;

    let mut d = f32::NEG_INFINITY;
    for i in 0..n {
        let (ax, ay) = verts[i];
        let (bx, by) = verts[(i + 1) % n];
        let ex = bx - ax;
        let ey = by - ay;
        let len = ex.hypot(ey).max(1e-5);
        let mut nx = -ey / len;
        let mut ny = ex / len;
        // Ensure the normal points away from the centroid so the signed
        // distance is negative inside the polygon.
        if (ccx - ax) * nx + (ccy - ay) * ny > 0.0 {
            nx = -nx;
            ny = -ny;
        }
        d = d.max((px - ax) * nx + (py - ay) * ny);
    }
    d
}

/// Speaker body + cone.
fn speaker_coverage(px: f32, py: f32, cx: f32, cy: f32, s: f32) -> f32 {
    let body = sd_round_rect(px, py, cx - 6.0 * s, cy, 3.2 * s, 4.4 * s, 1.2 * s);
    let cone = sd_convex_polygon(
        px,
        py,
        &[
            (cx + 2.6 * s, cy - 9.0 * s),
            (cx - 3.0 * s, cy - 3.2 * s),
            (cx - 3.0 * s, cy + 3.2 * s),
            (cx + 2.6 * s, cy + 9.0 * s),
        ],
    );
    coverage(body.min(cone))
}

/// Two sound waves to the right of the speaker.
fn waves_coverage(px: f32, py: f32, cx: f32, cy: f32, s: f32) -> f32 {
    let origin_x = cx + 2.6 * s;
    let ang = (py - cy).atan2(px - origin_x);
    let fade = ((0.95 - ang.abs()) / 0.18).clamp(0.0, 1.0);
    if fade <= 0.0 {
        return 0.0;
    }
    let dist = (px - origin_x).hypot(py - cy);
    let mut m = 0.0f32;
    for radius in [5.4 * s, 9.0 * s] {
        let d = (dist - radius).abs() - 0.85 * s;
        m = m.max(coverage(d) * fade);
    }
    m
}

/// Red X used when audio is muted.
fn cross_coverage(px: f32, py: f32, cx: f32, cy: f32, s: f32) -> f32 {
    let half = 3.2 * s;
    let th = 0.9 * s;
    let d1 = segment_distance(px, py, cx - half, cy - half, cx + half, cy + half) - th;
    let d2 = segment_distance(px, py, cx - half, cy + half, cx + half, cy - half) - th;
    coverage(d1.min(d2))
}

/// Sun disc plus eight rays.
fn sun_coverage(px: f32, py: f32, cx: f32, cy: f32, s: f32) -> f32 {
    let d = (px - cx).hypot(py - cy) - 4.6 * s;
    let mut m = coverage(d);
    for i in 0..8 {
        let a = i as f32 * std::f32::consts::FRAC_PI_4;
        let (sin, cos) = a.sin_cos();
        let inner = 7.0 * s;
        let outer = 9.7 * s;
        let th = 0.8 * s;
        let d = segment_distance(
            px,
            py,
            cx + inner * cos,
            cy + inner * sin,
            cx + outer * cos,
            cy + outer * sin,
        ) - th;
        m = m.max(coverage(d));
    }
    m
}

fn blend_pixel(r: &mut u32, g: &mut u32, b: &mut u32, cr: u32, cg: u32, cb: u32, mask: f32) {
    let m = mask.clamp(0.0, 1.0);
    if m <= 0.0 {
        return;
    }
    let inv = 1.0 - m;
    *r = (*r as f32 * inv + cr as f32 * m) as u32;
    *g = (*g as f32 * inv + cg as f32 * m) as u32;
    *b = (*b as f32 * inv + cb as f32 * m) as u32;
}

#[derive(Debug, Clone)]
struct OsdDisplayData {
    kind: SliderKind,
    value: f32,
    muted: bool,
}

pub struct OsdController {
    data: Arc<Mutex<(Option<OsdDisplayData>, bool)>>, // (data, requested_update)
}

impl Default for OsdController {
    fn default() -> Self {
        Self::new()
    }
}

impl OsdController {
    pub fn new() -> Self {
        let data = Arc::new(Mutex::new((None, false)));
        let data_clone = Arc::clone(&data);

        thread::Builder::new()
            .name("osd-window".into())
            .spawn(move || {
                run_osd_loop(data_clone);
            })
            .expect("Failed to start OSD thread");

        Self { data }
    }

    pub fn show(&self, kind: SliderKind, value: f32, muted: bool) {
        if let Ok(mut lock) = self.data.lock() {
            lock.0 = Some(OsdDisplayData { kind, value, muted });
            lock.1 = true;
        }
    }
}

fn run_osd_loop(shared_data: Arc<Mutex<(Option<OsdDisplayData>, bool)>>) {
    unsafe {
        let class_name = w!("SidewiseOsdWindow");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(osd_wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default().into(),
            hIcon: HICON::default(),
            hCursor: windows::Win32::UI::WindowsAndMessaging::LoadCursorW(None, windows::Win32::UI::WindowsAndMessaging::IDC_ARROW).unwrap_or_default(),
            hbrBackground: windows::Win32::Graphics::Gdi::HBRUSH::default(),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: class_name,
            hIconSm: HICON::default(),
        };
        let _ = RegisterClassExW(&wc);

        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT,
            class_name,
            w!("Sidewise OSD"),
            WS_POPUP,
            0,
            0,
            BASE_WIDTH,
            BASE_HEIGHT,
            HWND::default(),
            None,
            windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default(),
            None,
        );

        let hwnd = match hwnd {
            Ok(h) if !h.0.is_null() => h,
            _ => return,
        };

        // Animation state. Opacity and the displayed level are both eased every
        // frame with time-based exponential smoothing, so rapid back-to-back
        // volume/brightness updates blend into one fluid motion instead of
        // snapping from step to step.
        let mut visible = false;
        let mut alpha_f = 0.0f32;
        let mut display_value = 0.0f32;
        let mut hold_until: Option<Instant> = None;
        let mut current_data: Option<OsdDisplayData> = None;
        let mut last_tick = Instant::now();

        let screen_dc = GetDC(HWND::default());
        let mem_dc = CreateCompatibleDC(screen_dc);

        loop {
            // Process window messages
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, hwnd, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }

            // Frame delta, clamped so a stall can't make the easing jump.
            let dt = last_tick.elapsed().as_secs_f32().min(0.05);
            last_tick = Instant::now();

            // Check for new trigger
            let mut trigger_new = false;
            if let Ok(mut lock) = shared_data.lock() {
                if lock.1 {
                    current_data = lock.0.clone();
                    lock.1 = false;
                    trigger_new = true;
                }
            }

            if trigger_new {
                if !visible {
                    // Fresh appearance: place the pill, start fully
                    // transparent, and show the current level immediately
                    // (there is no previous value to glide from).
                    position_and_show(hwnd);
                    visible = true;
                    alpha_f = 0.0;
                    if let Some(ref d) = current_data {
                        display_value = d.value.clamp(0.0, 1.0);
                    }
                }
                // New input keeps the popup up; it never restarts the fade,
                // so continuous changes stay perfectly steady.
                hold_until = Some(Instant::now() + Duration::from_millis(HOLD_MS));
            }

            if !visible {
                thread::sleep(Duration::from_millis(30));
                continue;
            }

            // Smoothly fade towards 1 while held, towards 0 once released.
            let now = Instant::now();
            let target_alpha = match hold_until {
                Some(t) if now < t => 1.0f32,
                _ => 0.0f32,
            };
            let tau = if target_alpha > alpha_f {
                FADE_IN_TAU
            } else {
                FADE_OUT_TAU
            };
            alpha_f += (target_alpha - alpha_f) * (1.0 - (-dt / tau).exp());

            if target_alpha == 0.0 && alpha_f < 0.02 {
                visible = false;
                alpha_f = 0.0;
                hold_until = None;
                let _ = ShowWindow(hwnd, SW_HIDE);
                continue;
            }

            if let Some(ref data) = current_data {
                // Ease the bar towards the latest level. Using
                // 1 - e^(-dt/tau) keeps the motion identical regardless of
                // render frame rate, and interruptions stay smooth.
                let target = data.value.clamp(0.0, 1.0);
                let k = 1.0 - (-dt / VALUE_TAU).exp();
                display_value += (target - display_value) * k;
                if (target - display_value).abs() < 0.0005 {
                    display_value = target;
                }

                let alpha = (alpha_f.clamp(0.0, 1.0) * 240.0).round() as u8;
                render_osd(
                    hwnd,
                    mem_dc,
                    screen_dc,
                    data,
                    display_value,
                    alpha,
                );
            }

            thread::sleep(Duration::from_millis(16));
        }
    }
}

unsafe fn position_and_show(hwnd: HWND) {
    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);

    let hmon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if GetMonitorInfoW(hmon, &mut mi).as_bool() {
        let mon_width = mi.rcWork.right - mi.rcWork.left;

        let dpi = GetDpiForWindow(hwnd).max(96) as f32;
        let scale = dpi / 96.0;
        let width = (BASE_WIDTH as f32 * scale) as i32;
        let height = (BASE_HEIGHT as f32 * scale) as i32;

        let x = mi.rcWork.left + (mon_width - width) / 2;
        let y = mi.rcWork.bottom - height - (60.0 * scale) as i32;

        let _ = SetWindowPos(
            hwnd,
            windows::Win32::UI::WindowsAndMessaging::HWND_TOPMOST,
            x,
            y,
            width,
            height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
    }
}

unsafe fn render_osd(
    hwnd: HWND,
    mem_dc: HDC,
    screen_dc: HDC,
    data: &OsdDisplayData,
    value: f32,
    alpha: u8,
) {
    let dpi = GetDpiForWindow(hwnd).max(96) as f32;
    let scale = dpi / 96.0;
    let width = (BASE_WIDTH as f32 * scale) as i32;
    let height = (BASE_HEIGHT as f32 * scale) as i32;

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
    let hbitmap = CreateDIBSection(
        screen_dc,
        &bmi,
        DIB_RGB_COLORS,
        &mut bits as *mut *mut u8 as *mut *mut std::ffi::c_void,
        windows::Win32::Foundation::HANDLE::default(),
        0,
    );

    if hbitmap.is_err() || bits.is_null() {
        return;
    }
    let hbitmap = hbitmap.unwrap();
    let old_obj = SelectObject(mem_dc, hbitmap);

    // Render 32-bit ARGB software buffer for high quality rounded pill
    let slice = std::slice::from_raw_parts_mut(bits as *mut u32, (width * height) as usize);
    let corner_r = (18.0 * scale) as f32;

    let bar_left = 64.0 * scale;
    let bar_right = (width as f32) - 60.0 * scale;
    let bar_cy = height as f32 / 2.0;
    let bar_half_h = 4.0 * scale;
    let bar_width = bar_right - bar_left;
    let bar_half_w = bar_width / 2.0;
    let fill_w = bar_width * value.clamp(0.0, 1.0);

    for y in 0..height {
        for x in 0..width {
            let idx = (y * width + x) as usize;
            let fx = x as f32;
            let fy = y as f32;

            // Rounded pill signed distance
            let dx = (fx - (width as f32 / 2.0)).abs() - (width as f32 / 2.0 - corner_r);
            let dy = (fy - (height as f32 / 2.0)).abs() - (height as f32 / 2.0 - corner_r);
            let d = (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt() + dx.max(dy).min(0.0) - corner_r;

            if d > 1.0 {
                slice[idx] = 0; // outside
                continue;
            }

            let antialias = (1.0 - d).clamp(0.0, 1.0);
            let bg_a = (antialias * 230.0) as u32;

            // Modern Dark acrylic background (R=32, G=32, B=32)
            let mut r = 32u32;
            let mut g = 32u32;
            let mut b = 32u32;
            let a = bg_a;

            // Progress bar: rounded track with a rounded fill whose width
            // follows the eased level, so even tiny changes glide.
            let track_d = sd_round_rect(
                fx,
                fy,
                bar_left + bar_half_w,
                bar_cy,
                bar_half_w,
                bar_half_h,
                bar_half_h,
            );
            let track_cov = (0.5 - track_d).clamp(0.0, 1.0);
            if track_cov > 0.0 {
                blend_pixel(&mut r, &mut g, &mut b, 70, 70, 70, track_cov);
            }

            if fill_w > 0.5 {
                let fill_half = fill_w / 2.0;
                let fill_r = bar_half_h.min(fill_half);
                let fill_d = sd_round_rect(
                    fx,
                    fy,
                    bar_left + fill_half,
                    bar_cy,
                    fill_half,
                    bar_half_h,
                    fill_r,
                );
                let fill_cov = (0.5 - fill_d).clamp(0.0, 1.0);
                if fill_cov > 0.0 {
                    blend_pixel(&mut r, &mut g, &mut b, 96, 165, 250, fill_cov);
                }
            }

            // Icon glyph (left side)
            let icon_cx = 32.0 * scale;
            let icon_cy = height as f32 / 2.0;
            match data.kind {
                SliderKind::Volume => {
                    let speaker = speaker_coverage(fx, fy, icon_cx, icon_cy, scale);
                    blend_pixel(&mut r, &mut g, &mut b, 240, 240, 240, speaker);
                    if data.muted {
                        let cross = cross_coverage(fx, fy, icon_cx + 10.5 * scale, icon_cy, scale);
                        blend_pixel(&mut r, &mut g, &mut b, 239, 68, 68, cross);
                    } else {
                        let waves = waves_coverage(fx, fy, icon_cx, icon_cy, scale);
                        blend_pixel(&mut r, &mut g, &mut b, 240, 240, 240, waves);
                    }
                }
                SliderKind::Brightness => {
                    let sun = sun_coverage(fx, fy, icon_cx, icon_cy, scale);
                    blend_pixel(&mut r, &mut g, &mut b, 250, 204, 21, sun);
                }
            }

            // Premultiplied alpha for UpdateLayeredWindow
            let pr = (r * a) / 255;
            let pg = (g * a) / 255;
            let pb = (b * a) / 255;
            slice[idx] = (a << 24) | (pr << 16) | (pg << 8) | pb;
        }
    }

    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER,
        BlendFlags: 0,
        SourceConstantAlpha: alpha,
        AlphaFormat: AC_SRC_ALPHA,
    };

    let pt_src = POINT { x: 0, y: 0 };
    let size = SIZE { cx: width, cy: height };

    let _ = windows::Win32::UI::WindowsAndMessaging::UpdateLayeredWindow(
        hwnd,
        screen_dc,
        None,
        Some(&size),
        mem_dc,
        Some(&pt_src),
        COLORREF(0),
        Some(&blend),
        ULW_ALPHA,
    );

    let _ = SelectObject(mem_dc, old_obj);
    let _ = DeleteObject(hbitmap);
}

unsafe extern "system" fn osd_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    DefWindowProcW(hwnd, msg, wparam, lparam)
}
