//! The carved moment: when you let go of the key, a mote flies from the pill to your caret,
//! waits there while the speech is transcribed, and dissolves into your words.

use crate::editor_kit::ease;
use crate::motion;
use std::time::{Duration, Instant};

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

/// What to draw this frame: where, how opaque, and how large (1 = normal).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Sprite {
    pub at: Pt,
    pub alpha: f32,
    pub radius: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    Hidden,
    Flying { from: Pt, to: Pt, start: Instant },
    Settled { at: Pt, since: Instant },
    Leaving { at: Pt, start: Instant, alpha: f32, grow: bool },
}

pub(crate) struct Flight {
    phase: Phase,
    reduced: bool,
}

impl Flight {
    pub(crate) fn new(reduced: bool) -> Flight {
        Flight { phase: Phase::Hidden, reduced }
    }

    /// Starts a flight from the pill to the caret, replacing whatever was showing.
    pub(crate) fn launch(&mut self, from: Pt, to: Pt, now: Instant) {
        self.phase = if self.reduced { Phase::Settled { at: to, since: now } } else { Phase::Flying { from, to, start: now } };
    }

    /// The words landed: grow and fade into them.
    pub(crate) fn dissolve(&mut self, now: Instant) {
        self.leave(now, true);
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
            Phase::Flying { from, to, start } => {
                let t = secs(start, FLIGHT);
                if t >= 1.0 {
                    self.phase = Phase::Settled { at: to, since: start + motion::scaled(FLIGHT) };
                    return self.sprite(now);
                }
                Some(Sprite { at: arc_point(from, to, ease(motion::easing::ENTER, t)), alpha: 1.0, radius: 1.0 })
            }
            Phase::Settled { at, since } => {
                // breathes slowly while the words are on their way
                let breath = (secs(since, motion::duration::LOCATE) * std::f32::consts::TAU).cos();
                let alpha = if self.reduced { 1.0 } else { 0.85 + 0.15 * breath };
                Some(Sprite { at, alpha, radius: 1.0 })
            }
            Phase::Leaving { at, start, alpha, grow } => {
                let t = secs(start, motion::duration::EXIT);
                if t >= 1.0 {
                    self.phase = Phase::Hidden;
                    return None;
                }
                let e = ease(motion::easing::EXIT, t);
                Some(Sprite { at, alpha: alpha * (1.0 - e), radius: if grow { 1.0 + 0.6 * e } else { 1.0 } })
            }
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
    fn nothing_to_leave_when_hidden() {
        let t0 = Instant::now();
        let mut f = Flight::new(false);
        f.dissolve(t0);
        f.fade(t0);
        assert!(f.sprite(t0).is_none() && !f.is_active());
    }
}
