use crossbeam_channel::Sender;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::thread::{self, sleep, JoinHandle};
use std::time::{Duration, Instant};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_ESCAPE, VK_LSHIFT, VK_RSHIFT, VK_SHIFT};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEvent {
    Down,
    Press,
    Release,
    Cancel,
    FixLast,
    /// Second press of a double-tap: recording stays on after the key is let go.
    Latch,
}

const POLL: Duration = Duration::from_millis(30);
const MIN_HOLD: Duration = Duration::from_millis(150);
/// Max gap between a short tap's release and the next press for the pair to count as a double-tap.
const DOUBLE_TAP: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, Copy, Default)]
pub struct Input {
    pub ptt: bool,
    pub shift: bool,
    pub esc: bool,
}

#[derive(Debug, Clone, Copy)]
enum Phase {
    /// Key up. `last_tap` is when a too-short press was released, for double-tap detection.
    Idle { last_tap: Option<Instant> },
    Held { t0: Instant, shift: bool, sent_down: bool, sent_press: bool },
    /// Hands-free recording. `armed` once the latching press has been let go; the next press stops.
    Latched { since: Instant, armed: bool },
    /// Recording is over but the key is still down; wait for key-up so it does not start another.
    WaitUp,
}

/// Push-to-talk state machine, fed one key snapshot per poll.
/// Down fires immediately on key-down (unless Shift is held) so capture starts early;
/// Press is reported only after MIN_HOLD; a release before MIN_HOLD sends Cancel instead of Release.
/// Shift at any point during the hold means FixLast instead of a recording.
/// A second press within DOUBLE_TAP of a cancelled tap sends Down + Latch; while latched, a press
/// sends Release, Esc sends Cancel, and `max` after latching sends Release.
pub struct Machine {
    phase: Phase,
    max: Duration,
}

impl Machine {
    pub fn new(max: Duration) -> Self {
        Machine { phase: Phase::Idle { last_tap: None }, max }
    }

    /// True while a recording may be in flight, so a crash must be compensated with Cancel.
    fn recording(&self) -> bool {
        matches!(self.phase, Phase::Held { sent_down: true, .. } | Phase::Latched { .. })
    }

    pub fn step(&mut self, i: Input, now: Instant) -> Vec<HotkeyEvent> {
        use HotkeyEvent::*;
        let mut out = vec![];
        self.phase = match self.phase {
            Phase::Idle { last_tap } if i.ptt => {
                log::info!("ptt down, shift={}", i.shift);
                if i.shift {
                    Phase::Held { t0: now, shift: true, sent_down: false, sent_press: false }
                } else if last_tap.is_some_and(|t| now.duration_since(t) <= DOUBLE_TAP) {
                    log::info!("hands-free latched");
                    out.extend([Down, Latch]);
                    Phase::Latched { since: now, armed: false }
                } else {
                    out.push(Down);
                    Phase::Held { t0: now, shift: false, sent_down: true, sent_press: false }
                }
            }
            p @ Phase::Idle { .. } => p,
            Phase::Held { t0, mut shift, mut sent_down, mut sent_press } if i.ptt => {
                // Shift is often reported a poll or two late; any Shift during the hold
                // turns the press into fix-last and abandons the capture.
                if !shift && i.shift {
                    shift = true;
                    log::info!("shift seen during hold");
                    if sent_down {
                        out.push(Cancel);
                        sent_down = false;
                        sent_press = false;
                    }
                }
                if !shift && !sent_press && now.duration_since(t0) >= MIN_HOLD {
                    out.push(Press);
                    sent_press = true;
                }
                Phase::Held { t0, shift, sent_down, sent_press }
            }
            Phase::Held { shift, sent_press, .. } => {
                if shift {
                    out.push(FixLast);
                    Phase::Idle { last_tap: None }
                } else if sent_press {
                    out.push(Release);
                    Phase::Idle { last_tap: None }
                } else {
                    out.push(Cancel);
                    Phase::Idle { last_tap: Some(now) }
                }
            }
            Phase::Latched { .. } if i.esc => {
                log::info!("hands-free cancelled (Esc)");
                out.push(Cancel);
                Phase::WaitUp
            }
            Phase::Latched { since, .. } if now.duration_since(since) >= self.max => {
                log::info!("hands-free hit the length cap");
                out.push(Release);
                Phase::WaitUp
            }
            Phase::Latched { armed: false, .. } if i.ptt && i.shift => {
                // Shift on the latching press still means fix-last.
                out.push(Cancel);
                Phase::Held { t0: now, shift: true, sent_down: false, sent_press: false }
            }
            Phase::Latched { since, armed: false } => Phase::Latched { since, armed: !i.ptt },
            Phase::Latched { .. } if i.ptt => {
                log::info!("hands-free stopped");
                out.push(Release);
                Phase::WaitUp
            }
            p @ Phase::Latched { .. } => p,
            Phase::WaitUp if i.ptt => Phase::WaitUp,
            Phase::WaitUp => Phase::Idle { last_tap: None },
        };
        out
    }
}

pub(crate) fn down(vk: u16) -> bool {
    unsafe { (GetAsyncKeyState(vk as i32) as u16 & 0x8000) != 0 }
}

fn shift_down() -> bool {
    down(VK_SHIFT.0) || down(VK_LSHIFT.0) || down(VK_RSHIFT.0)
}

/// Whether the PTT key is held. A key of 0 is no key: the thread isn't listening for one.
fn ptt_down(ptt_vk: &AtomicU16) -> bool {
    match ptt_vk.load(Ordering::Relaxed) {
        0 => false,
        vk => down(vk),
    }
}

/// `ptt_vk` is read on every poll, so storing another key (or 0, to stop listening) takes effect
/// at once.
pub fn spawn(ptt_vk: Arc<AtomicU16>, max: Duration, tx: Sender<HotkeyEvent>) -> JoinHandle<()> {
    thread::Builder::new()
        .name("hotkey".into())
        .spawn(move || {
            let mut m = Machine::new(max);
            loop {
                let input = Input { ptt: ptt_down(&ptt_vk), shift: shift_down(), esc: down(VK_ESCAPE.0) };
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| m.step(input, Instant::now()))) {
                    Ok(evs) => {
                        for ev in evs {
                            let _ = tx.send(ev);
                        }
                    }
                    Err(_) => {
                        log::error!("hotkey step panicked; sending compensating event");
                        if m.recording() {
                            let _ = tx.send(HotkeyEvent::Cancel);
                        }
                        m = Machine::new(max);
                    }
                }
                sleep(POLL);
            }
        })
        .expect("spawn hotkey thread")
}

#[cfg(test)]
mod tests {
    use super::*;
    use HotkeyEvent::*;

    const MAX: Duration = Duration::from_secs(300);

    /// Drives the machine with (ms since start, ptt, shift, esc) snapshots, collecting all events.
    fn run(steps: &[(u64, bool, bool, bool)]) -> Vec<HotkeyEvent> {
        let t0 = Instant::now();
        let mut m = Machine::new(MAX);
        steps
            .iter()
            .flat_map(|&(ms, ptt, shift, esc)| m.step(Input { ptt, shift, esc }, t0 + Duration::from_millis(ms)))
            .collect()
    }

    #[test]
    fn no_key_set_is_never_down() {
        assert!(!ptt_down(&AtomicU16::new(0)));
    }

    #[test]
    fn short_tap_cancels() {
        assert_eq!(run(&[(0, true, false, false), (60, false, false, false)]), vec![Down, Cancel]);
    }

    #[test]
    fn hold_then_release_dictates() {
        assert_eq!(
            run(&[(0, true, false, false), (90, true, false, false), (180, true, false, false), (600, false, false, false)]),
            vec![Down, Press, Release]
        );
    }

    #[test]
    fn shift_at_press_is_fix_last() {
        assert_eq!(run(&[(0, true, true, false), (200, true, true, false), (300, false, false, false)]), vec![FixLast]);
    }

    #[test]
    fn late_shift_cancels_capture_then_fix_last() {
        assert_eq!(
            run(&[(0, true, false, false), (60, true, true, false), (300, false, false, false)]),
            vec![Down, Cancel, FixLast]
        );
    }

    #[test]
    fn double_tap_latches_and_tap_stops() {
        assert_eq!(
            run(&[
                (0, true, false, false),
                (60, false, false, false),
                (200, true, false, false), // second press within 300 ms
                (260, false, false, false),
                (5_000, false, false, false),
                (9_000, true, false, false), // stop tap
                (9_060, false, false, false),
            ]),
            vec![Down, Cancel, Down, Latch, Release]
        );
    }

    #[test]
    fn latching_press_can_be_held_before_letting_go() {
        assert_eq!(
            run(&[
                (0, true, false, false),
                (60, false, false, false),
                (200, true, false, false),
                (2_000, true, false, false), // still holding: no Release
                (2_100, false, false, false),
            ]),
            vec![Down, Cancel, Down, Latch]
        );
    }

    #[test]
    fn slow_second_tap_is_a_new_press() {
        assert_eq!(
            run(&[(0, true, false, false), (60, false, false, false), (500, true, false, false), (560, false, false, false)]),
            vec![Down, Cancel, Down, Cancel]
        );
    }

    #[test]
    fn after_a_dictation_a_quick_press_does_not_latch() {
        assert_eq!(
            run(&[(0, true, false, false), (200, true, false, false), (400, false, false, false), (500, true, false, false), (560, false, false, false)]),
            vec![Down, Press, Release, Down, Cancel]
        );
    }

    #[test]
    fn esc_cancels_latched() {
        assert_eq!(
            run(&[(0, true, false, false), (60, false, false, false), (200, true, false, false), (260, false, false, false), (3_000, false, false, true)]),
            vec![Down, Cancel, Down, Latch, Cancel]
        );
    }

    #[test]
    fn esc_does_nothing_outside_hands_free() {
        assert_eq!(
            run(&[(0, true, false, false), (200, true, false, true), (400, false, false, false)]),
            vec![Down, Press, Release]
        );
    }

    #[test]
    fn cap_stops_latched() {
        assert_eq!(
            run(&[(0, true, false, false), (60, false, false, false), (200, true, false, false), (260, false, false, false), (200 + 300_000, false, false, false)]),
            vec![Down, Cancel, Down, Latch, Release]
        );
    }

    #[test]
    fn stop_press_held_does_not_start_another_recording() {
        assert_eq!(
            run(&[
                (0, true, false, false),
                (60, false, false, false),
                (200, true, false, false),
                (260, false, false, false),
                (3_000, true, false, false),
                (3_500, true, false, false),
                (3_600, false, false, false),
                (3_700, false, false, false),
            ]),
            vec![Down, Cancel, Down, Latch, Release]
        );
    }

    #[test]
    fn shift_on_latching_press_is_fix_last() {
        assert_eq!(
            run(&[(0, true, false, false), (60, false, false, false), (200, true, false, false), (230, true, true, false), (300, false, false, false)]),
            vec![Down, Cancel, Down, Latch, Cancel, FixLast]
        );
    }
}
