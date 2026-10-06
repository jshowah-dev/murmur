// The Mac pill lands in phase 3; until then its drawing is only used on Windows.
#![cfg_attr(target_os = "macos", allow(dead_code))]

#[cfg(windows)]
use anyhow::{anyhow, Result};
#[cfg(windows)]
use windows::core::PCWSTR;
#[cfg(windows)]
use windows::Win32::Foundation::{ERROR_CLASS_ALREADY_EXISTS, HWND, LPARAM, LRESULT, POINT, WPARAM};
use crate::platform::Rect as RECT;
#[cfg(windows)]
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
#[cfg(windows)]
use windows::Win32::Foundation::GetLastError;
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetCursorPos, GetForegroundWindow, GetWindowRect, LoadCursorW, PeekMessageW,
    RegisterClassW, SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage, GWL_EXSTYLE,
    HWND_TOPMOST, IDC_ARROW, MSG,
    PM_REMOVE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_SHOWNOACTIVATE, WINDOW_EX_STYLE, WM_QUIT,
    WM_MOUSEMOVE, WM_RBUTTONUP, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
#[cfg(windows)]
use std::time::Instant;
use crate::editor_kit::ease;
#[cfg(windows)]
use crate::editor_kit::reduced_motion;
use crate::motion;
use crate::canvas::Canvas;
#[cfg(windows)]
use crate::canvas;
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{MonitorFromWindow, GetMonitorInfoW, MONITORINFO, MONITOR_DEFAULTTONEAREST};

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
#[cfg(windows)]
fn ex_style(state: OverlayState) -> WINDOW_EX_STYLE {
    let base = WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW;
    if state.is_resting() { base } else { base | WS_EX_TRANSPARENT }
}

/// Set by the window procedure, taken by the main loop, which shows the menu.
#[cfg(windows)]
static RIGHT_CLICKED: AtomicBool = AtomicBool::new(false);
/// winuser.h; the windows crate only exports it with the Win32_UI_Controls feature
#[cfg(windows)]
const WM_MOUSELEAVE: u32 = 0x02A3;

/// Whether the cursor is over the pill.
#[cfg(windows)]
static HOVERED: AtomicBool = AtomicBool::new(false);

/// Centres of the "more" dots that hint at the right-click menu, right of the status dot.
fn more_dots(w: f32, h: f32) -> [(f32, f32); 3] {
    [16.0, 12.0, 8.0].map(|from_right| (w - from_right, h / 2.0))
}

/// Everything besides the state that shapes a frame. Progress values are linear 0..=1 and
/// eased when drawn.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Look {
    hover: f32,
    /// 0 = resting size, 1 = live size
    grow: f32,
    /// the voice, smoothed: 0..=1
    level: f32,
    /// seconds of ribbon drift; only advances while there's voice
    phase: f32,
    /// cursor proximity, 0..=1, and where along the pill it is
    near: f32,
    near_x: f32,
    /// landing pulse progress, 0 = none
    pulse: f32,
    /// a mote carries the "working" signal, so the processing dots step back
    quiet: bool,
    /// which processing dot is lit, 0..3
    dot: u32,
}

/// How far out the resting pill notices the cursor.
const NEAR_PX: f32 = 60.0;

/// (strength 0..=1, where along the pill 0..=1) for a cursor at `cursor` near `pill`: full
/// strength on the pill, nothing from NEAR_PX away.
fn nearness(cursor: (i32, i32), pill: RECT) -> (f32, f32) {
    let (x, y) = cursor;
    let dx = (pill.left - x).max(x - pill.right).max(0) as f32;
    let dy = (pill.top - y).max(y - pill.bottom).max(0) as f32;
    let strength = (1.0 - dx.hypot(dy) / NEAR_PX).max(0.0);
    let along = ((x - pill.left) as f32 / (pill.right - pill.left).max(1) as f32).clamp(0.0, 1.0);
    (strength, along)
}

/// The processing dot lit `elapsed` into processing: one step per emphasis beat, so the dots
/// keep moving however long the words take, with nothing else changing.
fn processing_dot(elapsed: Duration) -> u32 {
    (elapsed.as_millis() / motion::scaled(motion::duration::EMPHASIS).as_millis().max(1)) as u32 % 3
}

/// Linear progress after `dt`: toward 1 over `up` while `on`, back toward 0 over `down`
/// otherwise, from wherever it is. Reduced motion jumps straight there.
fn step(p: f32, on: bool, dt: Duration, up: Duration, down: Duration, reduced: bool) -> f32 {
    if reduced {
        return if on { 1.0 } else { 0.0 };
    }
    let d = motion::scaled(if on { up } else { down });
    let delta = dt.as_secs_f32() / d.as_secs_f32();
    if on { (p + delta).min(1.0) } else { (p - delta).max(0.0) }
}

/// How fast the ribbon answers the voice. Deliberately quicker than any kit token: the
/// material is breath, and a meter that lags your voice feels deaf. Logged in the kit ledger.
const VOICE_RISE: Duration = Duration::from_millis(40);

/// The smoothed level one frame on: rises toward `target` over VOICE_RISE, falls at a steady
/// pace over the fill token. Reduced motion follows the voice exactly.
fn follow(level: f32, target: f32, dt: Duration, reduced: bool) -> f32 {
    if reduced {
        return target;
    }
    if target > level {
        level + (target - level) * (dt.as_secs_f32() / motion::scaled(VOICE_RISE).as_secs_f32()).min(1.0)
    } else {
        (level - dt.as_secs_f32() / motion::scaled(motion::duration::FILL).as_secs_f32()).max(target)
    }
}

/// Seven soft blobs across `span` that swell with the voice, each slightly out of step with
/// its neighbours, so it reads as a voice rather than a meter.
fn ribbon(c: &mut Canvas, x0: f32, span: f32, hf: f32, level: f32, phase: f32, alpha: f32) {
    const N: usize = 7;
    let bw = 8.0;
    let gap = (span - bw).max(0.0) / (N - 1) as f32;
    for i in 0..N {
        let wobble = 0.6 + 0.4 * (phase * 7.0 + i as f32 * 0.9).sin();
        let bh = (4.0 + (hf - 14.0) * level.clamp(0.0, 1.0) * wobble).clamp(4.0, hf - 8.0);
        c.capsule(x0 + i as f32 * gap, hf / 2.0 - bh / 2.0, bw, bh, 0x60D060, alpha);
    }
}

/// Pill size for eased growth `g`: the resting size at 0, the live size at 1.
fn size(g: f32) -> (i32, i32) {
    let lerp = |a: i32, b: i32| (a as f32 + (b - a) as f32 * g).round() as i32;
    (lerp(IDLE_W, W), lerp(IDLE_H, H))
}

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
pub use mac::{live, Overlay};

#[cfg(windows)]
pub struct Overlay {
    hwnd: HWND,
    state: OverlayState,
    look: Look,
    processing_at: Option<Instant>,
    pulse_at: Option<Instant>,
    stepped_at: Instant,
    reduced: bool,
}

#[cfg(windows)]
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

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
impl Overlay {
    pub fn create() -> Result<Overlay> {
        unsafe {
            let hinst = GetModuleHandleW(None)?;
            let class = wide("MurmurOverlay");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                // without a class cursor the pill shows the thread's initial wait cursor
                hCursor: LoadCursorW(None, IDC_ARROW)?,
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
            Ok(Overlay { hwnd, state: OverlayState::Idle, look: Look::default(), processing_at: None, pulse_at: None, stepped_at: Instant::now(), reduced: reduced_motion() })
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

    fn paint(&mut self) {
        let (w, h, pixels) = render(self.state, self.look);
        let (x, y) = self.target_position(w, h);
        canvas::push(self.hwnd, x, y, w, h, &pixels);
    }

    /// The pill's centre in physical pixels, where the mote takes off.
    pub fn centre_physical(&self) -> (f32, f32) {
        crate::caret::physical(|| unsafe {
            let mut r = RECT::default();
            let _ = GetWindowRect(self.hwnd, &mut r);
            ((r.left + r.right) as f32 / 2.0, (r.top + r.bottom) as f32 / 2.0)
        })
    }

    /// The pill's top centre in physical pixels: where a message goes when there's no caret.
    pub fn above_physical(&self) -> (f32, f32) {
        crate::caret::physical(|| unsafe {
            let mut r = RECT::default();
            let _ = GetWindowRect(self.hwnd, &mut r);
            ((r.left + r.right) as f32 / 2.0, r.top as f32)
        })
    }

    pub fn hwnd(&self) -> crate::platform::Window {
        crate::platform::Window(self.hwnd.0 as isize)
    }

    pub fn set(&mut self, state: OverlayState) {
        let was_resting = self.state.is_resting();
        if state != OverlayState::Processing {
            self.processing_at = None;
        } else if self.state != OverlayState::Processing {
            self.processing_at = Some(Instant::now());
        }
        self.state = state;
        if !state.is_resting() {
            // the live pill is click-through, so no leave message will come
            HOVERED.store(false, Ordering::Relaxed);
            self.look.hover = 0.0;
        }
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

    /// Steps every per-frame value one frame toward where the state and cursor say it should be;
    /// repaints only when the look changed.
    pub fn animate(&mut self) {
        let now = Instant::now();
        let dt = now - self.stepped_at;
        self.stepped_at = now;
        let resting = self.state.is_resting();
        let mut look = self.look;
        look.hover = step(look.hover, resting && HOVERED.load(Ordering::Relaxed), dt, motion::duration::HOVER, motion::duration::EXIT, self.reduced);
        look.grow = step(look.grow, !resting, dt, motion::duration::ENTER, motion::duration::EXIT, self.reduced);
        let target = match self.state {
            OverlayState::Listening(l) | OverlayState::Locked(l) => l.clamp(0.0, 1.0),
            _ => 0.0,
        };
        look.level = follow(look.level, target, dt, self.reduced);
        // still at silence, so a quiet pill doesn't repaint every frame
        if look.level > 0.01 && !self.reduced {
            look.phase = (look.phase + dt.as_secs_f32()) % 1000.0;
        }
        let (near, near_x) = if resting && !self.reduced { self.cursor_nearness() } else { (0.0, look.near_x) };
        // quantized so a still cursor or a far one causes no repaints
        look.near = (near * 64.0).round() / 64.0;
        look.near_x = (near_x * 64.0).round() / 64.0;
        look.pulse = match self.pulse_at {
            Some(t0) => {
                let p = (now - t0).as_secs_f32() / motion::scaled(motion::duration::EMPHASIS).as_secs_f32();
                if p >= 1.0 {
                    self.pulse_at = None;
                    0.0
                } else if self.reduced {
                    // no bloom, just a brighter dot for the same moment
                    0.5
                } else {
                    p.max(0.001)
                }
            }
            None => 0.0,
        };
        look.dot = self.processing_at.map_or(0, |t0| processing_dot(now - t0));
        if look != self.look {
            self.look = look;
            self.paint();
        }
    }

    pub fn set_quiet(&mut self, quiet: bool) {
        if self.look.quiet != quiet {
            self.look.quiet = quiet;
            self.paint();
        }
    }

    /// One soft pulse of the resting dot: your words landed.
    pub fn pulse(&mut self) {
        self.pulse_at = Some(Instant::now());
    }

    fn cursor_nearness(&self) -> (f32, f32) {
        unsafe {
            let mut pt = POINT::default();
            let mut r = RECT::default();
            if GetCursorPos(&mut pt).is_err() || GetWindowRect(self.hwnd, &mut r).is_err() {
                return (0.0, 0.5);
            }
            nearness((pt.x, pt.y), r)
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

/// Paint `state` into premultiplied BGRA pixels, row-major, `w * h` long. The size follows
/// `look.grow`; `look.hover` wakes the resting pill (full body opacity plus the "more" dots).
fn render(state: OverlayState, look: Look) -> (i32, i32, Vec<u32>) {
    let grow = ease(motion::easing::STANDARD, look.grow.clamp(0.0, 1.0));
    let hover = if state.is_resting() { ease(motion::easing::STANDARD, look.hover.clamp(0.0, 1.0)) } else { 0.0 };
    let (w, h) = size(grow);
    let mut c = Canvas::new(w, h);
    let (wf, hf) = (w as f32, h as f32);
    let (_, _, rest_alpha) = OverlayState::Idle.geometry();
    let (_, _, live_alpha) = OverlayState::Processing.geometry();
    let alpha = rest_alpha as f32 + (live_alpha as f32 - rest_alpha as f32) * grow.max(hover);
    // body: dim when resting, near-opaque when live or hovered
    c.capsule(0.0, 0.0, wf, hf, 0x202020, alpha / 255.0);
    match state {
        OverlayState::Idle | OverlayState::Paused => {
            // shrinking back from live: the dot fades in as the pill settles
            let fade = 1.0 - grow;
            let rgb = if state == OverlayState::Idle { 0x60D060 } else { 0x808080 };
            if look.near > 0.0 {
                // gathers toward an approaching cursor; the hover dots take over on arrival
                c.glow(look.near_x * wf, hf / 2.0, hf * 2.5, 0x60D060, 0.35 * look.near * (1.0 - hover) * fade);
            }
            let d = (IDLE_H - 8) as f32;
            if look.pulse > 0.0 {
                let s = (std::f32::consts::PI * look.pulse).sin();
                let ring = d + 6.0 * s;
                c.capsule(wf / 2.0 - ring / 2.0, hf / 2.0 - ring / 2.0, ring, ring, rgb, 0.35 * s * fade);
            }
            c.capsule(wf / 2.0 - d / 2.0, hf / 2.0 - d / 2.0, d, d, rgb, fade);
            if hover > 0.0 {
                let r = 1.2;
                for (x, y) in more_dots(wf, hf) {
                    c.capsule(x - r, y - r, 2.0 * r, 2.0 * r, 0xE0E0E0, hover * fade);
                }
            }
        }
        OverlayState::Listening(_) | OverlayState::Locked(_) => {
            // live content arrives late in the growth, once there's room for it
            let show = grow * grow;
            let locked = matches!(state, OverlayState::Locked(_));
            // the locked bar stops short of the dot at the right end
            let span = (if locked { wf - 58.0 } else { wf - 40.0 }).max(0.0);
            ribbon(&mut c, 20.0, span, hf, look.level, look.phase, show);
            if locked {
                let d = 10.0;
                c.capsule(wf - 20.0 - d, hf / 2.0 - d / 2.0, d, d, 0xE04040, show);
            }
        }
        OverlayState::Processing => {
            let show = grow * grow;
            for i in 0..3 {
                let rgb = if i == look.dot { 0xFFFFFF } else { 0x707070 };
                c.capsule(wf / 2.0 - 18.0 + i as f32 * 14.0, hf / 2.0 - 3.0, 6.0, 6.0, rgb, show * if look.quiet { 0.3 } else { 1.0 });
            }
        }
    }
    (w, h, c.into_bgra())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Settled look for a state: live states fully grown, resting ones at rest.
    fn look_for(state: OverlayState) -> Look {
        Look { grow: if state.is_resting() { 0.0 } else { 1.0 }, ..Default::default() }
    }

    #[test]
    fn pill_size_follows_growth() {
        assert_eq!(size(0.0), (IDLE_W, IDLE_H));
        assert_eq!(size(1.0), (W, H));
        let (w, h) = size(0.5);
        assert!(w > IDLE_W && w < W && h > IDLE_H && h < H, "{w}x{h}");
        let (w, h, _) = render(OverlayState::Listening(0.0), Look { grow: 0.0, ..Default::default() });
        assert_eq!((w, h), (IDLE_W, IDLE_H), "a live state starts from the resting size");
    }

    #[test]
    fn growth_turns_back_from_where_it_is() {
        let e = motion::duration::ENTER;
        let x = motion::duration::EXIT;
        assert_eq!(step(0.0, true, e, e, x, false), 1.0);
        let mid = step(0.0, true, e / 2, e, x, false);
        assert!((mid - 0.5).abs() < 1e-3, "{mid}");
        // released midway: shrinks from 0.5 at the exit pace, no jump to either end
        let back = step(mid, false, x / 4, e, x, false);
        assert!((back - 0.25).abs() < 1e-3, "{back}");
        assert_eq!(step(0.2, true, Duration::from_millis(1), e, x, true), 1.0, "reduced motion snaps");
    }

    #[test]
    fn voice_level_rises_fast_and_falls_slowly() {
        let ms = |n: u64| Duration::from_millis(n);
        assert_eq!(follow(0.0, 1.0, VOICE_RISE, false), 1.0);
        let half = follow(0.0, 1.0, VOICE_RISE / 2, false);
        assert!((half - 0.5).abs() < 1e-3, "{half}");
        let fall = follow(1.0, 0.0, motion::duration::FILL / 2, false);
        assert!((fall - 0.5).abs() < 1e-3, "{fall}");
        assert_eq!(follow(0.6, 0.2, ms(1), true), 0.2, "reduced motion follows the voice exactly");
        assert_eq!(follow(0.3, 0.3, ms(15), false), 0.3);
    }

    #[test]
    fn ribbon_swells_with_the_voice_and_stays_inside() {
        let quiet = Look { grow: 1.0, level: 0.0, ..Default::default() };
        let loud = Look { grow: 1.0, level: 1.0, ..Default::default() };
        let (w, h, px_q) = render(OverlayState::Listening(0.0), quiet);
        let (_, _, px_l) = render(OverlayState::Listening(1.0), loud);
        let green = |p: u32| (p >> 8) & 0xFF;
        // first blob's column, 7 px above the middle: body when quiet, ribbon when loud
        let at = |y: i32| (y * w + 24) as usize;
        assert!(green(px_l[at(h / 2 - 7)]) > green(px_q[at(h / 2 - 7)]) + 0x40);
        // never touches the top or bottom 3 rows
        for x in 0..w {
            for y in [0, 1, 2, h - 3, h - 2, h - 1] {
                assert!(green(px_l[(y * w + x) as usize]) < 0x40, "ribbon at ({x},{y})");
            }
        }
    }

    #[test]
    fn locked_ribbon_leaves_the_red_dot_alone() {
        let (w, h, px) = render(OverlayState::Locked(1.0), Look { grow: 1.0, level: 1.0, ..Default::default() });
        let p = px[((h / 2) * w + W - 25) as usize];
        assert!((p >> 16) & 0xFF > (p >> 8) & 0xFF, "red dot overdrawn: {p:08X}");
    }

    #[test]
    fn nearness_rises_as_the_cursor_approaches() {
        let pill = RECT { left: 100, top: 100, right: 156, bottom: 114 };
        assert_eq!(nearness((0, 0), pill).0, 0.0);
        assert_eq!(nearness((128, 107), pill), (1.0, 0.5), "over the pill");
        let (n, x) = nearness((70, 107), pill);
        assert!((n - 0.5).abs() < 1e-3 && x == 0.0, "30 px to the left: {n} {x}");
        assert_eq!(nearness((200, 60), pill).1, 1.0, "right of the pill");
        assert_eq!(nearness((128, 30), pill).0, 0.0, "70 px above: out of reach");
    }

    #[test]
    fn glow_gathers_on_the_cursor_side() {
        let near = Look { near: 1.0, near_x: 0.0, ..Default::default() };
        let (w, h, px) = render(OverlayState::Idle, near);
        let green = |p: u32| (p >> 8) & 0xFF;
        let row = (h / 2) * w;
        assert!(green(px[(row + 5) as usize]) > green(px[(row + w - 6) as usize]) + 8);
        let (_, _, plain) = render(OverlayState::Idle, Look::default());
        assert_ne!(px, plain);
    }

    #[test]
    fn live_pill_ignores_the_cursor() {
        let live = Look { grow: 1.0, ..Default::default() };
        let (_, _, a) = render(OverlayState::Listening(0.0), live);
        let (_, _, b) = render(OverlayState::Listening(0.0), Look { near: 1.0, near_x: 0.0, hover: 1.0, ..live });
        assert_eq!(a, b);
    }

    #[test]
    fn landing_pulse_blooms_around_the_dot() {
        let (w, h, calm) = render(OverlayState::Idle, Look::default());
        let (_, _, pulse) = render(OverlayState::Idle, Look { pulse: 0.5, ..Default::default() });
        let green = |p: u32| (p >> 8) & 0xFF;
        let beside = ((h / 2) * w + w / 2 + 4) as usize;
        assert!(green(pulse[beside]) > green(calm[beside]) + 0x10, "{:08X} vs {:08X}", pulse[beside], calm[beside]);
        let (_, _, over) = render(OverlayState::Idle, Look { pulse: 0.0, ..Default::default() });
        assert_eq!(over, calm);
    }

    #[test]
    fn processing_dots_step_with_time_alone() {
        let d = motion::scaled(motion::duration::EMPHASIS);
        assert_eq!(processing_dot(Duration::ZERO), 0);
        assert_eq!(processing_dot(d), 1);
        assert_eq!(processing_dot(d * 2), 2);
        assert_eq!(processing_dot(d * 3), 0);
        let live = Look { grow: 1.0, ..Default::default() };
        let (_, _, first) = render(OverlayState::Processing, live);
        let (_, _, second) = render(OverlayState::Processing, Look { dot: 1, ..live });
        assert_ne!(first, second, "the lit dot moved");
    }

    #[test]
    fn quiet_dims_the_processing_dots() {
        let live = Look { grow: 1.0, ..Default::default() };
        let (w, h, loud) = render(OverlayState::Processing, live);
        let (_, _, quiet) = render(OverlayState::Processing, Look { quiet: true, ..live });
        let lit = ((h / 2) * w + w / 2 - 15) as usize;
        assert!(((quiet[lit] >> 16) & 0xFF) + 0x40 < ((loud[lit] >> 16) & 0xFF));
    }

    fn alpha(p: u32) -> u32 {
        p >> 24
    }

    #[test]
    fn body_is_opaque_inside_and_clear_at_corners() {
        let (w, _, px) = render(OverlayState::Listening(0.0), look_for(OverlayState::Listening(0.0)));
        assert_eq!(alpha(px[(4 * w + w / 2) as usize]), 0xE6);
        assert_eq!(px[0], 0);
        assert_eq!(px[(w - 1) as usize], 0);
    }

    #[test]
    fn rim_is_antialiased() {
        for state in [OverlayState::Idle, OverlayState::Listening(0.0)] {
            let (_, _, px) = render(state, look_for(state));
            let (_, _, body) = state.geometry();
            assert!(px.iter().any(|&p| alpha(p) > 0 && alpha(p) < body), "{state:?} has no partial rim pixels");
        }
    }

    #[test]
    fn pixels_are_premultiplied() {
        for state in [OverlayState::Idle, OverlayState::Paused, OverlayState::Listening(0.7), OverlayState::Locked(0.4), OverlayState::Processing] {
            let (_, _, px) = render(state, look_for(state));
            for p in px {
                let a = alpha(p);
                assert!((p >> 16) & 0xFF <= a && (p >> 8) & 0xFF <= a && p & 0xFF <= a, "{state:?}: {p:08X}");
            }
        }
    }

    #[test]
    fn hovered_pill_brightens_and_shows_more_dots() {
        let (w, h, rest) = render(OverlayState::Idle, look_for(OverlayState::Idle));
        let (_, _, hover) = render(OverlayState::Idle, Look { hover: 1.0, ..Default::default() });
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
        assert_eq!(step(0.0, true, motion::duration::HOVER, motion::duration::HOVER, motion::duration::EXIT, false), 1.0);
        assert_eq!(step(1.0, false, motion::duration::EXIT, motion::duration::HOVER, motion::duration::EXIT, false), 0.0);
        let half = step(0.0, true, motion::duration::HOVER / 2, motion::duration::HOVER, motion::duration::EXIT, false);
        assert!((half - 0.5).abs() < 1e-3, "{half}");
        // leaving midway heads back from where it is, at the exit pace
        let back = step(half, false, ms(50), motion::duration::HOVER, motion::duration::EXIT, false);
        assert!((back - 0.0).abs() < 1e-3, "{back}");
        assert_eq!(step(0.0, true, ms(1), motion::duration::HOVER, motion::duration::EXIT, true), 1.0, "reduced motion jumps");
        assert_eq!(step(1.0, false, ms(1), motion::duration::HOVER, motion::duration::EXIT, true), 0.0);
    }

    #[cfg(windows)] // window styles
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
        let (w, h, px) = render(OverlayState::Locked(0.0), look_for(OverlayState::Locked(0.0)));
        let p = px[((h / 2) * w + W - 25) as usize];
        assert!((p >> 16) & 0xFF > p & 0xFF, "locked dot not red: {p:08X}");
        let (w, h, px) = render(OverlayState::Idle, look_for(OverlayState::Idle));
        let p = px[((h / 2) * w + w / 2) as usize];
        assert!((p >> 8) & 0xFF > (p >> 16) & 0xFF, "idle dot not green: {p:08X}");
    }
}
