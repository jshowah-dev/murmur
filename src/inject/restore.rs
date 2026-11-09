//! Putting the clipboard back after a paste. The restore waits long enough for a slow app to read
//! the clipboard (since macOS 27.2 some read it well over 150 ms after the keystroke, and an
//! Electron app busy building its accessibility tree for the caret lookup takes longer still), and is
//! skipped if anything else wrote to the clipboard in the meantime.

use anyhow::Result;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::thread::sleep;
use std::time::Duration;

/// How long after the paste keystroke the old clipboard goes back. 600 ms was too short for
/// the Claude app right after Murmur started.
pub(super) const RESTORE_AFTER: Duration = Duration::from_secs(2);

/// The system clipboard, as much of it as a paste needs.
pub(super) trait Board: Sync + 'static {
    /// Everything on the clipboard, in every format, so an image or copied files come back too.
    type Saved: Send + 'static;
    /// `Ok(None)` for an empty clipboard; an error when it couldn't be read, so a paste doesn't
    /// go on to overwrite what it couldn't save.
    fn save(&self) -> Result<Option<Self::Saved>>;
    fn put_back(&self, saved: &Self::Saved) -> Result<()>;
    fn write(&self, text: &str) -> Result<()>;
    /// Goes up on every write to the clipboard, by any app.
    fn count(&self) -> i64;
}

struct Pending<S> {
    id: u64,
    original: Option<S>,
    count: i64,
}

/// Our text is on the clipboard; `original` goes back once the paste has been read.
pub(super) struct Swapped<S> {
    original: Option<S>,
    count: i64,
}

pub(super) struct Restorer<B: Board> {
    board: B,
    delay: Duration,
    // held while the clipboard is written, so a restore and the next paste can't interleave
    pending: Mutex<Option<Pending<B::Saved>>>,
    next_id: AtomicU64,
}

impl<B: Board> Restorer<B> {
    pub(super) const fn new(board: B, delay: Duration) -> Self {
        Restorer { board, delay, pending: Mutex::new(None), next_id: AtomicU64::new(0) }
    }

    /// Puts `text` on the clipboard. When a restore is still waiting and nothing else has
    /// written to the clipboard since, its original is the one kept, not the previous
    /// dictation's text.
    pub(super) fn put(&self, text: &str) -> Result<Swapped<B::Saved>> {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        let original = match pending.take() {
            Some(p) if self.board.count() == p.count => p.original,
            _ => self.board.save()?,
        };
        if let Err(e) = self.board.write(text) {
            if let Some(o) = &original {
                let _ = self.board.put_back(o);
            }
            return Err(e);
        }
        Ok(Swapped { original, count: self.board.count() })
    }

    /// Puts the original back after the delay, unless the clipboard changed since `put` or
    /// another paste took the restore over.
    pub(super) fn restore_later(&'static self, s: Swapped<B::Saved>) {
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
            if let Err(e) = self.board.put_back(&o) {
                log::warn!("couldn't put the clipboard back: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A clipboard holding one item, in any number of formats.
    #[derive(Default)]
    struct Fake(Mutex<(Vec<(String, String)>, i64)>);

    impl Fake {
        fn read(&self) -> Option<String> {
            self.0.lock().unwrap().0.iter().find(|(f, _)| f == "text").map(|(_, v)| v.clone())
        }
        fn set(&self, item: Vec<(String, String)>) {
            let mut b = self.0.lock().unwrap();
            *b = (item, b.1 + 1);
        }
    }

    impl Board for Fake {
        type Saved = Vec<(String, String)>;
        fn save(&self) -> Result<Option<Self::Saved>> {
            let b = self.0.lock().unwrap();
            Ok((!b.0.is_empty()).then(|| b.0.clone()))
        }
        fn put_back(&self, saved: &Self::Saved) -> Result<()> {
            self.set(saved.clone());
            Ok(())
        }
        fn write(&self, text: &str) -> Result<()> {
            self.set(vec![("text".into(), text.into())]);
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

    #[test]
    fn an_image_comes_back_too() {
        let r = restorer("unused");
        r.board.set(vec![("png".into(), "pixels".into())]);
        let s = r.put("dictated").unwrap();
        r.restore_later(s);
        wait();
        assert_eq!(r.board.save().unwrap(), Some(vec![("png".into(), "pixels".into())]));
    }

    #[test]
    fn a_copy_between_two_pastes_is_the_one_restored() {
        let r = restorer("old");
        let s = r.put("one").unwrap();
        r.restore_later(s);
        r.board.write("copied").unwrap();
        let s = r.put("two").unwrap();
        r.restore_later(s);
        wait();
        assert_eq!(r.board.read().as_deref(), Some("copied"));
    }
}
