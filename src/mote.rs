//! The carved moment: when you let go of the key, a mote flies from the pill to your caret,
//! waits there while the speech is transcribed, and dissolves into your words.

use crate::canvas::{self, Canvas, Span};
use crate::editor_kit::{ease, reduced_motion};
use crate::motion;
use anyhow::{anyhow, Result};
use std::time::{Duration, Instant};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{GetLastError, ERROR_CLASS_ALREADY_EXISTS, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, RegisterClassW, SetWindowPos, ShowWindow, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SW_HIDE, SW_SHOWNOACTIVATE, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};

pub(crate) type Pt = (f32, f32);

/// The flight's length. The carved moment breaks the grammar here; logged in the kit ledger.
pub(crate) const FLIGHT: Duration = Duration::from_millis(300);

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
    Leaving { at: Pt, start: Instant, alpha: f32, grow: bool },
}

pub(crate) struct Flight {
    phase: Phase,
    reduced: bool,
    message: Option<Message>,
    centred: bool,
}

impl Flight {
    pub(crate) fn new(reduced: bool) -> Flight {
        Flight { phase: Phase::Hidden, reduced, message: None, centred: false }
    }

    /// Starts a flight from the pill to the caret, replacing whatever was showing.
    pub(crate) fn launch(&mut self, from: Pt, to: Pt, now: Instant) {
        self.message = None;
        self.phase = if self.reduced { Phase::Settled { at: to, since: now } } else { Phase::Flying { from, to, start: now, then: Then::Settle } };
    }

    /// The words landed: grow and fade into them. Mid-flight, it finishes the flight first.
    pub(crate) fn dissolve(&mut self, now: Instant) {
        if let Phase::Flying { then, .. } = &mut self.phase {
            *then = Then::Dissolve;
            return;
        }
        self.leave(now, true);
    }

    /// Says `m`: a mote already out says it where it is (finishing a flight first); otherwise
    /// one flies from `from` to the target and says it there.
    pub(crate) fn say(&mut self, m: Message, from: Pt, to: Target, now: Instant) {
        self.message = Some(m);
        self.phase = match self.phase {
            Phase::Flying { from, to, start, then } => {
                // a dictation's flight ends at a caret; an earlier message keeps its own placement
                if then != Then::Say {
                    self.centred = false;
                }
                Phase::Flying { from, to, start, then: Then::Say }
            }
            Phase::Settled { at, .. } | Phase::Speaking { at, .. } => {
                self.centred = false;
                Phase::Speaking { at, start: now }
            }
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

    /// Closes an open message early, from however far it has opened.
    pub(crate) fn dismiss(&mut self, now: Instant) {
        match self.phase {
            Phase::Speaking { at, .. } => {
                let open = self.sprite(now).map_or(0.0, |s| s.open);
                self.phase = if self.reduced { Phase::Hidden } else { Phase::Furling { at, start: now, open } };
            }
            Phase::Flying { then: Then::Say, .. } => self.leave(now, false),
            _ => {}
        }
    }

    /// Whether a message is on its way, open, or closing.
    pub(crate) fn is_speaking(&self) -> bool {
        self.message.is_some() && matches!(self.phase, Phase::Flying { then: Then::Say, .. } | Phase::Speaking { .. } | Phase::Furling { .. })
    }

    pub(crate) fn message(&self) -> Option<&Message> {
        self.message.as_ref()
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
            self.phase = Phase::Hidden;
            return;
        }
        if let Some(s) = self.sprite(now) {
            self.phase = Phase::Leaving { at: s.at, start: now, alpha: s.alpha, grow };
        }
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
                        Then::Dissolve => Phase::Leaving { at: to, start: landed, alpha: 1.0, grow: true },
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
                if el >= unfurl + hold {
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

fn render(s: &Sprite) -> Vec<u32> {
    let mut c = Canvas::new(S, S);
    let m = S as f32 / 2.0;
    c.halo(m, m, 12.0 * s.radius, 0x60D060, 0.5 * s.alpha);
    let core = 5.0 * s.radius;
    c.capsule(m - core / 2.0, m - core / 2.0, core, core, 0xE8FFE8, s.alpha);
    c.into_bgra()
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

/// The mote's own click-through window. It's created and moved in physical pixels, the space
/// `caret::find` reports in, while the rest of Murmur is DPI-unaware.
pub(crate) struct Mote {
    hwnd: HWND,
    flight: Flight,
    shown: bool,
}

impl Mote {
    pub(crate) fn create() -> Result<Mote> {
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
        Ok(Mote { hwnd, flight: Flight::new(reduced_motion()), shown: false })
    }

    pub(crate) fn launch(&mut self, from: Pt, to: Pt) {
        self.flight.launch(from, to, Instant::now());
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

    /// Draws this frame of the flight, or hides the window once it's over.
    pub(crate) fn animate(&mut self) {
        match self.flight.sprite(Instant::now()) {
            Some(s) => {
                let px = render(&s);
                let (x, y) = ((s.at.0 - S as f32 / 2.0).round() as i32, (s.at.1 - S as f32 / 2.0).round() as i32);
                crate::caret::physical(|| canvas::push(self.hwnd, x, y, S, S, &px));
                if !self.shown {
                    unsafe {
                        let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOSIZE | SWP_NOMOVE | SWP_NOACTIVATE);
                        let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                    }
                    self.shown = true;
                }
            }
            None if self.shown => {
                unsafe {
                    let _ = ShowWindow(self.hwnd, SW_HIDE);
                }
                self.shown = false;
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use windows::Win32::Foundation::RECT;

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
        let s = f.sprite(t0 + FLIGHT + motion::duration::EXIT / 2).unwrap();
        assert!(s.at == CARET && s.radius > 1.0 && s.alpha < 1.0, "dissolves at the caret: {s:?}");
        assert!(f.sprite(t0 + FLIGHT + motion::duration::EXIT + ms(1)).is_none());
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
    fn a_new_launch_replaces_a_message() {
        let t0 = Instant::now();
        let mut f = Flight::new(false);
        f.say(msg("Didn't catch that"), PILL, Target::Caret(CARET), t0);
        f.launch(PILL, CARET, t0 + ms(50));
        assert!(!f.is_speaking());
        assert!(f.message().is_none());
    }
}
