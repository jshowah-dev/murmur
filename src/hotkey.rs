use crossbeam_channel::Sender;
use std::thread::{self, sleep, JoinHandle};
use std::time::{Duration, Instant};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_SHIFT};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEvent {
    Down,
    Press,
    Release,
    Cancel,
    FixLast,
}

const POLL: Duration = Duration::from_millis(30);
const MIN_HOLD: Duration = Duration::from_millis(150);

fn down(vk: u16) -> bool {
    unsafe { (GetAsyncKeyState(vk as i32) as u16 & 0x8000) != 0 }
}

/// Polls one key. Down fires immediately on key-down (unless Shift is held) so capture starts early;
/// Press is reported only after MIN_HOLD; a release before MIN_HOLD sends Cancel instead of Release.
/// Shift held at the moment of the press means FixLast instead of a recording.
pub fn spawn(ptt_vk: u16, tx: Sender<HotkeyEvent>) -> JoinHandle<()> {
    thread::Builder::new()
        .name("hotkey".into())
        .spawn(move || loop {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                while !down(ptt_vk) {
                    sleep(POLL);
                }
                let shift = down(VK_SHIFT.0);
                let t0 = Instant::now();
                let mut sent_press = false;
                if !shift {
                    let _ = tx.send(HotkeyEvent::Down);
                }
                while down(ptt_vk) {
                    if !shift && !sent_press && t0.elapsed() >= MIN_HOLD {
                        let _ = tx.send(HotkeyEvent::Press);
                        sent_press = true;
                    }
                    sleep(POLL);
                }
                if shift {
                    let _ = tx.send(HotkeyEvent::FixLast);
                } else if sent_press {
                    let _ = tx.send(HotkeyEvent::Release);
                } else {
                    let _ = tx.send(HotkeyEvent::Cancel);
                }
            }));
            if result.is_err() {
                log::error!("hotkey thread panicked; restarting");
            }
        })
        .expect("spawn hotkey thread")
}
