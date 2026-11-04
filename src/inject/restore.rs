//! Putting the clipboard back after a paste. The restore waits long enough for a slow app to read
//! the clipboard (since macOS 27.2 some read it well over 150 ms after the keystroke), and is
//! skipped if anything else wrote to the clipboard in the meantime.

use anyhow::Result;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::thread::sleep;
use std::time::Duration;

/// How long after the paste keystroke the old clipboard goes back.
pub(super) const RESTORE_AFTER: Duration = Duration::from_millis(600);

/// The system clipboard, as much of it as a paste needs.
pub(super) trait Board: Sync + 'static {
    fn read(&self) -> Option<String>;
    fn write(&self, text: &str) -> Result<()>;
    /// Goes up on every write to the clipboard, by any app.
    fn count(&self) -> i64;
}

struct Pending {
    id: u64,
    original: Option<String>,
    count: i64,
}

/// Our text is on the clipboard; `original` goes back once the paste has been read.
pub(super) struct Swapped {
    original: Option<String>,
    count: i64,
}

pub(super) struct Restorer<B> {
    board: B,
    delay: Duration,
    // held while the clipboard is written, so a restore and the next paste can't interleave
    pending: Mutex<Option<Pending>>,
    next_id: AtomicU64,
}

impl<B: Board> Restorer<B> {
    pub(super) const fn new(board: B, delay: Duration) -> Self {
        Restorer { board, delay, pending: Mutex::new(None), next_id: AtomicU64::new(0) }
    }

    /// Puts `text` on the clipboard. When a restore is still waiting, its original is the one kept,
    /// not the previous dictation's text.
    pub(super) fn put(&self, text: &str) -> Result<Swapped> {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        let original = match pending.take() {
            Some(p) => p.original,
            None => self.board.read(),
        };
        if let Err(e) = self.board.write(text) {
            if let Some(o) = &original {
                let _ = self.board.write(o);
            }
            return Err(e);
        }
        Ok(Swapped { original, count: self.board.count() })
    }

    /// Puts the original back after the delay, unless the clipboard changed since `put` or
    /// another paste took the restore over.
    pub(super) fn restore_later(&'static self, s: Swapped) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        *self.pending.lock().unwrap_or_else(|e| e.into_inner()) = Some(Pending { id, original: s.original, count: s.count });
        std::thread::spawn(move || {
            sleep(self.delay);
            self.restore_now(id);
        });
    }

    fn restore_now(&self, id: u64) {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        let Some(p) = pending.take_if(|p| p.id == id) else { return };
        if self.board.count() != p.count {
            log::info!("clipboard changed since the paste, so it's left as it is");
            return;
        }
        if let Some(o) = p.original {
            let _ = self.board.write(&o);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Fake(Mutex<(Option<String>, i64)>);

    impl Board for Fake {
        fn read(&self) -> Option<String> {
            self.0.lock().unwrap().0.clone()
        }
        fn write(&self, text: &str) -> Result<()> {
            let mut b = self.0.lock().unwrap();
            *b = (Some(text.to_string()), b.1 + 1);
            Ok(())
        }
        fn count(&self) -> i64 {
            self.0.lock().unwrap().1
        }
    }

    fn restorer(on_board: &str) -> &'static Restorer<Fake> {
        let r = Box::leak(Box::new(Restorer::new(Fake::default(), Duration::from_millis(50))));
        r.board.write(on_board).unwrap();
        r
    }

    fn wait() {
        sleep(Duration::from_millis(300));
    }

    #[test]
    fn the_original_comes_back_only_after_the_delay() {
        let r = restorer("old");
        let s = r.put("dictated").unwrap();
        r.restore_later(s);
        assert_eq!(r.board.read().as_deref(), Some("dictated"));
        wait();
        assert_eq!(r.board.read().as_deref(), Some("old"));
    }

    #[test]
    fn a_copy_made_during_the_wait_is_kept() {
        let r = restorer("old");
        let s = r.put("dictated").unwrap();
        r.restore_later(s);
        r.board.write("copied").unwrap();
        wait();
        assert_eq!(r.board.read().as_deref(), Some("copied"));
    }

    #[test]
    fn back_to_back_pastes_restore_the_first_original() {
        let r = restorer("old");
        let s = r.put("one").unwrap();
        r.restore_later(s);
        let s = r.put("two").unwrap();
        r.restore_later(s);
        assert_eq!(r.board.read().as_deref(), Some("two"));
        wait();
        assert_eq!(r.board.read().as_deref(), Some("old"));
    }
}
