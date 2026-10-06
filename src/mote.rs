//! The carved moment: when you let go of the key, a mote flies from the pill to your caret,
//! waits there while the speech is transcribed, and dissolves into your words.

use crate::canvas::{self, Canvas, Span};
use crate::editor_kit::{ease, reduced_motion};
use crate::motion;
use crate::platform::Rect as RECT;
use anyhow::Result;
#[cfg(windows)]
use anyhow::anyhow;
use std::time::{Duration, Instant};
#[cfg(windows)]
use windows::core::PCWSTR;
#[cfg(windows)]
use windows::Win32::Foundation::{GetLastError, ERROR_CLASS_ALREADY_EXISTS, HWND, LPARAM, LRESULT, WPARAM};
#[cfg(windows)]
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
#[cfg(windows)]
use windows::Win32::UI::HiDpi::GetDpiForSystem;
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, RegisterClassW, SetWindowPos, ShowWindow, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SW_HIDE, SW_SHOWNOACTIVATE, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};

pub(crate) type Pt = (f32, f32);

/// The flight's length. The carved moment breaks the grammar here; logged in the kit ledger.
#[cfg(windows)]
pub(crate) const FLIGHT: Duration = Duration::from_millis(300);
/// Longer on a Mac, where 300 ms read as too quick (Jeff, 2026-10-05).
#[cfg(target_os = "macos")]
pub(crate) const FLIGHT: Duration = Duration::from_millis(450);

/// The least time the mote stays at the caret before dissolving. Words land in 0.1–0.3 s on a
/// Mac, often mid-flight, so without a stay the mote sometimes swells away on arrival and
/// sometimes sits first, and reads as an inconsistent dot (Jeff, 2026-10-05).
#[cfg(windows)]
const DWELL: Duration = Duration::ZERO;
#[cfg(target_os = "macos")]
const DWELL: Duration = Duration::from_millis(250);

/// A point on a quadratic arc from `from` to `to` at `t` (0..=1). The arc lobs upward, like
/// something tossed: its control point sits off the midpoint on the screen-up side.
pub(crate) fn arc_point(from: Pt, to: Pt, t: f32) -> Pt {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let len = dx.hypot(dy).max(1.0);
    // the perpendicular that points up the screen (negative y)
    let (mut px, mut py) = (-dy / len, dx / len);
    if py > 0.0 {
        px = -px;
        py = -py;
    }
    let bulge = 0.2 * len;
    let ctrl = ((from.0 + to.0) / 2.0 + px * bulge, (from.1 + to.1) / 2.0 + py * bulge);
    let u = 1.0 - t;
    (
        u * u * from.0 + 2.0 * u * t * ctrl.0 + t * t * to.0,
        u * u * from.1 + 2.0 * u * t * ctrl.1 + t * t * to.1,
    )
}

/// What the mote says: runs of text, each in its own colour.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Message(pub Vec<Span>);

impl Message {
    pub(crate) fn plain(s: &str) -> Message {
        Message(vec![(s.to_string(), crate::correction_ui::rgb(crate::correction_ui::TEXT))])
    }

    pub(crate) fn chars(&self) -> usize {
        self.0.iter().map(|(s, _)| s.chars().count()).sum()
    }
}

/// How long a message stays open: long enough to read it.
pub(crate) fn hold_for(m: &Message) -> Duration {
    motion::duration::LOCATE.max(motion::duration::READ_PER_CHAR * m.chars() as u32)
}

/// Where a message is said: at a caret it grows rightward from the caret; above the pill it's centred.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Target {
    Caret(Pt),
    Pill(Pt),
}

/// The landing point for a caret rect: its left edge, halfway down the line.
pub(crate) fn caret_point(r: RECT) -> Pt {
    (r.left as f32, (r.top + r.bottom) as f32 / 2.0)
}

/// Text opacity for capsule openness `open`: the words come in once it's about 70% wide.
fn words_for(open: f32) -> f32 {
    ((open - 0.7) / 0.3).clamp(0.0, 1.0)
}

/// What to draw this frame: where, how opaque, how large (1 = normal), and how far a message
/// capsule has opened (0 = a dot) and how visible its words are.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Sprite {
    pub at: Pt,
    pub alpha: f32,
    pub radius: f32,
    pub open: f32,
    pub words: f32,
}

/// What a flight does on arrival.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Then {
    Settle,
    Dissolve,
    Say,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    Hidden,
    Flying { from: Pt, to: Pt, start: Instant, then: Then },
    Settled { at: Pt, since: Instant },
    /// unfurling into the message, then holding it open
    Speaking { at: Pt, start: Instant },
    /// pulling back to a dot from openness `open`
    Furling { at: Pt, start: Instant, open: f32 },
    /// before `start`, the plain dot
    Leaving { at: Pt, start: Instant, alpha: f32, grow: bool },
}

pub(crate) struct Flight {
    phase: Phase,
    reduced: bool,
    message: Option<Message>,
    centred: bool,
    /// the message holds until dismissed instead of for its read time
    sticky: bool,
}

impl Flight {
    pub(crate) fn new(reduced: bool) -> Flight {
        Flight { phase: Phase::Hidden, reduced, message: None, centred: false, sticky: false }
    }

    /// Starts a flight from the pill to the caret, replacing whatever was showing.
    pub(crate) fn launch(&mut self, from: Pt, to: Pt, now: Instant) {
        self.sticky = false;
        self.message = None;
        self.phase = if self.reduced { Phase::Settled { at: to, since: now } } else { Phase::Flying { from, to, start: now, then: Then::Settle } };
    }

    /// The words landed: grow and fade into them. Mid-flight, it finishes the flight first.
    pub(crate) fn dissolve(&mut self, now: Instant) {
        if let Phase::Flying { then, .. } = &mut self.phase {
            *then = Then::Dissolve;
            self.sticky = false;
            return;
        }
        self.leave(now, true);
    }

    /// Says `m`: a mote already out says it where it is (finishing a flight first); otherwise
    /// one flies from `from` to the target and says it there.
    pub(crate) fn say(&mut self, m: Message, from: Pt, to: Target, now: Instant) {
        self.sticky = false;
        self.message = Some(m);
        self.phase = match self.phase {
            Phase::Flying { from, to, start, then } => {
                // a dictation's flight ends at a caret; an earlier message keeps its own placement
                if then != Then::Say {
                    self.centred = false;
                }
                Phase::Flying { from, to, start, then: Then::Say }
            }
            Phase::Settled { at, .. } => {
                self.centred = false;
                Phase::Speaking { at, start: now }
            }
            // an open message is replaced where it is, keeping its placement
            Phase::Speaking { at, .. } => Phase::Speaking { at, start: now },
            _ => {
                let (at, centred) = match to {
                    Target::Caret(p) => (p, false),
                    Target::Pill(p) => (p, true),
                };
                self.centred = centred;
                if self.reduced { Phase::Speaking { at, start: now } } else { Phase::Flying { from, to: at, start: now, then: Then::Say } }
            }
        };
    }

    /// Says `m` like `say`, but it stays open until dismissed (or replaced).
    pub(crate) fn say_until_dismissed(&mut self, m: Message, from: Pt, to: Target, now: Instant) {
        self.say(m, from, to, now);
        self.sticky = true;
    }

    /// Closes an open message early, from however far it has opened.
    pub(crate) fn dismiss(&mut self, now: Instant) {
        match self.phase {
            Phase::Speaking { at, .. } => {
                // read while still sticky: unstuck, a message past its read time is already gone
                let open = self.sprite(now).map_or(0.0, |s| s.open);
                self.sticky = false;
                // the call above may have run the phase on already (furling, or gone); leave that be
                if matches!(self.phase, Phase::Speaking { .. }) {
                    self.phase = if self.reduced { Phase::Hidden } else { Phase::Furling { at, start: now, open } };
                }
            }
            Phase::Flying { then: Then::Say, .. } => self.leave(now, false),
            _ => {}
        }
        self.sticky = false;
    }

    /// Whether a message is on its way, open, or closing.
    pub(crate) fn is_speaking(&self) -> bool {
        self.message.is_some() && matches!(self.phase, Phase::Flying { then: Then::Say, .. } | Phase::Speaking { .. } | Phase::Furling { .. })
    }

    pub(crate) fn message(&self) -> Option<&Message> {
        self.message.as_ref()
    }

    /// Whether `m` is the message on its way, open, or closing.
    pub(crate) fn is_saying(&self, m: &Message) -> bool {
        self.is_speaking() && self.message.as_ref() == Some(m)
    }

    pub(crate) fn centred(&self) -> bool {
        self.centred
    }

    /// No words, an error, or you moved on: fade where it is.
    pub(crate) fn fade(&mut self, now: Instant) {
        self.leave(now, false);
    }

    fn leave(&mut self, now: Instant, grow: bool) {
        if self.reduced {
            self.sticky = false;
            self.phase = Phase::Hidden;
            return;
        }
        // read while still sticky, as in `dismiss`
        if let Some(s) = self.sprite(now) {
            // a mote that has only just landed stays its minimum first
            let start = match self.phase {
                Phase::Settled { since, .. } => now.max(since + motion::scaled(DWELL)),
                _ => now,
            };
            self.phase = Phase::Leaving { at: s.at, start, alpha: s.alpha, grow };
        }
        self.sticky = false;
    }

    pub(crate) fn is_active(&self) -> bool {
        self.phase != Phase::Hidden
    }

    /// The frame at `now`, moving on to the next phase when one ends; None once gone.
    pub(crate) fn sprite(&mut self, now: Instant) -> Option<Sprite> {
        let secs = |since: Instant, d: Duration| now.saturating_duration_since(since).as_secs_f32() / motion::scaled(d).as_secs_f32();
        let phase = self.phase;
        match phase {
            Phase::Hidden => None,
            Phase::Flying { from, to, start, then } => {
                let t = secs(start, FLIGHT);
                if t >= 1.0 {
                    let landed = start + motion::scaled(FLIGHT);
                    self.phase = match then {
                        Then::Settle => Phase::Settled { at: to, since: landed },
                        Then::Dissolve => Phase::Leaving { at: to, start: landed + motion::scaled(DWELL), alpha: 1.0, grow: true },
                        Then::Say => Phase::Speaking { at: to, start: landed },
                    };
                    return self.sprite(now);
                }
                Some(Sprite { at: arc_point(from, to, ease(motion::easing::ENTER, t)), alpha: 1.0, radius: 1.0, open: 0.0, words: 0.0 })
            }
            Phase::Speaking { at, start } => {
                let unfurl = if self.reduced { Duration::ZERO } else { motion::scaled(motion::duration::ENTER) };
                let hold = motion::scaled(self.message.as_ref().map_or(motion::duration::LOCATE, hold_for));
                let el = now.saturating_duration_since(start);
                if !self.sticky && el >= unfurl + hold {
                    self.phase = if self.reduced { Phase::Hidden } else { Phase::Furling { at, start: start + unfurl + hold, open: 1.0 } };
                    return self.sprite(now);
                }
                let open = if unfurl.is_zero() { 1.0 } else { ease(motion::easing::ENTER, (el.as_secs_f32() / unfurl.as_secs_f32()).min(1.0)) };
                Some(Sprite { at, alpha: 1.0, radius: 1.0, open, words: words_for(open) })
            }
            Phase::Furling { at, start, open } => {
                let t = secs(start, motion::duration::EXIT);
                if t >= 1.0 {
                    self.phase = Phase::Leaving { at, start: start + motion::scaled(motion::duration::EXIT), alpha: 1.0, grow: true };
                    return self.sprite(now);
                }
                // the words go in the first half; the capsule pulls back over the whole furl
                let words = (1.0 - 2.0 * t).max(0.0).min(words_for(open));
                Some(Sprite { at, alpha: 1.0, radius: 1.0, open: open * (1.0 - ease(motion::easing::EXIT, t)), words })
            }
            Phase::Settled { at, since } => {
                // breathes slowly while the words are on their way
                let breath = (secs(since, motion::duration::LOCATE) * std::f32::consts::TAU).cos();
                let alpha = if self.reduced { 1.0 } else { 0.85 + 0.15 * breath };
                Some(Sprite { at, alpha, radius: 1.0, open: 0.0, words: 0.0 })
            }
            Phase::Leaving { at, start, alpha, grow } => {
                let t = secs(start, motion::duration::EXIT);
                if t >= 1.0 {
                    self.phase = Phase::Hidden;
                    return None;
                }
                let e = ease(motion::easing::EXIT, t);
                Some(Sprite { at, alpha: alpha * (1.0 - e), radius: if grow { 1.0 + 0.6 * e } else { 1.0 }, open: 0.0, words: 0.0 })
            }
        }
    }
}

/// Where to fly for a caret found after key release: only for the dictation still waiting on
/// its words (`awaiting` holds its window), and only if that window is still in front.
pub(crate) fn landing_point(awaiting: Option<isize>, target: isize, foreground: isize, caret: Option<RECT>) -> Option<Pt> {
    let r = caret?;
    (awaiting == Some(target) && foreground == target).then(|| caret_point(r))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Landing {
    Dissolve,
    Fade,
    Pulse,
    Nothing,
}

/// How the words land when the pipeline finishes.
pub(crate) fn on_done(mote_active: bool, has_text: bool, same_window: bool) -> Landing {
    match (mote_active, has_text) {
        (true, true) if same_window => Landing::Dissolve,
        (true, _) => Landing::Fade,
        (false, true) => Landing::Pulse,
        (false, false) => Landing::Nothing,
    }
}

/// The mote window's side, in physical pixels.
const S: i32 = 40;

#[cfg(test)]
fn render(s: &Sprite) -> Vec<u32> {
    render_scaled(s, 1.0)
}

/// `render` at `scale` pixels per unit.
fn render_scaled(s: &Sprite, scale: f32) -> Vec<u32> {
    let mut c = Canvas::scaled(S, S, scale);
    let m = S as f32 / 2.0;
    c.halo(m, m, 12.0 * s.radius, 0x60D060, 0.5 * s.alpha);
    let core = 5.0 * s.radius;
    c.capsule(m - core / 2.0, m - core / 2.0, core, core, 0xE8FFE8, s.alpha);
    c.into_bgra()
}

/// The mote's core diameter, which a message capsule grows from.
const DOT: f32 = 5.0;
/// Room around the capsule for its rim and the fading halo.
const MARGIN: f32 = 8.0;

/// A message capsule's centre and size, physical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TagBox {
    pub cx: f32,
    pub cy: f32,
    pub w: f32,
    pub h: f32,
    pub pad: f32,
}

/// The capsule for text of size `text` at openness `open`: the dot at `at` when closed; open,
/// it sits above the line, its left edge at the caret (or centred over a pill target).
fn tag_box(at: Pt, open: f32, text: (i32, i32), scale: f32, centred: bool) -> TagBox {
    let pad = 12.0 * scale;
    let (full_w, full_h) = (text.0 as f32 + 2.0 * pad, 26.0 * scale);
    let lerp = |a: f32, b: f32| a + (b - a) * open;
    let (w, h) = (lerp(DOT, full_w), lerp(DOT, full_h));
    let lift = full_h / 2.0 + 16.0 * scale;
    let cx = if centred { at.0 } else { at.0 - DOT / 2.0 + w / 2.0 };
    TagBox { cx, cy: at.1 - lift * open, w, h, pad }
}

/// Top-left for a `w`×`h` window at (x, y), pulled inside `work`.
fn fit(x: i32, y: i32, w: i32, h: i32, work: RECT) -> (i32, i32) {
    (x.clamp(work.left, (work.right - w).max(work.left)), y.clamp(work.top, (work.bottom - h).max(work.top)))
}

/// A frame of a speaking mote, `w`×`h`, with the capsule centred at `centre`.
#[allow(clippy::too_many_arguments)]
fn render_tag(s: &Sprite, m: &Message, b: &TagBox, centre: Pt, w: i32, h: i32, font: i32, text: (i32, i32), scale: f32) -> Vec<u32> {
    let mut c = Canvas::scaled(w, h, scale);
    let (cx, cy) = centre;
    let dot = 1.0 - s.open;
    if dot > 0.0 {
        c.halo(cx, cy, 12.0 * s.radius, 0x60D060, 0.5 * s.alpha * dot);
    }
    c.capsule(cx - b.w / 2.0, cy - b.h / 2.0, b.w, b.h, 0x202020, 0.9 * s.alpha);
    if dot > 0.0 {
        let core = DOT * s.radius;
        c.capsule(cx - core / 2.0, cy - core / 2.0, core, core, 0xE8FFE8, s.alpha * dot);
    }
    if s.words > 0.0 {
        c.text(cx - b.w / 2.0 + b.pad, cy - text.1 as f32 / 2.0, &m.0, font, text, s.words * s.alpha);
    }
    c.into_bgra()
}

#[cfg(windows)]
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

/// Records a frame's inputs in `last`; whether they differ from the frame before.
fn changed(last: &mut Option<(Sprite, i32, i32, i32, i32)>, s: &Sprite, x: i32, y: i32, w: i32, h: i32) -> bool {
    let now = Some((*s, x, y, w, h));
    let differs = *last != now;
    *last = now;
    differs
}

/// The mote's own click-through window. It's created and moved in physical pixels, the space
/// `caret::find` reports in, while the rest of Murmur is DPI-unaware.
#[cfg(windows)]
struct Surface {
    hwnd: HWND,
}

#[cfg(windows)]
impl Surface {
    fn create() -> Result<Surface> {
        let hwnd = crate::caret::physical(|| unsafe {
            let hinst = GetModuleHandleW(None)?;
            let class: Vec<u16> = "MurmurMote\0".encode_utf16().collect();
            let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: hinst.into(), lpszClassName: PCWSTR(class.as_ptr()), ..Default::default() };
            if RegisterClassW(&wc) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
                return Err(anyhow!("RegisterClassW"));
            }
            Ok(CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                PCWSTR(class.as_ptr()),
                PCWSTR(class.as_ptr()),
                WS_POPUP,
                0, 0, S, S,
                None, None, Some(hinst.into()), None,
            )?)
        })?;
        Ok(Surface { hwnd })
    }

    /// (physical pixels per 96-dpi pixel, for the message's size; units per unit of what the app
    /// hands the mote, which is already physical pixels; pixels drawn per unit; dot size)
    fn scales(&self) -> (f32, f32, f32, f32) {
        (crate::caret::physical(|| unsafe { GetDpiForSystem() }) as f32 / 96.0, 1.0, 1.0, 1.0)
    }

    fn push(&self, x: i32, y: i32, w: i32, h: i32, px: &[u32]) {
        crate::caret::physical(|| canvas::push(self.hwnd, x, y, w, h, px));
    }

    fn show(&self) {
        unsafe {
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOSIZE | SWP_NOMOVE | SWP_NOACTIVATE);
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
        }
    }

    fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }
}

/// The mote's window on macOS: a still, click-through stage over the screen that the mote moves
/// across. The mote works in points, as the app does there (and `caret::find`), drawn at the
/// screen's pixels per point.
#[cfg(target_os = "macos")]
struct Surface {
    panel: crate::platform::Stage,
    /// pixels per point
    scale: f32,
}

#[cfg(target_os = "macos")]
impl Surface {
    fn create() -> Result<Surface> {
        Ok(Surface { panel: crate::platform::Stage::new()?, scale: crate::platform::work_area().1 })
    }

    /// (message scale, points per unit, pixels drawn per unit, dot size). A point-sized dot comes
    /// out about a third bigger than Windows' pixel-sized one on a typical laptop; 0.8 evens it.
    fn scales(&self) -> (f32, f32, f32, f32) {
        (1.0, 1.0, self.scale, 0.8)
    }

    fn push(&self, x: i32, y: i32, w: i32, h: i32, px: &[u32]) {
        let size = (canvas::pixels(w, self.scale) as usize, canvas::pixels(h, self.scale) as usize);
        self.panel.show((x as f32, y as f32, w as f32, h as f32), px, size, self.scale);
    }

    /// `push` orders the panel front the first time.
    fn show(&self) {}

    fn hide(&self) {
        self.panel.hide();
    }
}

pub(crate) struct Mote {
    surface: Surface,
    flight: Flight,
    shown: bool,
    /// physical pixels per 96-dpi pixel, for the message's size
    scale: f32,
    /// mote units per unit of the points the app hands in (1 on both systems today)
    unit: f32,
    /// pixels drawn per unit: 1 on Windows, the screen's pixels per point on macOS
    px_scale: f32,
    /// the dot's size against Windows'
    dot: f32,
    /// the current message's font size, text size and work area, measured once per message
    layout: Option<(i32, (i32, i32), RECT)>,
    /// the last frame pushed, to skip an identical one
    last: Option<(Sprite, i32, i32, i32, i32)>,
}

impl Mote {
    pub(crate) fn create() -> Result<Mote> {
        let surface = Surface::create()?;
        let (scale, unit, px_scale, dot) = surface.scales();
        Ok(Mote { surface, flight: Flight::new(reduced_motion()), shown: false, scale, unit, px_scale, dot, layout: None, last: None })
    }

    /// A point from the app in the mote's pixels.
    fn px(&self, (x, y): Pt) -> Pt {
        (x * self.unit, y * self.unit)
    }

    fn target(&self, to: Target) -> Target {
        match to {
            Target::Caret(p) => Target::Caret(self.px(p)),
            Target::Pill(p) => Target::Pill(self.px(p)),
        }
    }

    pub(crate) fn launch(&mut self, from: Pt, to: Pt) {
        self.layout = None;
        self.flight.launch(self.px(from), self.px(to), Instant::now());
    }

    pub(crate) fn dissolve(&mut self) {
        self.flight.dissolve(Instant::now());
    }

    pub(crate) fn fade(&mut self) {
        self.flight.fade(Instant::now());
    }

    pub(crate) fn is_active(&self) -> bool {
        self.flight.is_active()
    }

    pub(crate) fn say(&mut self, m: Message, from: Pt, to: Target) {
        self.layout = None;
        self.flight.say(m, self.px(from), self.target(to), Instant::now());
    }

    pub(crate) fn say_until_dismissed(&mut self, m: Message, from: Pt, to: Target) {
        self.layout = None;
        self.flight.say_until_dismissed(m, self.px(from), self.target(to), Instant::now());
    }

    pub(crate) fn dismiss(&mut self) {
        self.flight.dismiss(Instant::now());
    }

    pub(crate) fn is_speaking(&self) -> bool {
        self.flight.is_speaking()
    }

    pub(crate) fn is_saying(&self, m: &Message) -> bool {
        self.flight.is_saying(m)
    }

    /// The frame for `s`: (x, y, w, h, pixels), physical pixels; None when it's the one already
    /// on screen (a held message is the same frame every tick).
    fn frame(&mut self, s: &Sprite) -> Option<(i32, i32, i32, i32, Vec<u32>)> {
        let Some(m) = self.flight.message().filter(|_| s.open > 0.0) else {
            let (x, y) = ((s.at.0 - S as f32 / 2.0).round() as i32, (s.at.1 - S as f32 / 2.0).round() as i32);
            return changed(&mut self.last, s, x, y, S, S).then(|| (x, y, S, S, render_scaled(s, self.px_scale)));
        };
        let (font, text, work) = *self.layout.get_or_insert_with(|| {
            let font = (16.0 * self.scale).round() as i32;
            let at = RECT { left: s.at.0 as i32, top: s.at.1 as i32, right: s.at.0 as i32 + 1, bottom: s.at.1 as i32 + 1 };
            let work = crate::caret::work_area(&crate::caret::Anchor::Area(at));
            let u = |v: i32| (v as f32 * self.unit).round() as i32;
            (font, canvas::measure(&m.0, font), RECT { left: u(work.left), top: u(work.top), right: u(work.right), bottom: u(work.bottom) })
        });
        let b = tag_box(s.at, s.open, text, self.scale, self.flight.centred());
        let (w, h) = (((b.w + 2.0 * MARGIN).ceil() as i32).max(S), ((b.h + 2.0 * MARGIN).ceil() as i32).max(S));
        let (x0, y0) = ((b.cx - w as f32 / 2.0).round() as i32, (b.cy - h as f32 / 2.0).round() as i32);
        let (x, y) = fit(x0, y0, w, h, work);
        changed(&mut self.last, s, x, y, w, h).then(|| (x, y, w, h, render_tag(s, m, &b, (b.cx - x as f32, b.cy - y as f32), w, h, font, text, self.px_scale)))
    }

    /// Draws this frame of the flight, or hides the window once it's over.
    pub(crate) fn animate(&mut self) {
        match self.flight.sprite(Instant::now()).map(|s| Sprite { radius: s.radius * self.dot, ..s }) {
            Some(s) => {
                if let Some((x, y, w, h, px)) = self.frame(&s) {
                    self.surface.push(x, y, w, h, &px);
                }
                if !self.shown {
                    self.surface.show();
                    self.shown = true;
                }
            }
            None if self.shown => {
                self.surface.hide();
                self.shown = false;
                self.layout = None;
                self.last = None;
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }
    /// Easing is solved numerically, so t = 0 lands within a hair of the start, not on it.
    fn close(a: Pt, b: Pt) -> bool {
        (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01
    }
    const PILL: Pt = (960.0, 1000.0);
    const CARET: Pt = (400.0, 300.0);

    #[test]
    fn arc_starts_at_the_pill_ends_at_the_caret_and_lobs_upward() {
        assert_eq!(arc_point(PILL, CARET, 0.0), PILL);
        let end = arc_point(PILL, CARET, 1.0);
        assert!((end.0 - CARET.0).abs() < 1e-3 && (end.1 - CARET.1).abs() < 1e-3);
        let mid = arc_point(PILL, CARET, 0.5);
        let straight = ((PILL.0 + CARET.0) / 2.0, (PILL.1 + CARET.1) / 2.0);
        assert!(mid.1 < straight.1, "the arc bulges up: {mid:?} vs {straight:?}");
    }

    #[test]
    fn flies_then_settles_at_the_caret() {
        let t0 = Instant::now();
        let mut f = Flight::new(false);
        f.launch(PILL, CARET, t0);
        assert!(close(f.sprite(t0).unwrap().at, PILL));
        let s = f.sprite(t0 + FLIGHT + ms(50)).unwrap();
        assert_eq!(s.at, CARET);
        assert!(s.alpha > 0.6 && f.is_active());
    }

    #[test]
    fn dissolve_grows_and_fades_away_fade_just_fades() {
        let t0 = Instant::now();
        let settled = t0 + FLIGHT + ms(10);
        let mut f = Flight::new(false);
        f.launch(PILL, CARET, t0);
        f.sprite(settled);
        // settled past the dwell, so it dissolves at once
        let settled = settled + DWELL;
        f.sprite(settled);
        f.dissolve(settled);
        let mid = f.sprite(settled + motion::duration::EXIT / 2).unwrap();
        assert!(mid.radius > 1.0 && mid.alpha < 1.0);
        assert!(f.sprite(settled + motion::duration::EXIT + ms(1)).is_none());
        assert!(!f.is_active());

        let mut g = Flight::new(false);
        g.launch(PILL, CARET, t0);
        let mid_air = g.sprite(t0 + FLIGHT / 2).unwrap().at;
        g.fade(t0 + FLIGHT / 2);
        let s = g.sprite(t0 + FLIGHT / 2 + ms(1)).unwrap();
        assert_eq!((s.at, s.radius), (mid_air, 1.0), "fades where it is, without growing");
    }

    #[test]
    fn words_landing_mid_flight_still_fly_home_first() {
        let t0 = Instant::now();
        let mut f = Flight::new(false);
        f.launch(PILL, CARET, t0);
        f.dissolve(t0 + FLIGHT / 3);
        let s = f.sprite(t0 + FLIGHT / 2).unwrap();
        assert!(s.at != CARET && s.alpha == 1.0 && s.radius == 1.0, "keeps flying: {s:?}");
        let s = f.sprite(t0 + FLIGHT + DWELL + motion::duration::EXIT / 2).unwrap();
        assert!(s.at == CARET && s.radius > 1.0 && s.alpha < 1.0, "dissolves at the caret: {s:?}");
        assert!(f.sprite(t0 + FLIGHT + DWELL + motion::duration::EXIT + ms(1)).is_none());
    }

    #[test]
    fn words_landing_early_wait_out_the_dwell_at_the_caret() {
        let t0 = Instant::now();
        let landed = t0 + FLIGHT;
        let mut f = Flight::new(false);
        f.launch(PILL, CARET, t0);
        f.dissolve(t0 + FLIGHT / 3);
        let s = f.sprite(landed + DWELL / 2).unwrap();
        assert!(s.at == CARET && s.alpha > 0.99 && s.radius < 1.01, "a plain dot while it dwells: {s:?}");
        assert!(f.sprite(landed + DWELL + motion::duration::EXIT / 2).unwrap().radius > 1.0, "then dissolves");
        assert!(f.sprite(landed + DWELL + motion::duration::EXIT + ms(1)).is_none());

        // words just after landing wait out the rest of the dwell, not a whole one
        let mut g = Flight::new(false);
        g.launch(PILL, CARET, t0);
        g.sprite(landed + ms(1));
        g.dissolve(landed + DWELL / 2);
        assert!(g.sprite(landed + DWELL / 2 + ms(1)).unwrap().radius < 1.01);
        assert!(g.sprite(landed + DWELL + motion::duration::EXIT + ms(1)).is_none());
    }

    #[test]
    fn a_new_launch_while_leaving_starts_fresh_from_the_pill() {
        let t0 = Instant::now();
        let mut f = Flight::new(false);
        f.launch(PILL, CARET, t0);
        f.dissolve(t0 + FLIGHT + ms(10));
        let t1 = t0 + FLIGHT + ms(40);
        f.launch(PILL, (500.0, 200.0), t1);
        let s = f.sprite(t1).unwrap();
        assert!(close(s.at, PILL) && s.alpha == 1.0 && s.radius == 1.0, "{s:?}");
    }

    #[test]
    fn reduced_motion_has_no_flight() {
        let t0 = Instant::now();
        let mut f = Flight::new(true);
        f.launch(PILL, CARET, t0);
        assert_eq!(f.sprite(t0).unwrap().at, CARET, "a still dot at the caret");
        f.dissolve(t0 + ms(5));
        assert!(f.sprite(t0 + ms(6)).is_none());
    }

    #[test]
    fn mote_is_a_bright_core_in_a_soft_halo() {
        let px = render(&Sprite { at: (0.0, 0.0), alpha: 1.0, radius: 1.0, open: 0.0, words: 0.0 });
        let mid = (S / 2 * S + S / 2) as usize;
        assert!(px[mid] >> 24 > 0xE0, "core opaque: {:08X}", px[mid]);
        assert_eq!(px[0], 0, "corner clear");
        let dim = render(&Sprite { at: (0.0, 0.0), alpha: 0.5, radius: 1.0, open: 0.0, words: 0.0 });
        assert!(dim[mid] >> 24 < px[mid] >> 24);
        let big = render(&Sprite { at: (0.0, 0.0), alpha: 1.0, radius: 1.6, open: 0.0, words: 0.0 });
        let ring = (S / 2 * S + S / 2 + 9) as usize;
        assert!(big[ring] >> 24 > px[ring] >> 24, "dissolving spreads out");
    }

    #[test]
    fn only_a_fresh_caret_in_the_same_window_launches() {
        let caret = Some(RECT { left: 400, top: 290, right: 401, bottom: 310 });
        assert_eq!(landing_point(Some(7), 7, 7, caret), Some((400.0, 300.0)));
        assert_eq!(landing_point(None, 7, 7, caret), None, "after Done or Cancel");
        assert_eq!(landing_point(Some(8), 7, 7, caret), None, "an older dictation's lookup");
        assert_eq!(landing_point(Some(7), 7, 9, caret), None, "you switched windows");
        assert_eq!(landing_point(Some(7), 7, 7, None), None, "no caret");
    }

    #[test]
    fn done_picks_how_the_words_land() {
        assert_eq!(on_done(true, true, true), Landing::Dissolve);
        assert_eq!(on_done(true, true, false), Landing::Fade);
        assert_eq!(on_done(true, false, true), Landing::Fade);
        assert_eq!(on_done(false, true, true), Landing::Pulse);
        assert_eq!(on_done(false, true, false), Landing::Pulse);
        assert_eq!(on_done(false, false, true), Landing::Nothing);
    }

    #[test]
    fn nothing_to_leave_when_hidden() {
        let t0 = Instant::now();
        let mut f = Flight::new(false);
        f.dissolve(t0);
        f.fade(t0);
        assert!(f.sprite(t0).is_none() && !f.is_active());
    }

    fn msg(s: &str) -> Message {
        Message::plain(s)
    }

    #[test]
    fn hold_reads_at_least_locate_and_grows_with_length() {
        assert_eq!(hold_for(&msg("Didn't catch that")), motion::duration::LOCATE, "17 chars is under locate");
        let long = msg("Learned hob → HAWB · Copied, press Ctrl+V");
        assert_eq!(hold_for(&long), motion::duration::READ_PER_CHAR * long.chars() as u32);
        assert_eq!(msg("café").chars(), 4, "characters, not bytes");
    }

    #[test]
    fn say_flies_unfurls_holds_then_furls_away() {
        let t0 = Instant::now();
        let m = msg("Didn't catch that");
        let hold = hold_for(&m);
        let mut f = Flight::new(false);
        f.say(m, PILL, Target::Caret(CARET), t0);
        let s = f.sprite(t0).unwrap();
        assert!(close(s.at, PILL) && s.open == 0.0, "starts as a dot at the pill");
        let open_at = t0 + FLIGHT + motion::duration::ENTER + ms(1);
        let s = f.sprite(open_at).unwrap();
        assert_eq!(s.at, CARET);
        assert!(s.open > 0.99 && s.words > 0.99, "{s:?}");
        let furling = t0 + FLIGHT + motion::duration::ENTER + hold + motion::duration::EXIT / 2;
        let s = f.sprite(furling).unwrap();
        assert!(s.open < 1.0 && s.open > 0.0, "{s:?}");
        assert!(f.is_speaking());
        let gone = t0 + FLIGHT + motion::duration::ENTER + hold + motion::duration::EXIT * 2 + ms(2);
        assert!(f.sprite(gone).is_none());
        assert!(!f.is_speaking());
    }

    #[test]
    fn say_on_a_settled_mote_unfurls_where_it_is() {
        let t0 = Instant::now();
        let mut f = Flight::new(false);
        f.launch(PILL, CARET, t0);
        let settled = t0 + FLIGHT + ms(10);
        f.sprite(settled);
        f.say(msg("Didn't catch that"), PILL, Target::Pill((0.0, 0.0)), settled);
        let s = f.sprite(settled + motion::duration::ENTER + ms(1)).unwrap();
        assert_eq!(s.at, CARET, "stays at the caret, ignoring the new target");
        assert!(s.open > 0.99);
        assert!(!f.centred(), "a caret message grows rightward from the caret");
    }

    #[test]
    fn say_mid_flight_finishes_the_flight_first() {
        let t0 = Instant::now();
        let mut f = Flight::new(false);
        f.launch(PILL, CARET, t0);
        f.say(msg("Didn't catch that"), PILL, Target::Pill((5.0, 5.0)), t0 + ms(100));
        assert_eq!(f.sprite(t0 + FLIGHT + ms(1)).unwrap().at, CARET);
    }

    #[test]
    fn dismiss_furls_from_where_it_is() {
        let t0 = Instant::now();
        let mut f = Flight::new(false);
        f.launch(PILL, CARET, t0);
        let settled = t0 + FLIGHT + ms(10);
        f.sprite(settled);
        f.say(msg("Nothing to fix yet"), PILL, Target::Caret(CARET), settled);
        let mid = settled + motion::duration::ENTER / 2;
        let half = f.sprite(mid).unwrap().open;
        f.dismiss(mid);
        let after = f.sprite(mid + ms(10)).unwrap();
        assert!(after.open < half && after.open > 0.0, "{half} -> {}", after.open);
        assert!(f.sprite(mid + motion::duration::EXIT * 2 + ms(2)).is_none());
    }

    #[test]
    fn dismiss_after_the_message_has_run_its_course_leaves_no_ghost() {
        let t0 = Instant::now();
        let m = msg("Nothing to fix yet");
        let hold = hold_for(&m);
        let mut f = Flight::new(false);
        f.launch(PILL, CARET, t0);
        let settled = t0 + FLIGHT + ms(10);
        f.sprite(settled);
        f.say(m, PILL, Target::Caret(CARET), settled);
        // nothing polled the sprite since, so the phase is still Speaking
        f.dismiss(settled + motion::duration::ENTER + hold + motion::duration::EXIT * 2 + ms(5));
        assert!(f.sprite(settled + motion::duration::ENTER + hold + motion::duration::EXIT * 2 + ms(10)).is_none());
    }

    #[test]
    fn an_identical_frame_is_skipped_until_something_changes() {
        let s = Sprite { at: CARET, alpha: 1.0, radius: 1.0, open: 1.0, words: 1.0 };
        let mut last = None;
        assert!(changed(&mut last, &s, 10, 20, 100, 40), "first frame always draws");
        assert!(!changed(&mut last, &s, 10, 20, 100, 40), "the held frame repeats");
        assert!(changed(&mut last, &Sprite { open: 0.9, ..s }, 10, 20, 100, 40));
        assert!(changed(&mut last, &Sprite { open: 0.9, ..s }, 11, 20, 100, 40), "moved");
        last = None;
        assert!(changed(&mut last, &s, 11, 20, 100, 40), "hiding clears it");
    }

    #[test]
    fn reduced_motion_appears_open_and_still_holds() {
        let t0 = Instant::now();
        let m = msg("Didn't catch that");
        let hold = hold_for(&m);
        let mut f = Flight::new(true);
        f.say(m, PILL, Target::Pill(CARET), t0);
        let s = f.sprite(t0).unwrap();
        assert_eq!(s.at, CARET, "no flight");
        assert!(s.open == 1.0 && s.words == 1.0, "no stretch");
        assert!(f.centred(), "a pill message is centred");
        assert!(f.sprite(t0 + hold - ms(1)).is_some());
        assert!(f.sprite(t0 + hold + ms(1)).is_none());
    }

    #[test]
    fn a_sticky_message_holds_until_dismissed() {
        let t0 = Instant::now();
        let m = msg("Hold Right Ctrl and talk");
        let hold = hold_for(&m);
        let mut f = Flight::new(false);
        f.say_until_dismissed(m, PILL, Target::Pill(CARET), t0);
        let late = t0 + FLIGHT + motion::duration::ENTER + hold * 10;
        let s = f.sprite(late).unwrap();
        assert!(s.open == 1.0 && s.words == 1.0, "still open long after its read time: {s:?}");
        assert!(f.is_speaking());
        f.dismiss(late);
        assert!(f.sprite(late + motion::duration::EXIT * 2 + ms(5)).is_none(), "dismiss furls and dissolves it");
    }

    #[test]
    fn dismissing_a_sticky_message_furls_it_instead_of_popping() {
        let t0 = Instant::now();
        let m = msg("Hold Right Ctrl and talk");
        let late = t0 + FLIGHT + motion::duration::ENTER + hold_for(&m) * 10;
        let mut f = Flight::new(false);
        f.say_until_dismissed(m, PILL, Target::Pill(CARET), t0);
        // the loop draws every frame, so the flight has landed and opened by now
        f.sprite(late);
        f.dismiss(late);
        let s = f.sprite(late + ms(1)).expect("still furling");
        assert!(s.open > 0.5, "furls from fully open: {s:?}");
    }

    #[test]
    fn fading_a_sticky_message_fades_from_where_it_is() {
        let t0 = Instant::now();
        let m = msg("Hold Right Ctrl and talk");
        let late = t0 + FLIGHT + motion::duration::ENTER + hold_for(&m) * 10;
        let mut f = Flight::new(false);
        f.say_until_dismissed(m, PILL, Target::Pill(CARET), t0);
        f.sprite(late);
        f.fade(late);
        let s = f.sprite(late + ms(1)).expect("still fading");
        assert!(s.alpha > 0.5, "{s:?}");
    }

    #[test]
    fn a_message_replacing_one_above_the_pill_stays_centred() {
        let t0 = Instant::now();
        let mut f = Flight::new(false);
        f.say_until_dismissed(msg("Hold Right Ctrl and talk"), PILL, Target::Pill(CARET), t0);
        let t1 = t0 + FLIGHT + ms(300);
        f.sprite(t1);
        f.say(msg("Nothing to fix yet"), PILL, Target::Pill(CARET), t1);
        assert!(f.centred(), "keeps the pill's centred placement");
    }

    #[test]
    fn is_saying_names_the_message_on_screen() {
        let t0 = Instant::now();
        let mut f = Flight::new(false);
        let invite = msg("Hold Right Ctrl and talk");
        f.say_until_dismissed(invite.clone(), PILL, Target::Pill(CARET), t0);
        assert!(f.is_saying(&invite));
        f.say(msg("Nothing to fix yet"), PILL, Target::Pill(CARET), t0 + ms(10));
        assert!(!f.is_saying(&invite), "replaced");
    }

    #[test]
    fn a_message_after_a_sticky_one_times_out() {
        let t0 = Instant::now();
        let mut f = Flight::new(false);
        f.say_until_dismissed(msg("Hold Right Ctrl and talk"), PILL, Target::Pill(CARET), t0);
        let t1 = t0 + FLIGHT + ms(500);
        f.sprite(t1);
        let m = msg("Didn't catch that");
        let hold = hold_for(&m);
        f.say(m, PILL, Target::Pill(CARET), t1);
        assert!(f.sprite(t1 + motion::duration::ENTER + hold + motion::duration::EXIT * 2 + ms(10)).is_none());
    }

    #[test]
    fn fade_clears_sticky() {
        let t0 = Instant::now();
        let mut f = Flight::new(false);
        f.say_until_dismissed(msg("Hold Right Ctrl and talk"), PILL, Target::Pill(CARET), t0);
        f.sprite(t0 + FLIGHT + ms(300));
        f.fade(t0 + FLIGHT + ms(300));
        let t1 = t0 + FLIGHT + ms(300) + motion::duration::EXIT + ms(5);
        assert!(f.sprite(t1).is_none());
        let m = msg("Replaced");
        let hold = hold_for(&m);
        f.say(m, PILL, Target::Pill(CARET), t1);
        assert!(f.sprite(t1 + FLIGHT + motion::duration::ENTER + hold + motion::duration::EXIT * 2 + ms(10)).is_none(), "not sticky");
    }

    #[test]
    fn reduced_motion_sticky_appears_open_and_stays() {
        let t0 = Instant::now();
        let m = msg("Hold Right Ctrl and talk");
        let hold = hold_for(&m);
        let mut f = Flight::new(true);
        f.say_until_dismissed(m, PILL, Target::Pill(CARET), t0);
        let s = f.sprite(t0).unwrap();
        assert!(s.at == CARET && s.open == 1.0, "no flight, no stretch: {s:?}");
        assert!(f.sprite(t0 + hold * 10).is_some());
        f.dismiss(t0 + hold * 10);
        assert!(f.sprite(t0 + hold * 10 + ms(1)).is_none());
    }

    #[test]
    fn a_new_launch_replaces_a_message() {
        let t0 = Instant::now();
        let mut f = Flight::new(false);
        f.say(msg("Didn't catch that"), PILL, Target::Caret(CARET), t0);
        f.launch(PILL, CARET, t0 + ms(50));
        assert!(!f.is_speaking());
        assert!(f.message().is_none());
    }

    #[test]
    fn a_closed_tag_is_the_dot_and_an_open_one_sits_above_the_caret() {
        let at = (400.0, 300.0);
        let dot = tag_box(at, 0.0, (100, 18), 1.0, false);
        assert_eq!((dot.cx, dot.cy), at);
        assert_eq!((dot.w, dot.h), (DOT, DOT));
        let open = tag_box(at, 1.0, (100, 18), 1.0, false);
        assert!((open.cx - open.w / 2.0 - (at.0 - DOT / 2.0)).abs() < 1e-3, "left edge stays at the caret");
        assert_eq!(open.w, 100.0 + 2.0 * open.pad);
        assert!(open.cy + open.h / 2.0 < at.1, "entirely above the caret's middle: {open:?}");
        let centred = tag_box(at, 1.0, (100, 18), 1.0, true);
        assert_eq!(centred.cx, at.0);
        let big = tag_box(at, 1.0, (100, 18), 2.0, false);
        assert!(big.h > open.h && big.pad > open.pad, "scales with DPI");
    }

    #[test]
    fn fit_keeps_the_tag_on_screen() {
        let work = RECT { left: 0, top: 0, right: 1920, bottom: 1040 };
        assert_eq!(fit(1850, 500, 200, 40, work), (1720, 500), "pulled in from the right edge");
        assert_eq!(fit(-30, -10, 200, 40, work), (0, 0));
        let second = RECT { left: 1920, top: 0, right: 3840, bottom: 1040 };
        assert_eq!(fit(1900, 500, 200, 40, second).0, 1920);
    }
}

