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
    PostQuitMessage, RegisterClassW, SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage, GWL_EXSTYLE,
    HWND_TOPMOST, IDC_ARROW, MSG,
    PM_REMOVE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_SHOWNOACTIVATE, WINDOW_EX_STYLE, WM_QUIT,
    WM_ENDSESSION, WM_MOUSEMOVE, WM_RBUTTONUP, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
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
    fn is_voice(self) -> bool {
        matches!(self, OverlayState::Listening(_) | OverlayState::Locked(_))
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
    /// the voice over the last moments, newest first: the scope's trace
    history: [f32; HIST],
    /// how far, in columns 0..1, the trace has moved out since `history` last shifted
    scroll: f32,
    /// cursor proximity, 0..=1, and where along the pill it is
    near: f32,
    near_x: f32,
    /// landing pulse progress, 0 = none
    pulse: f32,
    /// a mote carries the "working" signal, so the processing braid steps back
    quiet: bool,
    /// seconds into processing: turns the braid
    braid: f32,
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

/// The beat `elapsed` into processing, 0..3: one step per emphasis beat, so with reduced motion
/// the braid keeps moving however long the words take, with nothing else changing.
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

/// How fast the scope answers the voice. Deliberately quicker than any kit token: the
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

// The live pill is a dot scope, after Hex's HUD (MIT, Christopher Langton): a plate of small
// dots lit by the voice. The newest moment sits in the middle and older ones drift out to both
// ends, dimming as they go, like phosphor.

/// Points between dot centres.
const PITCH: f32 = 4.5;
/// Samples of the voice the trace keeps: enough for half the widest field, plus the middle.
const HIST: usize = 32;
/// How fast the trace drifts out from the middle, points a second.
const DRIFT: f32 = 36.0;
/// Columns the phosphor tail takes to fade to about a third.
const TAIL: f32 = 7.0;
/// The dots' colour, and the near-white their brightest cores warm to.
const GREEN: u32 = 0x60D060;
const CORE: u32 = 0xE4FFE4;
/// Seconds per turn of the processing braid, and how many twists span the field.
const BRAID_TURN: f32 = 1.2;
const BRAID_TWISTS: f32 = 1.5;

fn sstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn mix_rgb(a: u32, b: u32, t: f32) -> u32 {
    let ch = |s: u32| lerp(((a >> s) & 0xFF) as f32, ((b >> s) & 0xFF) as f32, t).round() as u32;
    (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

/// Signed distance from (x, y) to a `hw`×`hh` half-size box with corner radius `r`, centred at 0.
fn rounded_box(x: f32, y: f32, hw: f32, hh: f32, r: f32) -> f32 {
    let (qx, qy) = (x.abs() - hw + r, y.abs() - hh + r);
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r
}

/// Signed distance to a box hanging from y = 0, `hw` from the middle each way and `height` tall,
/// rounded by `r` below, with concave fillets where it meets the top edge: the notch grown down.
fn island_distance(x: f32, y: f32, hw: f32, height: f32, r: f32) -> f32 {
    let fillet = (height * 0.25).clamp(0.0, 6.0);
    // reaches `r` above the top so only the lower corners round
    let body = rounded_box(x, y - (height - r) * 0.5, hw, (height + r) * 0.5, r);
    let qx = x.abs();
    let square = rounded_box(qx - (hw + fillet * 0.5), y - fillet * 0.5, fillet * 0.5 + 0.5, fillet * 0.5, 0.0);
    let hollow = fillet - (qx - (hw + fillet)).hypot(y - fillet);
    body.min(square.max(hollow))
}

/// Where the dot field sits: centre and size, units.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Field {
    cx: f32,
    cy: f32,
    w: f32,
    h: f32,
}

/// The field's dots, each lit by `light(dx, dy)` = (power, freshness), both 0..=1, for a dot
/// `dx`, `dy` units from the field's middle. Unlit dots still show faintly; dots fade out near
/// the field's rounded edge.
fn dots(c: &mut Canvas, f: Field, alpha: f32, light: impl Fn(f32, f32) -> (f32, f32)) {
    let cols = (f.w / PITCH).floor() as i32;
    let rows = (f.h / PITCH).floor() as i32;
    if cols <= 0 || rows <= 0 || alpha <= 0.0 {
        return;
    }
    let (ox, oy) = ((cols - 1) as f32 / 2.0, (rows - 1) as f32 / 2.0);
    let (hw, hh) = (f.w / 2.0, f.h / 2.0);
    for j in 0..rows {
        for i in 0..cols {
            let (dx, dy) = ((i as f32 - ox) * PITCH, (j as f32 - oy) * PITCH);
            let mask = sstep(1.4, 4.6, -rounded_box(dx, dy, hw, hh, hh));
            if mask <= 0.0 {
                continue;
            }
            let (power, fresh) = light(dx, dy);
            let power = power.clamp(1e-5, 1.0);
            let row = dy.abs() / hh;
            let unlit = 0.07 * (1.0 - 0.7 * sstep(0.35, 0.95, row));
            let a = unlit + (0.96 - unlit) * power.powf(0.9 * lerp(1.4, 1.0, fresh));
            let r = (0.25 + 0.17 * power.powf(0.6)) * lerp(1.0, lerp(0.82, 1.0, fresh), sstep(0.03, 0.2, power)) * PITCH;
            let rgb = mix_rgb(GREEN, CORE, (power * power * 1.1).min(0.9) * lerp(0.3, 1.0, fresh));
            let (x, y) = (f.cx + dx, f.cy + dy);
            if power > 0.05 {
                c.capsule(x - 2.0 * r, y - 2.0 * r, 4.0 * r, 4.0 * r, rgb, 0.132 * power * power * mask * alpha);
            }
            c.capsule(x - r, y - r, 2.0 * r, 2.0 * r, rgb, a * mask * alpha);
        }
    }
}

/// The voice `k` columns out from the middle: the trace has drifted `scroll` columns since the
/// newest sample, `history[0]`, was taken.
fn trace_at(history: &[f32; HIST], scroll: f32, k: f32) -> f32 {
    let s = (k - scroll).max(0.0);
    let i = s.floor() as usize;
    if i + 1 >= HIST {
        return 0.0;
    }
    lerp(history[i], history[i + 1], s.fract())
}

/// Lighting for the listening scope: each column lit from the middle row out as far as its
/// moment of voice reached, a faint flat line at silence, older columns dimmer.
fn scope_light(look: &Look, dx: f32, dy: f32, hh: f32) -> (f32, f32) {
    let k = dx.abs() / PITCH;
    let v = trace_at(&look.history, look.scroll, k).clamp(0.0, 1.0);
    let fresh = (-k / TAIL).exp();
    let r = dy.abs() / hh;
    let power = sstep(v + 0.15, v - 0.15, r) * (0.4 + 0.6 * v.sqrt()) * lerp(0.35, 1.0, fresh);
    (power, fresh)
}

/// Lighting for processing: two strands twisting round each other, the near one brighter.
fn braid_light(look: &Look, f: Field, dx: f32, dy: f32) -> (f32, f32) {
    let x = dx / (f.w / 2.0);
    let amp = (f.h / 2.0 - PITCH) * (1.0 - x * x).max(0.0).sqrt();
    let th = x * BRAID_TWISTS * std::f32::consts::TAU - look.braid / BRAID_TURN * std::f32::consts::TAU;
    let mut best = (0.0f32, 0.0f32);
    for sign in [1.0, -1.0] {
        let near = (sign * th.cos() + 1.0) / 2.0;
        let d = (dy - sign * amp * th.sin()) / (PITCH * 0.75);
        let p = lerp(0.3, 1.0, near) * (-d * d).exp();
        if p > best.0 {
            best = (p, near);
        }
    }
    if look.quiet { (best.0 * 0.3, best.1) } else { best }
}

/// Pill size for eased growth `g`: the resting size at 0, the live size at 1.
fn size(g: f32) -> (i32, i32) {
    let lerp = |a: i32, b: i32| (a as f32 + (b - a) as f32 * g).round() as i32;
    (lerp(IDLE_W, W), lerp(IDLE_H, H))
}

/// What the pill shows and how far each of its motions has run, apart from the window it's drawn
/// in: the same on both systems.
struct Pill {
    state: OverlayState,
    look: Look,
    processing_at: Option<Instant>,
    pulse_at: Option<Instant>,
    stepped_at: Instant,
    reduced: bool,
}

impl Pill {
    fn new(reduced: bool) -> Pill {
        Pill { state: OverlayState::Idle, look: Look::default(), processing_at: None, pulse_at: None, stepped_at: Instant::now(), reduced }
    }

    fn set(&mut self, state: OverlayState) {
        if state != OverlayState::Processing {
            self.processing_at = None;
        } else if self.state != OverlayState::Processing {
            self.processing_at = Some(Instant::now());
        }
        if state.is_voice() && !self.state.is_voice() {
            // a fresh dictation starts from a flat line
            self.look.history = [0.0; HIST];
            self.look.scroll = 0.0;
        }
        self.state = state;
        if !state.is_resting() {
            self.look.hover = 0.0;
        }
    }

    /// Steps every per-frame value one frame toward where the state and cursor say it should be.
    /// `hovered` is the cursor being on the pill; `near` is asked only when the resting pill
    /// shows it. True when the look changed and needs painting.
    fn step(&mut self, hovered: bool, near: impl FnOnce() -> (f32, f32)) -> bool {
        let now = Instant::now();
        let dt = now - self.stepped_at;
        self.stepped_at = now;
        let resting = self.state.is_resting();
        let mut look = self.look;
        look.hover = step(look.hover, resting && hovered, dt, motion::duration::HOVER, motion::duration::EXIT, self.reduced);
        look.grow = step(look.grow, !resting, dt, motion::duration::ENTER, motion::duration::EXIT, self.reduced);
        let target = match self.state {
            OverlayState::Listening(l) | OverlayState::Locked(l) => l.clamp(0.0, 1.0),
            _ => 0.0,
        };
        look.level = follow(look.level, target, dt, self.reduced);
        if self.state.is_voice() {
            if self.reduced {
                // no drift: the whole field shows the voice now
                look.history = [look.level; HIST];
            } else {
                look.history[0] = look.level;
                // still once the trace is flat, so a quiet pill doesn't repaint every frame
                if look.history.iter().any(|&v| v > 0.005) {
                    look.scroll += dt.as_secs_f32() * DRIFT / PITCH;
                    let shifts = (look.scroll.floor() as usize).min(HIST);
                    if shifts > 0 {
                        look.history.copy_within(0..HIST - shifts, shifts);
                        look.history[..shifts].fill(look.level);
                        look.scroll = look.scroll.fract();
                    }
                }
            }
        }
        let (near, near_x) = if resting && !self.reduced { near() } else { (0.0, look.near_x) };
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
        look.braid = self.processing_at.map_or(0.0, |t0| {
            if self.reduced {
                // a still braid that steps a third of a turn each beat
                processing_dot(now - t0) as f32 * BRAID_TURN / 3.0
            } else {
                (now - t0).as_secs_f32() % (BRAID_TURN * 1000.0)
            }
        });
        let changed = look != self.look;
        self.look = look;
        changed
    }

    /// True when it changed.
    fn set_quiet(&mut self, quiet: bool) -> bool {
        let changed = self.look.quiet != quiet;
        self.look.quiet = quiet;
        changed
    }

    /// One soft pulse of the resting dot: your words landed.
    fn pulse(&mut self) {
        self.pulse_at = Some(Instant::now());
    }
}

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
pub use mac::{live, Overlay};

#[cfg(windows)]
pub struct Overlay {
    hwnd: HWND,
    pill: Pill,
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
        // Restart Manager (an installer replacing murmur.exe) or a sign-out: carrying on makes
        // the installer wait out its timeout before it kills us
        WM_ENDSESSION if wp.0 != 0 => {
            log::info!("session ending (lparam {:#x}); exiting", lp.0);
            unsafe { PostQuitMessage(0) };
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
            Ok(Overlay { hwnd, pill: Pill::new(reduced_motion()) })
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
        let (w, h, pixels) = render(self.pill.state, self.pill.look);
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
        let was_resting = self.pill.state.is_resting();
        self.pill.set(state);
        if !state.is_resting() {
            // the live pill is click-through, so no leave message will come
            HOVERED.store(false, Ordering::Relaxed);
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
        if self.pill.state.is_resting() {
            self.paint();
        }
    }

    /// Steps the pill one frame; repaints only when the look changed.
    pub fn animate(&mut self) {
        let hwnd = self.hwnd;
        if self.pill.step(HOVERED.load(Ordering::Relaxed), || cursor_nearness(hwnd)) {
            self.paint();
        }
    }

    pub fn set_quiet(&mut self, quiet: bool) {
        if self.pill.set_quiet(quiet) {
            self.paint();
        }
    }

    /// One soft pulse of the resting dot: your words landed.
    pub fn pulse(&mut self) {
        self.pill.pulse();
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

/// (strength, where along) for the cursor near the pill window `hwnd`.
#[cfg(windows)]
fn cursor_nearness(hwnd: HWND) -> (f32, f32) {
    unsafe {
        let mut pt = POINT::default();
        let mut r = RECT::default();
        if GetCursorPos(&mut pt).is_err() || GetWindowRect(hwnd, &mut r).is_err() {
            return (0.0, 0.5);
        }
        nearness((pt.x, pt.y), r)
    }
}

/// Where the pill hangs. Free on the desktop it's a rounded plate (Windows, bottom-centre); on a
/// Mac it's an island hanging from the top edge of the screen, grown out of the notch when there
/// is one (`notch_w`, `notch_h` points; both 0 without one, where it hangs below the menu bar).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) enum Mount {
    #[default]
    Plate,
    #[cfg_attr(windows, allow(dead_code))]
    Island { notch_w: f32, notch_h: f32 },
}

/// Points the island reaches past the notch each side at rest: room for the status dot.
const EAR: f32 = 26.0;
/// Points of black between the island's edge and the dot field.
const PAD: f32 = 4.0;
/// The bottom corner radius of a resting island, about a notch's own.
const NOTCH_R: f32 = 8.0;

/// The shape for eased growth `g`: window size (units), the dot field, and for an island its
/// half width, height and bottom radius.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Layout {
    w: i32,
    h: i32,
    field: Field,
    island: Option<(f32, f32, f32)>,
}

fn layout(g: f32, mount: Mount) -> Layout {
    let (fw, fh) = size(g);
    let (fw, fh) = (fw as f32, fh as f32);
    match mount {
        Mount::Plate => Layout { w: fw as i32, h: fh as i32, field: Field { cx: fw / 2.0, cy: fh / 2.0, w: fw, h: fh }, island: None },
        Mount::Island { notch_w, notch_h } => {
            let notched = notch_h > 0.0;
            let (hw, fw, height, top, r) = if notched {
                // hidden behind the notch at rest, the field drops out below it as it grows
                let hw = (notch_w / 2.0 + EAR).max(fw / 2.0 + PAD);
                let height = notch_h + (fh + 2.0 * PAD) * g;
                (hw, lerp(fw, 2.0 * (hw - PAD), g), height, notch_h + PAD * g, lerp(NOTCH_R, fh / 2.0 + PAD, g))
            } else {
                (fw / 2.0 + PAD, fw, PAD / 2.0 + fh + PAD, PAD / 2.0, fh / 2.0 + PAD)
            };
            let r = r.min(height).min(hw);
            let fillet = (height * 0.25).clamp(0.0, 6.0);
            let w = (2.0 * (hw + fillet)).ceil() as i32;
            Layout { w, h: height.ceil() as i32, field: Field { cx: w as f32 / 2.0, cy: top + fh / 2.0, w: fw, h: fh }, island: Some((hw, height, r)) }
        }
    }
}

/// The last island drawn: its size, shape and scale (as bits), and the canvas.
type IslandCache = Option<((i32, i32, u32, u32, u32, u32), Canvas)>;

thread_local! {
    static ISLAND: std::cell::RefCell<IslandCache> = const { std::cell::RefCell::new(None) };
}

/// A `w`×`h` canvas at `scale` with the island (half width, height, bottom radius) drawn solid
/// black. Drawing it means a distance for every pixel, so the last one is kept and reused while
/// the shape holds: it only changes as the island grows.
fn island_canvas(w: i32, h: i32, (hw, height, r): (f32, f32, f32), scale: f32) -> Canvas {
    let key = (w, h, hw.to_bits(), height.to_bits(), r.to_bits(), scale.to_bits());
    ISLAND.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some((k, c)) = cache.as_ref() {
            if *k == key {
                return c.clone();
            }
        }
        let mut c = Canvas::scaled(w, h, scale);
        let wf = w as f32;
        c.shape(|x, y| island_distance(x - wf / 2.0, y, hw, height, r), 0x000000, 1.0);
        *cache = Some((key, c.clone()));
        c
    })
}

/// Paint `state` into premultiplied BGRA pixels, row-major, `w * h` long, as a free plate. The
/// size follows `look.grow`; `look.hover` wakes the resting pill (full body opacity plus the
/// "more" dots).
#[cfg_attr(target_os = "macos", allow(dead_code))] // macOS draws at the screen's scale
fn render(state: OverlayState, look: Look) -> (i32, i32, Vec<u32>) {
    render_scaled(state, look, Mount::Plate, 1.0)
}

/// `render` hung as `mount`, at `scale` pixels per unit: (w, h) stay in units, the pixels are
/// `scale` times as many each way.
fn render_scaled(state: OverlayState, look: Look, mount: Mount, scale: f32) -> (i32, i32, Vec<u32>) {
    let grow = ease(motion::easing::STANDARD, look.grow.clamp(0.0, 1.0));
    let hover = if state.is_resting() { ease(motion::easing::STANDARD, look.hover.clamp(0.0, 1.0)) } else { 0.0 };
    let l = layout(grow, mount);
    let f = l.field;
    let mut c = match l.island {
        // the island is part of the notch: solid black
        Some(shape) => island_canvas(l.w, l.h, shape, scale),
        None => Canvas::scaled(l.w, l.h, scale),
    };
    let (wf, hf) = (l.w as f32, l.h as f32);
    // where the resting status dot sits, and the "more" dots that hint at the right-click menu
    let (dot_at, more) = match (mount, l.island) {
        (Mount::Island { notch_w, notch_h }, Some(_)) => {
            if notch_h > 0.0 {
                // in the ears either side of the notch: status right, menu hint left
                let ear = |side: f32| wf / 2.0 + side * (notch_w / 2.0 + EAR / 2.0);
                ((ear(1.0), notch_h / 2.0), [-4.0, 0.0, 4.0].map(|d| (ear(-1.0) + d, notch_h / 2.0)))
            } else {
                ((f.cx, f.cy), more_dots(f.w, f.h).map(|(x, y)| (f.cx - f.w / 2.0 + x, f.cy - f.h / 2.0 + y)))
            }
        }
        _ => {
            let (_, _, rest_alpha) = OverlayState::Idle.geometry();
            let (_, _, live_alpha) = OverlayState::Processing.geometry();
            let alpha = rest_alpha as f32 + (live_alpha as f32 - rest_alpha as f32) * grow.max(hover);
            // body: dim when resting, near-opaque when live or hovered
            c.capsule(0.0, 0.0, wf, hf, 0x161616, alpha / 255.0);
            ((wf / 2.0, hf / 2.0), more_dots(wf, hf))
        }
    };
    match state {
        OverlayState::Idle | OverlayState::Paused => {
            // shrinking back from live: the dot fades in as the pill settles
            let fade = 1.0 - grow;
            let rgb = if state == OverlayState::Idle { GREEN } else { 0x808080 };
            if look.near > 0.0 {
                // gathers toward an approaching cursor; the hover dots take over on arrival
                c.glow(look.near_x * wf, dot_at.1, (IDLE_H as f32) * 2.5, GREEN, 0.35 * look.near * (1.0 - hover) * fade);
            }
            let d = (IDLE_H - 8) as f32;
            let (x, y) = dot_at;
            if look.pulse > 0.0 {
                let s = (std::f32::consts::PI * look.pulse).sin();
                let ring = d + 6.0 * s;
                c.capsule(x - ring / 2.0, y - ring / 2.0, ring, ring, rgb, 0.35 * s * fade);
            }
            c.capsule(x - d / 2.0, y - d / 2.0, d, d, rgb, fade);
            if hover > 0.0 {
                let r = 1.2;
                for (x, y) in more {
                    c.capsule(x - r, y - r, 2.0 * r, 2.0 * r, 0xE0E0E0, hover * fade);
                }
            }
        }
        OverlayState::Listening(_) | OverlayState::Locked(_) => {
            // live content arrives late in the growth, once there's room for it
            let show = grow * grow;
            let locked = matches!(state, OverlayState::Locked(_));
            // the hands-free dot takes the right end; the trace stops short of it
            let stop = if locked { f.w / 2.0 - 34.0 } else { f32::INFINITY };
            c.glow(f.cx, f.cy, 34.0 + 20.0 * look.level, mix_rgb(GREEN, CORE, 0.5), 0.165 * look.level.min(1.0) * show);
            dots(&mut c, f, show, |dx, dy| if dx > stop { (0.0, 0.0) } else { scope_light(&look, dx, dy, f.h / 2.0) });
            if locked {
                let d = 10.0;
                c.capsule(f.cx + f.w / 2.0 - 25.0 - d / 2.0, f.cy - d / 2.0, d, d, 0xE04040, show);
            }
        }
        OverlayState::Processing => {
            let show = grow * grow;
            dots(&mut c, f, show, |dx, dy| braid_light(&look, f, dx, dy));
        }
    }
    (l.w, l.h, c.into_bgra())
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

    /// A settled live look with `v` of voice across the whole trace.
    fn voiced(v: f32) -> Look {
        Look { grow: 1.0, level: v, history: [v; HIST], ..Default::default() }
    }

    fn green(p: u32) -> u32 {
        (p >> 8) & 0xFF
    }

    #[test]
    fn scope_lights_up_with_the_voice_and_stays_inside() {
        let (w, h, quiet) = render(OverlayState::Listening(0.0), voiced(0.0));
        let (_, _, loud) = render(OverlayState::Listening(1.0), voiced(1.0));
        let lit = |px: &[u32]| px.iter().map(|&p| green(p)).sum::<u32>();
        assert!(lit(&loud) > lit(&quiet) * 2, "{} vs {}", lit(&loud), lit(&quiet));
        // the middle rows show a faint line even at silence
        let mid = |px: &[u32]| (0..w).flat_map(|x| (h / 2 - 3..=h / 2 + 3).map(move |y| (x, y))).map(|(x, y)| green(px[(y * w + x) as usize])).max().unwrap();
        assert!(mid(&quiet) > 0x18 && mid(&loud) > mid(&quiet) + 0x40);
        // the dots never touch the rim (the glow, which follows `level`, may)
        let (_, _, dots_only) = render(OverlayState::Listening(1.0), Look { level: 0.0, ..voiced(1.0) });
        for x in 0..w {
            for y in [0, h - 1] {
                assert!(green(dots_only[(y * w + x) as usize]) < 0x30, "lit rim at ({x},{y})");
            }
        }
    }

    #[test]
    fn the_newest_voice_is_in_the_middle_and_the_past_drifts_out() {
        let mut history = [0.0; HIST];
        history[0] = 1.0;
        let (w, h, px) = render(OverlayState::Listening(1.0), Look { grow: 1.0, level: 1.0, history, ..Default::default() });
        let column = |x: i32| (0..h).map(|y| green(px[(y * w + x) as usize])).sum::<u32>();
        assert!(column(w / 2) > column(w / 2 - 30) + 0x100, "a word just now lights the middle");
        let mut history = [0.0; HIST];
        history[6] = 1.0;
        let (_, _, px) = render(OverlayState::Listening(0.0), Look { grow: 1.0, history, ..Default::default() });
        let column = |x: i32| (0..h).map(|y| green(px[(y * w + x) as usize])).sum::<u32>();
        let out = (6.0 * PITCH) as i32;
        assert!(column(w / 2 + out) > column(w / 2) + 0x80, "a word six columns ago has drifted out");
        assert!((column(w / 2 + out) as i32 - column(w / 2 - out) as i32).abs() < 0x40, "both ways alike");
    }

    #[test]
    fn the_trace_drifts_while_there_is_voice_and_rests_when_flat() {
        let mut pill = Pill::new(false);
        pill.set(OverlayState::Listening(0.0));
        pill.look.grow = 1.0;
        pill.stepped_at -= Duration::from_millis(50);
        pill.step(false, || (0.0, 0.5));
        assert_eq!(pill.look.scroll, 0.0, "a flat line doesn't move");
        pill.set(OverlayState::Listening(1.0));
        pill.look.level = 1.0;
        pill.stepped_at -= Duration::from_secs_f32(2.5 * PITCH / DRIFT);
        pill.step(false, || (0.0, 0.5));
        assert!(pill.look.history[1] > 0.5 && pill.look.history[2] > 0.5, "{:?}", &pill.look.history[..4]);
        assert!(pill.look.scroll < 1.0);
        // a new dictation starts flat
        pill.set(OverlayState::Processing);
        pill.set(OverlayState::Listening(0.0));
        assert_eq!(pill.look.history, [0.0; HIST]);
    }

    #[test]
    fn locked_scope_leaves_the_red_dot_alone() {
        let (w, h, px) = render(OverlayState::Locked(1.0), voiced(1.0));
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
    fn processing_braid_turns_with_time() {
        let d = motion::scaled(motion::duration::EMPHASIS);
        assert_eq!(processing_dot(Duration::ZERO), 0);
        assert_eq!(processing_dot(d), 1);
        assert_eq!(processing_dot(d * 3), 0);
        let live = Look { grow: 1.0, ..Default::default() };
        let (_, _, first) = render(OverlayState::Processing, live);
        let (_, _, later) = render(OverlayState::Processing, Look { braid: BRAID_TURN / 4.0, ..live });
        assert_ne!(first, later, "the braid moved");
        let (_, _, round) = render(OverlayState::Processing, Look { braid: BRAID_TURN, ..live });
        assert_eq!(first, round, "a whole turn comes back round");
    }

    #[test]
    fn quiet_dims_the_braid() {
        let live = Look { grow: 1.0, ..Default::default() };
        let (_, _, loud) = render(OverlayState::Processing, live);
        let (_, _, quiet) = render(OverlayState::Processing, Look { quiet: true, ..live });
        let lit = |px: &[u32]| px.iter().map(|&p| green(p)).sum::<u32>();
        assert!(lit(&quiet) * 10 < lit(&loud) * 8, "{} vs {}", lit(&quiet), lit(&loud));
    }

    #[test]
    fn the_island_hangs_from_the_notch_and_grows_below_it() {
        let notch = Mount::Island { notch_w: 180.0, notch_h: 32.0 };
        let rest = layout(0.0, notch);
        let (hw, height, _) = rest.island.unwrap();
        assert_eq!(height, 32.0, "at rest it's as tall as the notch");
        assert!(hw >= 90.0 + EAR - 0.01, "with an ear either side");
        let live = layout(1.0, notch);
        let (_, height, _) = live.island.unwrap();
        assert_eq!(height, 32.0 + H as f32 + 2.0 * PAD);
        assert!(live.field.cy - live.field.h / 2.0 >= 32.0, "the dots show below the notch");
        // solid black from the top edge, clear past the fillets
        let (w, h, px) = render_scaled(OverlayState::Listening(0.0), voiced(0.0), notch, 1.0);
        assert_eq!((w, h), (live.w, live.h));
        assert_eq!(px[(w / 2) as usize], 0xFF000000);
        assert_eq!(px[((h - 1) * w) as usize], 0, "bottom corner is round");
        // the resting status dot sits in the right ear, inside the notch's height
        let (w, _, px) = render_scaled(OverlayState::Idle, look_for(OverlayState::Idle), notch, 1.0);
        let ear = (w as f32 / 2.0 + 90.0 + EAR / 2.0) as i32;
        assert!(green(px[(16 * w + ear) as usize]) > 0x80, "{:08X}", px[(16 * w + ear) as usize]);
    }

    #[test]
    fn without_a_notch_the_island_hangs_below_the_menu_bar() {
        let l = layout(0.0, Mount::Island { notch_w: 0.0, notch_h: 0.0 });
        let (hw, height, _) = l.island.unwrap();
        assert_eq!((hw, height), (IDLE_W as f32 / 2.0 + PAD, PAD / 2.0 + IDLE_H as f32 + PAD));
        let (w, h, px) = render_scaled(OverlayState::Idle, look_for(OverlayState::Idle), Mount::Island { notch_w: 0.0, notch_h: 0.0 }, 1.0);
        let p = px[((l.field.cy as i32) * w + w / 2) as usize];
        assert!(green(p) > (p >> 16) & 0xFF, "idle dot not green: {p:08X}");
        assert!(h > 0);
    }

    fn alpha(p: u32) -> u32 {
        p >> 24
    }

    #[test]
    fn body_is_opaque_inside_and_clear_at_corners() {
        let (w, _, px) = render(OverlayState::Listening(0.0), look_for(OverlayState::Listening(0.0)));
        // just inside the top edge, clear of the dots
        assert_eq!(alpha(px[(w + w / 2) as usize]), 0xE6);
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

    #[test]
    fn a_live_island_frame_is_cheap() {
        let notch = Mount::Island { notch_w: 185.0, notch_h: 32.0 };
        let look = voiced(0.6);
        render_scaled(OverlayState::Listening(0.6), look, notch, 2.0);
        let t0 = std::time::Instant::now();
        for _ in 0..20 {
            render_scaled(OverlayState::Listening(0.6), look, notch, 2.0);
        }
        let each = t0.elapsed() / 20;
        eprintln!("live island frame: {each:?}");
        // the main thread draws this every frame and must keep hearing the keyboard. Loose
        // enough for a busy shared CI runner running other tests alongside (2.6 ms seen); the
        // freeze this guards against was over 20 ms a frame
        let budget = Duration::from_millis(if cfg!(debug_assertions) { 40 } else { 8 });
        assert!(each < budget, "{each:?} a frame");
    }
}
