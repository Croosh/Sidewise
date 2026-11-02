use windows::Win32::Foundation::{COLORREF, HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    AddFontResourceExW, CreateFontW, CreatePen, CreateSolidBrush, DeleteObject, DrawTextW,
    FillRect, RoundRect, SelectObject, SetBkMode, SetTextColor, HDC, HFONT, PS_SOLID,
    TRANSPARENT, FR_PRIVATE,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::core::{w, PCWSTR};
use std::os::windows::ffi::OsStrExt;
use std::sync::Once;

// Warm off-white redesign palette (matching Main.dc.html & Settings.dc.html)
// Note: In Win32 GDI, COLORREF is 0x00BBGGRR format.
pub const COLOR_BG: COLORREF = COLORREF(0x00F8FAFB);          // #FBFAF8
pub const COLOR_CARD: COLORREF = COLORREF(0x00E4E8E9);        // #E9E8E4 (surface)
pub const COLOR_BORDER: COLORREF = COLORREF(0x00DAE1E4);      // #E4E1DA (subtle borders/dividers)
pub const COLOR_BORDER_CARD: COLORREF = COLORREF(0x00C3CBCF); // #CFCBC3 (trackpad card border)
pub const COLOR_TEXT_PRIMARY: COLORREF = COLORREF(0x00191B1B); // #1B1B19
pub const COLOR_TEXT_MUTED: COLORREF = COLORREF(0x00666C6E);   // #6E6C66
pub const COLOR_TEXT_LABEL: COLORREF = COLORREF(0x007F878A);   // #8A877F (small uppercase labels)
pub const COLOR_ACCENT_TEAL: COLORREF = COLORREF(0x005F6A2D);  // #2D6A5F (Pine Teal)
pub const COLOR_ACCENT_TINT: COLORREF = COLORREF(0x00D6E2DE);  // Light teal tint over #E9E8E4
pub const COLOR_TRACK: COLORREF = COLORREF(0x00CCD5D9);        // #D9D5CC (inactive switch track / slider rail)
pub const COLOR_SEGMENT_BG: COLORREF = COLORREF(0x00E5EBEE);   // #EEEBE5 (segmented control track)
pub const COLOR_WHITE: COLORREF = COLORREF(0x00FFFFFF);        // #FFFFFF
pub const COLOR_DOT_BORDER: COLORREF = COLORREF(0x00B2BDB9);   // Outer dot ring tint

// Legacy mappings for backward compatibility if referenced
pub const COLOR_ACCENT_BLUE: COLORREF = COLOR_ACCENT_TEAL;
pub const COLOR_ACCENT_AMBER: COLORREF = COLOR_ACCENT_TEAL;
pub const COLOR_ACCENT_GREEN: COLORREF = COLORREF(0x005EC522);

pub fn get_window_scale(hwnd: HWND) -> f32 {
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 {
        1.0
    } else {
        dpi as f32 / 96.0
    }
}

// IBM Plex Mono (OFL) is bundled so the UI font is identical on every machine.
// Windows only lets CreateFontW use it after the file has been registered with
// AddFontResourceExW; FR_PRIVATE keeps it scoped to this process.
const FONT_REGULAR: &[u8] = include_bytes!("../assets/IBMPlexMono-Regular.ttf");
const FONT_MEDIUM: &[u8] = include_bytes!("../assets/IBMPlexMono-Medium.ttf");
const FONT_SEMIBOLD: &[u8] = include_bytes!("../assets/IBMPlexMono-SemiBold.ttf");
static FONT_INIT: Once = Once::new();

fn ensure_ui_font_loaded() {
    FONT_INIT.call_once(|| unsafe {
        let dir = std::env::temp_dir().join("Sidewise").join("fonts");
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        for (file, bytes) in [
            ("IBMPlexMono-Regular.ttf", FONT_REGULAR),
            ("IBMPlexMono-Medium.ttf", FONT_MEDIUM),
            ("IBMPlexMono-SemiBold.ttf", FONT_SEMIBOLD),
        ] {
            let path = dir.join(file);
            let up_to_date = std::fs::read(&path)
                .map(|existing| existing == bytes)
                .unwrap_or(false);
            if !up_to_date && std::fs::write(&path, bytes).is_err() {
                continue;
            }
            let wide: Vec<u16> = path
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            AddFontResourceExW(PCWSTR(wide.as_ptr()), FR_PRIVATE, None);
        }
    });
}

pub unsafe fn create_ui_font(point_size: i32, weight: i32, scale: f32, _monospace: bool) -> HFONT {
    ensure_ui_font_loaded();
    let scaled_height = -((point_size as f32 * scale * 96.0 / 72.0).round() as i32);
    let face = w!("IBM Plex Mono");

    CreateFontW(
        scaled_height,
        0,
        0,
        0,
        weight,
        0,
        0,
        0,
        windows::Win32::Graphics::Gdi::DEFAULT_CHARSET.0 as u32,
        windows::Win32::Graphics::Gdi::OUT_DEFAULT_PRECIS.0 as u32,
        windows::Win32::Graphics::Gdi::CLIP_DEFAULT_PRECIS.0 as u32,
        5, // CLEARTYPE_QUALITY
        windows::Win32::Graphics::Gdi::DEFAULT_PITCH.0 as u32,
        face,
    )
}

pub unsafe fn draw_text(
    hdc: HDC,
    text: &str,
    rect: &mut RECT,
    color: COLORREF,
    font: HFONT,
    flags: windows::Win32::Graphics::Gdi::DRAW_TEXT_FORMAT,
) {
    let mut wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let old_font = SelectObject(hdc, font);
    SetTextColor(hdc, color);
    SetBkMode(hdc, TRANSPARENT);
    let len = wide.len() - 1;
    let _ = DrawTextW(hdc, &mut wide[..len], rect as *mut RECT, flags);
    let _ = SelectObject(hdc, old_font);
}

pub unsafe fn fill_round_rect(
    hdc: HDC,
    rect: &RECT,
    bg_color: COLORREF,
    border_color: COLORREF,
    radius: i32,
) {
    let brush = CreateSolidBrush(bg_color);
    let pen = CreatePen(PS_SOLID, 1, border_color);
    let old_brush = SelectObject(hdc, brush);
    let old_pen = SelectObject(hdc, pen);

    let _ = RoundRect(
        hdc,
        rect.left,
        rect.top,
        rect.right,
        rect.bottom,
        radius,
        radius,
    );

    let _ = SelectObject(hdc, old_pen);
    let _ = SelectObject(hdc, old_brush);
    let _ = DeleteObject(pen);
    let _ = DeleteObject(brush);
}

pub unsafe fn draw_switch(hdc: HDC, rect: &RECT, active: bool) {
    let bg_color = if active { COLOR_ACCENT_TEAL } else { COLOR_TRACK };
    let border_color = if active { COLOR_ACCENT_TEAL } else { COLOR_BORDER_CARD };
    let radius = rect.bottom - rect.top; // Full pill rounding

    fill_round_rect(hdc, rect, bg_color, border_color, radius);

    let pad = 2;
    let knob_diam = (rect.bottom - rect.top) - (pad * 2);
    let knob_left = if active {
        rect.right - pad - knob_diam
    } else {
        rect.left + pad
    };

    let knob_rect = RECT {
        left: knob_left,
        top: rect.top + pad,
        right: knob_left + knob_diam,
        bottom: rect.bottom - pad,
    };

    fill_round_rect(hdc, &knob_rect, COLOR_WHITE, COLOR_BORDER, knob_diam);
}

pub unsafe fn draw_segmented_control(
    hdc: HDC,
    rect: &RECT,
    items: &[&str],
    selected_index: usize,
    radius: i32,
    font: HFONT,
) {
    // Background track
    fill_round_rect(hdc, rect, COLOR_SEGMENT_BG, COLOR_BORDER, radius);

    let count = items.len() as i32;
    if count == 0 {
        return;
    }

    let inner_pad = 2;
    let total_width = (rect.right - rect.left) - (inner_pad * 2);
    let seg_w = total_width / count;
    let top = rect.top + inner_pad;
    let bottom = rect.bottom - inner_pad;

    for (i, item) in items.iter().enumerate() {
        let left = rect.left + inner_pad + (i as i32 * seg_w);
        let right = if i == items.len() - 1 {
            rect.right - inner_pad
        } else {
            left + seg_w
        };

        let mut seg_rect = RECT { left, top, right, bottom };

        if i == selected_index {
            // Active segment: white rounded pill with subtle border
            fill_round_rect(hdc, &seg_rect, COLOR_WHITE, COLOR_BORDER, (radius - 2).max(4));
            draw_text(
                hdc,
                item,
                &mut seg_rect,
                COLOR_TEXT_PRIMARY,
                font,
                windows::Win32::Graphics::Gdi::DT_CENTER
                    | windows::Win32::Graphics::Gdi::DT_VCENTER
                    | windows::Win32::Graphics::Gdi::DT_SINGLELINE,
            );
        } else {
            draw_text(
                hdc,
                item,
                &mut seg_rect,
                COLOR_TEXT_MUTED,
                font,
                windows::Win32::Graphics::Gdi::DT_CENTER
                    | windows::Win32::Graphics::Gdi::DT_VCENTER
                    | windows::Win32::Graphics::Gdi::DT_SINGLELINE,
            );
        }
    }
}

pub unsafe fn draw_slider_bar(hdc: HDC, x: i32, y: i32, width: i32, track_h: i32, knob_radius: i32, ratio: f32) {
    let clamped = ratio.clamp(0.0, 1.0);
    let track_rect = RECT {
        left: x,
        top: y,
        right: x + width,
        bottom: y + track_h,
    };
    let t_brush = CreateSolidBrush(COLOR_TRACK);
    FillRect(hdc, &track_rect, t_brush);
    let _ = DeleteObject(t_brush);

    let fill_w = (width as f32 * clamped) as i32;
    if fill_w > 0 {
        let fill_rect = RECT {
            left: x,
            top: y,
            right: x + fill_w,
            bottom: y + track_h,
        };
        let f_brush = CreateSolidBrush(COLOR_ACCENT_TEAL);
        FillRect(hdc, &fill_rect, f_brush);
        let _ = DeleteObject(f_brush);
    }

    let knob_cx = x + fill_w;
    let knob_cy = y + track_h / 2;
    let knob_rect = RECT {
        left: knob_cx - knob_radius,
        top: knob_cy - knob_radius,
        right: knob_cx + knob_radius,
        bottom: knob_cy + knob_radius,
    };
    fill_round_rect(hdc, &knob_rect, COLOR_WHITE, COLOR_BORDER_CARD, knob_radius * 2);
}

pub unsafe fn draw_logo_badge(hdc: HDC, x: i32, y: i32, target_size: i32) {
    use crate::logo_badge::{LOGO_BADGE_BGRA, LOGO_BADGE_SIZE};
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject,
        BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS,
    };
    use windows::Win32::Graphics::Gdi::AlphaBlend;

    let mem_dc = CreateCompatibleDC(hdc);
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: LOGO_BADGE_SIZE,
            biHeight: -LOGO_BADGE_SIZE, // top-down
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

    if let Ok(bm) = hbm {
        if !bits.is_null() {
            std::ptr::copy_nonoverlapping(LOGO_BADGE_BGRA.as_ptr(), bits, LOGO_BADGE_BGRA.len());
            let old_bm = SelectObject(mem_dc, bm);

            let blend = BLENDFUNCTION {
                BlendOp: 0, // AC_SRC_OVER
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: 1, // AC_SRC_ALPHA
            };

            let _ = AlphaBlend(
                hdc,
                x,
                y,
                target_size,
                target_size,
                mem_dc,
                0,
                0,
                LOGO_BADGE_SIZE,
                LOGO_BADGE_SIZE,
                blend,
            );

            let _ = SelectObject(mem_dc, old_bm);
        }
        let _ = DeleteObject(bm);
    }
    let _ = DeleteDC(mem_dc);
}
