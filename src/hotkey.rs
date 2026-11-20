use crossbeam_channel::Sender;
use std::sync::{Arc, Mutex};
use std::thread::{self, sleep, JoinHandle};
use std::time::{Duration, Instant};

// virtual-key codes
const VK_SHIFT: u16 = 0x10;
#[cfg(target_os = "macos")]
const VK_CONTROL: u16 = 0x11;
const VK_ESCAPE: u16 = 0x1B;
const VK_LSHIFT: u16 = 0xA0;
const VK_RSHIFT: u16 = 0xA1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEvent {
    Down,
    Press,
    Release,
    Cancel,
    FixLast,
    /// Second press of a double-tap: recording stays on after the key is let go.
    Latch,
    /// A Release with Control held during the hold: the words are an instruction for the selection.
    Edit,
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
    pub ctrl: bool,
}

#[derive(Debug, Clone, Copy)]
enum Phase {
    /// Key up. `last_tap` is when a too-short press was released, for double-tap detection.
    Idle { last_tap: Option<Instant> },
    Held { t0: Instant, shift: bool, ctrl: bool, sent_down: bool, sent_press: bool },
    /// Hands-free recording. `armed` once the latching press has been let go; the next press stops.
    Latched { since: Instant, armed: bool },
    /// Recording is over but the key is still down; wait for key-up so it does not start another.
    WaitUp,
}

/// Push-to-talk state machine, fed one key snapshot per poll.
/// Down fires immediately on key-down (unless Shift is held) so capture starts early;
/// Press is reported only after MIN_HOLD; a release before MIN_HOLD sends Cancel instead of Release.
/// Shift at any point during the hold means FixLast instead of a recording; Control means the
/// recording ends in Edit instead of Release.
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
                    Phase::Held { t0: now, shift: true, ctrl: false, sent_down: false, sent_press: false }
                } else if last_tap.is_some_and(|t| now.duration_since(t) <= DOUBLE_TAP) {
                    log::info!("hands-free latched");
                    out.extend([Down, Latch]);
                    Phase::Latched { since: now, armed: false }
                } else {
                    out.push(Down);
                    Phase::Held { t0: now, shift: false, ctrl: i.ctrl, sent_down: true, sent_press: false }
                }
            }
            p @ Phase::Idle { .. } => p,
            Phase::Held { t0, mut shift, ctrl, mut sent_down, mut sent_press } if i.ptt => {
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
                // Control too may land a poll late
                Phase::Held { t0, shift, ctrl: ctrl || i.ctrl, sent_down, sent_press }
            }
            Phase::Held { shift, ctrl, sent_press, .. } => {
                if shift {
                    out.push(FixLast);
                    Phase::Idle { last_tap: None }
                } else if sent_press {
                    out.push(if ctrl { Edit } else { Release });
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
                Phase::Held { t0: now, shift: true, ctrl: false, sent_down: false, sent_press: false }
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
    crate::platform::key_down(vk)
}

fn shift_down() -> bool {
    down(VK_SHIFT) || down(VK_LSHIFT) || down(VK_RSHIFT)
}

/// Mac only: on Windows, Right Alt is AltGr on many layouts and reports Control as held.
#[cfg(target_os = "macos")]
fn ctrl_down() -> bool {
    down(VK_CONTROL)
}
#[cfg(not(target_os = "macos"))]
fn ctrl_down() -> bool {
    false
}

/// Whether the PTT key is, or includes, a Control key.
fn is_control(vks: &[u16]) -> bool {
    vks.iter().any(|vk| matches!(vk, 0x11 | 0xA2 | 0xA3))
}

/// Whether the PTT key is held: every key of a chord is down. No keys is no PTT key, so the
/// thread isn't listening for one.
fn all_down(vks: &[u16], down: impl Fn(u16) -> bool) -> bool {
    !vks.is_empty() && vks.iter().all(|vk| down(*vk))
}

/// `ptt_vks` is read on every poll, so storing other keys (or none, to stop listening) takes
/// effect at once.
pub fn spawn(ptt_vks: Arc<Mutex<Vec<u16>>>, max: Duration, tx: Sender<HotkeyEvent>) -> JoinHandle<()> {
    thread::Builder::new()
        .name("hotkey".into())
        .spawn(move || {
            let mut m = Machine::new(max);
            loop {
                let (ptt, ctrl_key) = ptt_vks.lock().map(|vks| (all_down(&vks, down), is_control(&vks))).unwrap_or((false, false));
                // a Control push-to-talk key holds Control on every dictation, so it can't mean edit
                let input = Input { ptt, shift: shift_down(), esc: down(VK_ESCAPE), ctrl: !ctrl_key && ctrl_down() };
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
            .flat_map(|&(ms, ptt, shift, esc)| m.step(Input { ptt, shift, esc, ctrl: false }, t0 + Duration::from_millis(ms)))
            .collect()
    }

    #[test]
    fn a_control_ptt_key_never_means_edit() {
        assert!(is_control(&[0xA3]));
        assert!(is_control(&[0xA2, 0xA4]));
        assert!(!is_control(&[0xA5]));
    }

    #[test]
    fn control_during_the_hold_ends_in_edit() {
        let t0 = Instant::now();
        let mut m = Machine::new(MAX);
        let mut evs = vec![];
        // Control lands a poll after the key, as it often does
        for (ms, ptt, ctrl) in [(0, true, false), (30, true, true), (200, true, true), (400, false, false)] {
            evs.extend(m.step(Input { ptt, ctrl, ..Default::default() }, t0 + Duration::from_millis(ms)));
        }
        assert_eq!(evs, [Down, Press, Edit]);
    }

    #[test]
    fn a_chord_is_down_only_while_every_key_is() {
        let held = |keys: &'static [u16]| move |vk: u16| keys.contains(&vk);
        assert!(all_down(&[0xA2, 0xA4], held(&[0xA2, 0xA4, 0x10])));
        assert!(!all_down(&[0xA2, 0xA4], held(&[0xA2])));
        assert!(all_down(&[0xA3], held(&[0xA3])));
    }

    #[test]
    fn no_keys_set_is_never_down() {
        assert!(!all_down(&[], |_| true));
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
