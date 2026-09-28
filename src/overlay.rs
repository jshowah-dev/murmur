use anyhow::{anyhow, Result};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    COLORREF, ERROR_CLASS_ALREADY_EXISTS, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject, DrawTextW, GdiFlush, GetDC, GetTextMetricsW,
    ReleaseDC, SelectObject, SetBkMode, SetTextColor, ANTIALIASED_QUALITY, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, DIB_RGB_COLORS, DT_CALCRECT, DT_NOPREFIX, DT_WORDBREAK, FF_SWISS,
    FW_NORMAL, HBITMAP, OUT_DEFAULT_PRECIS, TEXTMETRICW, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::Foundation::GetLastError;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetForegroundWindow, GetWindowRect, PeekMessageW, RegisterClassW,
    SetWindowPos, ShowWindow, TranslateMessage, UpdateLayeredWindow, HWND_TOPMOST, MSG, PM_REMOVE, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SW_SHOWNOACTIVATE, ULW_ALPHA, WM_QUIT, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::Win32::Graphics::Gdi::{MonitorFromWindow, GetMonitorInfoW, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::Graphics::Gdi::AC_SRC_ALPHA;
use windows::Win32::Graphics::Gdi::BLENDFUNCTION;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OverlayState {
    /// Always-on resting pill: small, dim, green dot.
    Idle,
    /// Resting pill while dictation is paused from the tray: small, dim, grey dot.
    Paused,
    Listening(f32),
    /// Hands-free listening: level bar plus a red dot, the key can be let go.
    Locked(f32),
    Processing,
}

impl OverlayState {
    fn is_resting(self) -> bool {
        matches!(self, OverlayState::Idle | OverlayState::Paused)
    }
    /// (width, height, body alpha) — the resting pill is small and dim, live states are full size.
    fn geometry(self) -> (i32, i32, u32) {
        if self.is_resting() { (IDLE_W, IDLE_H, 0x80) } else { (W, H, 0xE6) }
    }
}

const W: i32 = 160;
const H: i32 = 36;
const IDLE_W: i32 = 56;
const IDLE_H: i32 = 14;
/// Live-text panel above the pill: fixed width, grows to MAX_LINES and then shows the newest lines.
const PANEL_W: i32 = 560;
const PANEL_PAD: i32 = 12;
const PANEL_GAP: i32 = 8;
const MAX_LINES: i32 = 3;
const FONT_PX: i32 = 17;

/// Word-wrapped text as per-pixel coverage (0-255), laid out once per text change.
struct TextLayer {
    w: i32,
    h: i32,
    cov: Vec<u8>,
}

pub struct Overlay {
    hwnd: HWND,
    state: OverlayState,
    tick: u32,
    text: Option<TextLayer>,
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

impl Overlay {
    pub fn create() -> Result<Overlay> {
        unsafe {
            let hinst = GetModuleHandleW(None)?;
            let class = wide("MurmurOverlay");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: hinst.into(),
                lpszClassName: PCWSTR(class.as_ptr()),
                ..Default::default()
            };
            if RegisterClassW(&wc) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
                return Err(anyhow!("RegisterClassW"));
            }
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT,
                PCWSTR(class.as_ptr()),
                PCWSTR(wide("Murmur").as_ptr()),
                WS_POPUP,
                0, 0, W, H,
                None, None, Some(hinst.into()), None,
            )?;
            Ok(Overlay { hwnd, state: OverlayState::Idle, tick: 0, text: None })
        }
    }

    /// Bottom-centre of the work area on the foreground window's monitor; the pill's
    /// bottom edge stays put as it grows from the resting size to the live size.
    fn target_position(&self, w: i32, h: i32) -> (i32, i32) {
        unsafe {
            let fg = GetForegroundWindow();
            let mon = MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST);
            let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            let r: RECT = if GetMonitorInfoW(mon, &mut mi).as_bool() { mi.rcWork } else { RECT { left: 0, top: 0, right: 1920, bottom: 1080 } };
            let _ = GetWindowRect(fg, &mut RECT::default());
            ((r.left + r.right) / 2 - w / 2, r.bottom - h - 24)
        }
    }

    /// Paint the pill into a 32-bit DIB and push it with UpdateLayeredWindow.
    fn paint(&mut self) {
        let (w, h, pixels) = render(self.state, self.tick, self.text.as_ref());
        unsafe {
            let screen = GetDC(None);
            let mem = CreateCompatibleDC(Some(screen));
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let bmp: HBITMAP = CreateDIBSection(Some(mem), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).unwrap_or_default();
            if bmp.is_invalid() || bits.is_null() {
                let _ = DeleteDC(mem);
                ReleaseDC(None, screen);
                return;
            }
            let old = SelectObject(mem, bmp.into());

            std::slice::from_raw_parts_mut(bits as *mut u32, pixels.len()).copy_from_slice(&pixels);

            let (x, y) = self.target_position(w, h);
            let blend = BLENDFUNCTION { BlendOp: 0, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
            let _ = UpdateLayeredWindow(
                self.hwnd,
                Some(screen),
                Some(&POINT { x, y }),
                Some(&SIZE { cx: w, cy: h }),
                Some(mem),
                Some(&POINT { x: 0, y: 0 }),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            );
            let _ = SelectObject(mem, old);
            let _ = DeleteObject(bmp.into());
            let _ = DeleteDC(mem);
            ReleaseDC(None, screen);
        }
    }

    pub fn set(&mut self, state: OverlayState) {
        self.state = state;
        if state.is_resting() {
            self.text = None;
        }
        self.tick = self.tick.wrapping_add(1);
        self.paint();
        unsafe {
            // re-assert topmost on every state change so a fullscreen app cannot bury the pill
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOSIZE | SWP_NOMOVE | SWP_NOACTIVATE);
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
        }
    }

    /// Show `text` above the pill while recording or processing; ignored at rest.
    pub fn set_text(&mut self, text: &str) {
        if self.state.is_resting() {
            return;
        }
        self.text = layout_text(text);
        self.paint();
    }

    /// Re-paint the resting pill so it follows the foreground window's monitor.
    pub fn refresh_resting(&mut self) {
        if self.state.is_resting() {
            self.paint();
        }
    }

    /// Drain pending messages for this thread. Returns false on WM_QUIT.
    pub fn pump_once(&mut self) -> bool {
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    return false;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        true
    }
}

/// Premultiplied RGBA canvas; shapes are drawn with per-pixel coverage so edges are antialiased.
struct Canvas {
    w: i32,
    h: i32,
    px: Vec<[f32; 4]>,
}

impl Canvas {
    fn new(w: i32, h: i32) -> Self {
        Canvas { w, h, px: vec![[0.0; 4]; (w * h) as usize] }
    }

    /// Composite a capsule (rounded rect, radius = half the short side) of 0xRRGGBB at `alpha` over the canvas.
    fn capsule(&mut self, x: f32, y: f32, cw: f32, ch: f32, rgb: u32, alpha: f32) {
        self.rounded(x, y, cw, ch, cw.min(ch) / 2.0, rgb, alpha);
    }

    /// Composite a rounded rect with corner radius `r`.
    #[allow(clippy::too_many_arguments)]
    fn rounded(&mut self, x: f32, y: f32, cw: f32, ch: f32, r: f32, rgb: u32, alpha: f32) {
        let (cx, cy) = (x + cw / 2.0, y + ch / 2.0);
        let (bx, by) = (cw / 2.0 - r, ch / 2.0 - r);
        let col = [(rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF].map(|c| c as f32 / 255.0);
        for py in (y.floor() as i32).max(0)..((y + ch).ceil() as i32).min(self.h) {
            for pxl in (x.floor() as i32).max(0)..((x + cw).ceil() as i32).min(self.w) {
                // signed distance from the pixel centre to the capsule edge
                let qx = (pxl as f32 + 0.5 - cx).abs() - bx;
                let qy = (py as f32 + 0.5 - cy).abs() - by;
                let d = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r;
                let a = (0.5 - d).clamp(0.0, 1.0) * alpha;
                if a > 0.0 {
                    let dst = &mut self.px[(py * self.w + pxl) as usize];
                    for i in 0..3 {
                        dst[i] = col[i] * a + dst[i] * (1.0 - a);
                    }
                    dst[3] = a + dst[3] * (1.0 - a);
                }
            }
        }
    }

    /// Composite `layer` in 0xRRGGBB with its top-left corner at (x, y).
    fn text(&mut self, x: i32, y: i32, layer: &TextLayer, rgb: u32) {
        let col = [(rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF].map(|c| c as f32 / 255.0);
        for ty in 0..layer.h {
            for tx in 0..layer.w {
                let a = layer.cov[(ty * layer.w + tx) as usize] as f32 / 255.0;
                let (px, py) = (x + tx, y + ty);
                if a > 0.0 && px >= 0 && px < self.w && py >= 0 && py < self.h {
                    let dst = &mut self.px[(py * self.w + px) as usize];
                    for i in 0..3 {
                        dst[i] = col[i] * a + dst[i] * (1.0 - a);
                    }
                    dst[3] = a + dst[3] * (1.0 - a);
                }
            }
        }
    }

    /// Pack as premultiplied BGRA (0xAARRGGBB little-endian), what UpdateLayeredWindow expects.
    fn into_bgra(self) -> Vec<u32> {
        let q = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u32;
        self.px.into_iter().map(|[r, g, b, a]| (q(a) << 24) | (q(r) << 16) | (q(g) << 8) | q(b)).collect()
    }
}

/// Paint `state`, with `text` in a panel above the pill when there is any, into premultiplied
/// BGRA pixels, row-major, `w * h` long. The pill is bottom-centred.
fn render(state: OverlayState, tick: u32, text: Option<&TextLayer>) -> (i32, i32, Vec<u32>) {
    let (pw, ph, alpha) = state.geometry();
    let text = text.filter(|_| !state.is_resting());
    let panel_h = text.map_or(0, |t| t.h + 2 * PANEL_PAD);
    let w = if text.is_some() { PANEL_W.max(pw) } else { pw };
    let h = if text.is_some() { panel_h + PANEL_GAP + ph } else { ph };
    let mut c = Canvas::new(w, h);
    if let Some(t) = text {
        c.rounded(0.0, 0.0, w as f32, panel_h as f32, 10.0, 0x202020, alpha as f32 / 255.0);
        c.text(PANEL_PAD, PANEL_PAD, t, 0xF0F0F0);
    }
    let (x0, y0) = (((w - pw) / 2) as f32, (h - ph) as f32);
    let (wf, hf) = (pw as f32, ph as f32);
    // body: dim when resting, near-opaque when live
    c.capsule(x0, y0, wf, hf, 0x202020, alpha as f32 / 255.0);
    match state {
        OverlayState::Idle | OverlayState::Paused => {
            let rgb = if state == OverlayState::Idle { 0x60D060 } else { 0x808080 };
            let d = hf - 8.0;
            c.capsule(x0 + wf / 2.0 - d / 2.0, y0 + 4.0, d, d, rgb, 1.0);
        }
        OverlayState::Listening(level) | OverlayState::Locked(level) => {
            let locked = matches!(state, OverlayState::Locked(_));
            // the locked bar stops short of the dot at the right end
            let span = if locked { W - 58 } else { W - 40 } as f32;
            let bar_w = (span * level.clamp(0.0, 1.0)).max(6.0);
            c.capsule(x0 + 20.0, y0 + hf / 2.0 - 3.0, bar_w, 6.0, 0x60D060, 1.0);
            if locked {
                let d = 10.0;
                c.capsule(x0 + wf - 20.0 - d, y0 + hf / 2.0 - d / 2.0, d, d, 0xE04040, 1.0);
            }
        }
        OverlayState::Processing => {
            let on = (tick / 4) % 3;
            for i in 0..3 {
                let rgb = if i == on { 0xFFFFFF } else { 0x707070 };
                c.capsule(x0 + wf / 2.0 - 18.0 + i as f32 * 14.0, y0 + hf / 2.0 - 3.0, 6.0, 6.0, rgb, 1.0);
            }
        }
    }
    (w, h, c.into_bgra())
}

/// Lay `text` out word-wrapped to the panel's width with GDI, white on black, and keep the
/// brightness as coverage. Taller than MAX_LINES, only the bottom lines are kept so the newest
/// words stay in view. None for empty text or if GDI fails.
fn layout_text(text: &str) -> Option<TextLayer> {
    let mut wtext: Vec<u16> = text.trim().encode_utf16().collect();
    if wtext.is_empty() {
        return None;
    }
    let w = PANEL_W - 2 * PANEL_PAD;
    unsafe {
        let dc = CreateCompatibleDC(None);
        if dc.is_invalid() {
            return None;
        }
        // grayscale antialiasing: ClearType's coloured fringes would read as uneven coverage
        let font = CreateFontW(
            -FONT_PX, 0, 0, 0, FW_NORMAL.0 as i32, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS,
            ANTIALIASED_QUALITY, (DEFAULT_PITCH.0 | FF_SWISS.0) as u32, windows::core::w!("Segoe UI"),
        );
        let old_font = SelectObject(dc, font.into());
        let mut tm = TEXTMETRICW::default();
        let _ = GetTextMetricsW(dc, &mut tm);
        let line = (tm.tmHeight + tm.tmExternalLeading).max(1);
        let mut r = RECT { left: 0, top: 0, right: w, bottom: 0 };
        DrawTextW(dc, &mut wtext, &mut r, DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX);
        let full = r.bottom.max(line);
        let h = full.min(line * MAX_LINES);
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let bmp: HBITMAP = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).unwrap_or_default();
        let mut layer = None;
        if !bmp.is_invalid() && !bits.is_null() {
            let old_bmp = SelectObject(dc, bmp.into());
            SetBkMode(dc, TRANSPARENT);
            SetTextColor(dc, COLORREF(0xFFFFFF));
            // text taller than the bitmap starts above it, so the overflow is clipped off the top
            let mut r = RECT { left: 0, top: h - full, right: w, bottom: h };
            DrawTextW(dc, &mut wtext, &mut r, DT_WORDBREAK | DT_NOPREFIX);
            let _ = GdiFlush();
            let px = std::slice::from_raw_parts(bits as *const u32, (w * h) as usize);
            let cov = px.iter().map(|p| ((p >> 16) & 0xFF).max((p >> 8) & 0xFF).max(p & 0xFF) as u8).collect();
            layer = Some(TextLayer { w, h, cov });
            SelectObject(dc, old_bmp);
            let _ = DeleteObject(bmp.into());
        }
        SelectObject(dc, old_font);
        let _ = DeleteObject(font.into());
        let _ = DeleteDC(dc);
        layer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha(p: u32) -> u32 {
        p >> 24
    }

    #[test]
    fn body_is_opaque_inside_and_clear_at_corners() {
        let (w, _, px) = render(OverlayState::Listening(0.0), 0, None);
        assert_eq!(alpha(px[(4 * w + w / 2) as usize]), 0xE6);
        assert_eq!(px[0], 0);
        assert_eq!(px[(w - 1) as usize], 0);
    }

    #[test]
    fn rim_is_antialiased() {
        for state in [OverlayState::Idle, OverlayState::Listening(0.0)] {
            let (_, _, px) = render(state, 0, None);
            let (_, _, body) = state.geometry();
            assert!(px.iter().any(|&p| alpha(p) > 0 && alpha(p) < body), "{state:?} has no partial rim pixels");
        }
    }

    #[test]
    fn pixels_are_premultiplied() {
        for state in [OverlayState::Idle, OverlayState::Paused, OverlayState::Listening(0.7), OverlayState::Locked(0.4), OverlayState::Processing] {
            let (_, _, px) = render(state, 0, None);
            for p in px {
                let a = alpha(p);
                assert!((p >> 16) & 0xFF <= a && (p >> 8) & 0xFF <= a && p & 0xFF <= a, "{state:?}: {p:08X}");
            }
        }
    }

    #[test]
    fn long_text_keeps_the_newest_lines_in_a_panel_above_the_pill() {
        let short = layout_text("hello").expect("layout");
        let long = layout_text(&"word ".repeat(300)).expect("layout");
        assert!(long.h > short.h && long.h <= short.h * MAX_LINES, "{} vs {}", long.h, short.h);
        assert!(long.cov.iter().any(|&c| c > 200), "no text pixels");
        let (w, h, px) = render(OverlayState::Listening(0.0), 0, Some(&long));
        assert_eq!(w, PANEL_W);
        assert_eq!(h, long.h + 2 * PANEL_PAD + PANEL_GAP + H);
        // the pill keeps its size at the bottom centre, with clear space between it and the panel
        assert_eq!(alpha(px[((h - H / 2) * w + w / 2 - 50) as usize]), 0xE6);
        assert_eq!(alpha(px[((h - H - PANEL_GAP / 2) * w + w / 2) as usize]), 0);
        for p in px {
            let a = alpha(p);
            assert!((p >> 16) & 0xFF <= a && (p >> 8) & 0xFF <= a && p & 0xFF <= a, "{p:08X}");
        }
    }

    #[test]
    fn text_is_dropped_at_rest() {
        let t = layout_text("hello").expect("layout");
        assert_eq!(render(OverlayState::Idle, 0, Some(&t)).0, IDLE_W);
        assert!(layout_text("  ").is_none());
    }

    #[test]
    fn locked_dot_is_red_and_idle_dot_is_green() {
        let (w, h, px) = render(OverlayState::Locked(0.0), 0, None);
        let p = px[((h / 2) * w + W - 25) as usize];
        assert!((p >> 16) & 0xFF > p & 0xFF, "locked dot not red: {p:08X}");
        let (w, h, px) = render(OverlayState::Idle, 0, None);
        let p = px[((h / 2) * w + w / 2) as usize];
        assert!((p >> 8) & 0xFF > (p >> 16) & 0xFF, "idle dot not green: {p:08X}");
    }
}
