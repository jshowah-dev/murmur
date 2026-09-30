use anyhow::{anyhow, Result};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    COLORREF, ERROR_CLASS_ALREADY_EXISTS, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::Foundation::GetLastError;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetForegroundWindow, GetWindowRect, PeekMessageW, RegisterClassW,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage, UpdateLayeredWindow, GWL_EXSTYLE, HWND_TOPMOST, MSG,
    PM_REMOVE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_SHOWNOACTIVATE, ULW_ALPHA, WINDOW_EX_STYLE, WM_QUIT,
    WM_MOUSEMOVE, WM_RBUTTONUP, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use crate::editor_kit::{ease, reduced_motion};
use crate::motion;
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

/// The resting pill takes clicks so it can be right-clicked; the live pill is bigger and only
/// shows while you dictate, so it lets every click through to the app underneath.
fn ex_style(state: OverlayState) -> WINDOW_EX_STYLE {
    let base = WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW;
    if state.is_resting() { base } else { base | WS_EX_TRANSPARENT }
}

/// Set by the window procedure, taken by the main loop, which shows the menu.
static RIGHT_CLICKED: AtomicBool = AtomicBool::new(false);
/// winuser.h; the windows crate only exports it with the Win32_UI_Controls feature
const WM_MOUSELEAVE: u32 = 0x02A3;

/// Whether the cursor is over the pill.
static HOVERED: AtomicBool = AtomicBool::new(false);

/// Centres of the "more" dots that hint at the right-click menu, right of the status dot.
fn more_dots(w: f32, h: f32) -> [(f32, f32); 3] {
    [16.0, 12.0, 8.0].map(|from_right| (w - from_right, h / 2.0))
}

/// Linear hover progress after `dt`: toward 1 over the hover token while `on`, back toward 0
/// over the exit token otherwise, from wherever it is. Reduced motion jumps straight there.
fn step_hover(p: f32, on: bool, dt: Duration, reduced: bool) -> f32 {
    if reduced {
        return if on { 1.0 } else { 0.0 };
    }
    let d = motion::scaled(if on { motion::duration::HOVER } else { motion::duration::EXIT });
    let delta = dt.as_secs_f32() / d.as_secs_f32();
    if on { (p + delta).min(1.0) } else { (p - delta).max(0.0) }
}

pub struct Overlay {
    hwnd: HWND,
    state: OverlayState,
    tick: u32,
    /// linear 0..=1, eased when drawn
    hover: f32,
    stepped_at: Instant,
    reduced: bool,
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_MOUSEMOVE if !HOVERED.swap(true, Ordering::Relaxed) => {
            let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
            let _ = unsafe { TrackMouseEvent(&mut tme) };
        }
        WM_MOUSELEAVE => HOVERED.store(false, Ordering::Relaxed),
        WM_RBUTTONUP => {
            // the menu takes over from the hint; the menu's mouse capture also ends leave tracking
            HOVERED.store(false, Ordering::Relaxed);
            RIGHT_CLICKED.store(true, Ordering::Relaxed);
            return LRESULT(0);
        }
        _ => {}
    }
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
                ex_style(OverlayState::Idle),
                PCWSTR(class.as_ptr()),
                PCWSTR(wide("Murmur").as_ptr()),
                WS_POPUP,
                0, 0, W, H,
                None, None, Some(hinst.into()), None,
            )?;
            Ok(Overlay { hwnd, state: OverlayState::Idle, tick: 0, hover: 0.0, stepped_at: Instant::now(), reduced: reduced_motion() })
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
        let (w, h, pixels) = render(self.state, self.tick, self.hover);
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

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub fn set(&mut self, state: OverlayState) {
        let was_resting = self.state.is_resting();
        self.state = state;
        if !state.is_resting() {
            // the live pill is click-through, so no leave message will come
            HOVERED.store(false, Ordering::Relaxed);
            self.hover = 0.0;
        }
        self.tick = self.tick.wrapping_add(1);
        self.paint();
        unsafe {
            if was_resting != state.is_resting() {
                SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, ex_style(state).0 as isize);
            }
            // re-assert topmost on every state change so a fullscreen app cannot bury the pill
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOSIZE | SWP_NOMOVE | SWP_NOACTIVATE);
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
        }
    }

    /// Re-paint the resting pill so it follows the foreground window's monitor.
    pub fn refresh_resting(&mut self) {
        if self.state.is_resting() {
            self.paint();
        }
    }

    /// Moves the hover hint one frame toward the cursor's state; repaints only when it changed.
    pub fn animate(&mut self) {
        let now = Instant::now();
        let dt = now - self.stepped_at;
        self.stepped_at = now;
        let on = self.state.is_resting() && HOVERED.load(Ordering::Relaxed);
        let next = step_hover(self.hover, on, dt, self.reduced);
        if next != self.hover {
            self.hover = next;
            self.paint();
        }
    }

    /// True once per right-click on the pill.
    pub fn take_right_click(&self) -> bool {
        RIGHT_CLICKED.swap(false, Ordering::Relaxed)
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
        let r = cw.min(ch) / 2.0;
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

    /// Pack as premultiplied BGRA (0xAARRGGBB little-endian), what UpdateLayeredWindow expects.
    fn into_bgra(self) -> Vec<u32> {
        let q = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u32;
        self.px.into_iter().map(|[r, g, b, a]| (q(a) << 24) | (q(r) << 16) | (q(g) << 8) | q(b)).collect()
    }
}

/// Paint `state` into premultiplied BGRA pixels, row-major, `w * h` long.
/// `hover` (0..=1, linear) wakes the resting pill: full body opacity plus the "more" dots.
fn render(state: OverlayState, tick: u32, hover: f32) -> (i32, i32, Vec<u32>) {
    let (w, h, alpha) = state.geometry();
    let mut c = Canvas::new(w, h);
    let (wf, hf) = (w as f32, h as f32);
    let hover = if state.is_resting() { ease(motion::easing::STANDARD, hover.clamp(0.0, 1.0)) } else { 0.0 };
    let (_, _, live_alpha) = OverlayState::Processing.geometry();
    let alpha = alpha as f32 + (live_alpha as f32 - alpha as f32) * hover;
    // body: dim when resting, near-opaque when live or hovered
    c.capsule(0.0, 0.0, wf, hf, 0x202020, alpha / 255.0);
    match state {
        OverlayState::Idle | OverlayState::Paused => {
            let rgb = if state == OverlayState::Idle { 0x60D060 } else { 0x808080 };
            let d = hf - 8.0;
            c.capsule(wf / 2.0 - d / 2.0, 4.0, d, d, rgb, 1.0);
            if hover > 0.0 {
                let r = 1.2;
                for (x, y) in more_dots(wf, hf) {
                    c.capsule(x - r, y - r, 2.0 * r, 2.0 * r, 0xE0E0E0, hover);
                }
            }
        }
        OverlayState::Listening(level) | OverlayState::Locked(level) => {
            let locked = matches!(state, OverlayState::Locked(_));
            // the locked bar stops short of the dot at the right end
            let span = if locked { W - 58 } else { W - 40 } as f32;
            let bar_w = (span * level.clamp(0.0, 1.0)).max(6.0);
            c.capsule(20.0, hf / 2.0 - 3.0, bar_w, 6.0, 0x60D060, 1.0);
            if locked {
                let d = 10.0;
                c.capsule(wf - 20.0 - d, hf / 2.0 - d / 2.0, d, d, 0xE04040, 1.0);
            }
        }
        OverlayState::Processing => {
            let on = (tick / 4) % 3;
            for i in 0..3 {
                let rgb = if i == on { 0xFFFFFF } else { 0x707070 };
                c.capsule(wf / 2.0 - 18.0 + i as f32 * 14.0, hf / 2.0 - 3.0, 6.0, 6.0, rgb, 1.0);
            }
        }
    }
    (w, h, c.into_bgra())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn alpha(p: u32) -> u32 {
        p >> 24
    }

    #[test]
    fn body_is_opaque_inside_and_clear_at_corners() {
        let (w, _, px) = render(OverlayState::Listening(0.0), 0, 0.0);
        assert_eq!(alpha(px[(4 * w + w / 2) as usize]), 0xE6);
        assert_eq!(px[0], 0);
        assert_eq!(px[(w - 1) as usize], 0);
    }

    #[test]
    fn rim_is_antialiased() {
        for state in [OverlayState::Idle, OverlayState::Listening(0.0)] {
            let (_, _, px) = render(state, 0, 0.0);
            let (_, _, body) = state.geometry();
            assert!(px.iter().any(|&p| alpha(p) > 0 && alpha(p) < body), "{state:?} has no partial rim pixels");
        }
    }

    #[test]
    fn pixels_are_premultiplied() {
        for state in [OverlayState::Idle, OverlayState::Paused, OverlayState::Listening(0.7), OverlayState::Locked(0.4), OverlayState::Processing] {
            let (_, _, px) = render(state, 0, 0.0);
            for p in px {
                let a = alpha(p);
                assert!((p >> 16) & 0xFF <= a && (p >> 8) & 0xFF <= a && p & 0xFF <= a, "{state:?}: {p:08X}");
            }
        }
    }

    #[test]
    fn hovered_pill_brightens_and_shows_more_dots() {
        let (w, h, rest) = render(OverlayState::Idle, 0, 0.0);
        let (_, _, hover) = render(OverlayState::Idle, 0, 1.0);
        let at = |x: f32| ((h / 2) * w + x as i32) as usize;
        // body between the green dot and the "more" dots
        assert_eq!(alpha(rest[at(w as f32 / 2.0 - 8.0)]), 0x80);
        assert_eq!(alpha(hover[at(w as f32 / 2.0 - 8.0)]), 0xE6);
        for (x, _) in more_dots(w as f32, h as f32) {
            assert!((rest[at(x)] >> 16) & 0xFF < 0x40, "no dots at rest: {:08X}", rest[at(x)]);
            assert!((hover[at(x)] >> 16) & 0xFF > 0xA0, "dot missing on hover: {:08X}", hover[at(x)]);
        }
    }

    #[test]
    fn hover_eases_in_and_out_and_can_turn_back() {
        let ms = |n: u64| Duration::from_millis(n);
        assert_eq!(step_hover(0.0, true, motion::duration::HOVER, false), 1.0);
        assert_eq!(step_hover(1.0, false, motion::duration::EXIT, false), 0.0);
        let half = step_hover(0.0, true, motion::duration::HOVER / 2, false);
        assert!((half - 0.5).abs() < 1e-3, "{half}");
        // leaving midway heads back from where it is, at the exit pace
        let back = step_hover(half, false, ms(50), false);
        assert!((back - 0.0).abs() < 1e-3, "{back}");
        assert_eq!(step_hover(0.0, true, ms(1), true), 1.0, "reduced motion jumps");
        assert_eq!(step_hover(1.0, false, ms(1), true), 0.0);
    }

    #[test]
    fn only_the_resting_pill_takes_clicks() {
        for state in [OverlayState::Idle, OverlayState::Paused] {
            assert!(!ex_style(state).contains(WS_EX_TRANSPARENT), "{state:?} should take the right-click");
        }
        for state in [OverlayState::Listening(0.5), OverlayState::Locked(0.5), OverlayState::Processing] {
            assert!(ex_style(state).contains(WS_EX_TRANSPARENT), "{state:?} should be click-through");
        }
        for state in [OverlayState::Idle, OverlayState::Processing] {
            assert!(ex_style(state).contains(WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW));
        }
    }

    #[test]
    fn locked_dot_is_red_and_idle_dot_is_green() {
        let (w, h, px) = render(OverlayState::Locked(0.0), 0, 0.0);
        let p = px[((h / 2) * w + W - 25) as usize];
        assert!((p >> 16) & 0xFF > p & 0xFF, "locked dot not red: {p:08X}");
        let (w, h, px) = render(OverlayState::Idle, 0, 0.0);
        let p = px[((h / 2) * w + w / 2) as usize];
        assert!((p >> 8) & 0xFF > (p >> 16) & 0xFF, "idle dot not green: {p:08X}");
    }
}
