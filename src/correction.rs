use crate::correction_ui::{self, rgb, AMBER, MUTED, SPOKEN, TEXT};
use crate::dictionary::Dictionary;
use crate::history::{History, InjectMethod};
use crate::inject;
use crate::mote::Message;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The paste shortcut, as the fix-last message names it.
#[cfg(windows)]
const PASTE: &str = "Ctrl+V";
#[cfg(target_os = "macos")]
const PASTE: &str = "⌘V";

/// What fix-last did, for the mote to say at the caret.
#[derive(Debug, Default, PartialEq)]
pub struct FixOutcome {
    pub nothing_to_fix: bool,
    /// (spoken, written) for each term learned
    pub learned: Vec<(String, String)>,
    /// the correction went to the clipboard for you to paste
    pub copied: bool,
    /// the correction replaced the dictation in place
    pub replaced: bool,
    /// dictionary.toml couldn't be read, so nothing was learned
    pub not_learned: Option<crate::notice::FileProblem>,
    /// the card's centre when it closed, physical pixels
    pub card: Option<(f32, f32)>,
}

/// Terms named in a "Learned" message; any more are counted.
const NAMED: usize = 2;

/// Longest a term can run in a message, ellipsis included, so the capsule stays on screen.
const TERM_MAX: usize = 24;

fn shorten(term: &str) -> String {
    if term.chars().count() <= TERM_MAX {
        return term.to_string();
    }
    term.chars().take(TERM_MAX - 1).chain(['…']).collect()
}

/// What the mote says after fix-last, or None when nothing was changed.
pub(crate) fn fix_message(o: &FixOutcome) -> Option<Message> {
    if o.nothing_to_fix {
        return Some(Message::plain("Nothing to fix yet"));
    }
    let (text, muted) = (rgb(TEXT), rgb(MUTED));
    let mut spans = Vec::new();
    if let Some(p) = &o.not_learned {
        spans.push((crate::notice::not_learned(p), text));
    }
    if !o.learned.is_empty() {
        spans.push(("Learned ".to_string(), text));
        for (i, (spoken, written)) in o.learned.iter().take(NAMED).enumerate() {
            if i > 0 {
                spans.push((", ".into(), muted));
            }
            spans.push((shorten(spoken), rgb(SPOKEN)));
            spans.push((" → ".into(), muted));
            spans.push((shorten(written), rgb(AMBER)));
        }
        if o.learned.len() > NAMED {
            spans.push((format!(" +{} more", o.learned.len() - NAMED), muted));
        }
    }
    match (spans.is_empty(), o.copied) {
        (true, false) if o.replaced => spans.push(("Replaced".into(), text)),
        (true, false) => return None,
        (true, true) => spans.push((format!("Copied, press {PASTE} to paste"), text)),
        (false, true) => spans.push((format!(" · Copied, press {PASTE}"), text)),
        // "Learned …" already says the edit landed; "Not learned" doesn't
        (false, false) if o.replaced && o.learned.is_empty() => spans.push((" · Replaced".into(), text)),
        (false, false) => {}
    }
    Some(Message(spans))
}

pub fn fix_last(history: &mut History, dict: &Arc<Mutex<Dictionary>>) -> FixOutcome {
    let Some(last) = history.last().cloned() else {
        return FixOutcome { nothing_to_fix: true, ..Default::default() };
    };
    let target_hwnd = inject::foreground_hwnd();
    let snapshot = dict.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let heard_at = last.inject.as_ref().map(|r| r.at);
    let (edited, card) = correction_ui::show(&last.cleaned, heard_at, snapshot, target_hwnd);
    let mut out = FixOutcome { card, ..Default::default() };
    let Some(edited) = edited else { return out };
    let edited = edited.trim().to_string();
    if edited == last.cleaned {
        return out;
    }
    {
        let mut d = dict.lock().unwrap_or_else(|e| e.into_inner());
        match Dictionary::load_or_seed() {
            Ok(fresh) => {
                *d = fresh;
                let l = d.learn(&last.cleaned, &edited);
                if !l.is_empty() {
                    if let Err(e) = d.save() {
                        log::error!("save dictionary: {e}");
                    }
                }
                out.learned = l.iter().map(|t| (t.spoken.last().cloned().unwrap_or_default(), t.written.clone())).collect();
            }
            Err(e) => {
                log::error!("reload dictionary: {e:#}");
                out.not_learned = Some(crate::notice::file_problem(&crate::dictionary::path(), &e, crate::notice::Effect::CorrectionsOff));
            }
        }
    }
    let replaced = paste_or_copy_correction(history, &last, target_hwnd, edited);
    out.replaced = replaced;
    out.copied = !replaced;
    out
}

/// Replaces the last dictation in place when it's fresh, else puts the correction on the
/// clipboard. Returns whether it replaced.
fn paste_or_copy_correction(history: &mut History, last: &crate::history::Entry, target_hwnd: isize, edited: String) -> bool {
    let fresh = last
        .inject
        .as_ref()
        .map(|r| r.hwnd == target_hwnd && r.at.elapsed() < Duration::from_secs(60) && r.method == InjectMethod::Paste)
        .unwrap_or(false);
    if fresh {
        // the dialog took focus; hand it back to the target before undo+paste
        crate::platform::focus_target(target_hwnd);
        std::thread::sleep(Duration::from_millis(150));
        if inject::foreground_hwnd() == target_hwnd {
            match inject::undo_then_paste(&edited) {
                Ok(_) => {
                    history.replace_last_cleaned(edited);
                    return true;
                }
                Err(e) => log::error!("replace: {e}"),
            }
        }
    }
    let _ = inject::set_clipboard_text(&edited);
    history.replace_last_cleaned(edited);
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(m: &Message) -> String {
        m.0.iter().map(|(s, _)| s.as_str()).collect()
    }

    fn learned(n: usize) -> Vec<(String, String)> {
        (0..n).map(|i| (format!("hob{i}"), format!("HAWB{i}"))).collect()
    }

    fn unreadable(line: Option<usize>) -> Option<crate::notice::FileProblem> {
        Some(crate::notice::FileProblem {
            path: std::path::Path::new("m").join("dictionary.toml"),
            line,
            reason: "expected `=`.".into(),
            effect: crate::notice::Effect::CorrectionsOff,
        })
    }

    #[test]
    fn not_learned_comes_first_then_what_happened_to_the_text() {
        let m = fix_message(&FixOutcome { not_learned: unreadable(Some(12)), replaced: true, ..Default::default() }).unwrap();
        assert_eq!(text(&m), "Not learned: dictionary.toml line 12 has a mistake · Replaced");
        let m = fix_message(&FixOutcome { not_learned: unreadable(Some(12)), copied: true, ..Default::default() }).unwrap();
        assert_eq!(text(&m), format!("Not learned: dictionary.toml line 12 has a mistake · Copied, press {PASTE}"));
        let m = fix_message(&FixOutcome { not_learned: unreadable(None), ..Default::default() }).unwrap();
        assert_eq!(text(&m), "Not learned: dictionary.toml can't be read");
    }

    #[test]
    fn nothing_to_fix_says_so() {
        let m = fix_message(&FixOutcome { nothing_to_fix: true, ..Default::default() }).unwrap();
        assert_eq!(text(&m), "Nothing to fix yet");
    }

    #[test]
    fn learned_terms_show_spoken_and_written_in_their_colours() {
        let m = fix_message(&FixOutcome { learned: vec![("hob".into(), "HAWB".into())], replaced: true, ..Default::default() }).unwrap();
        assert_eq!(text(&m), "Learned hob → HAWB");
        let colour = |s: &str| m.0.iter().find(|(t, _)| t == s).unwrap().1;
        assert_eq!(colour("hob"), rgb(SPOKEN));
        assert_eq!(colour("HAWB"), rgb(AMBER));
    }

    #[test]
    fn copied_alone_and_with_learned() {
        let m = fix_message(&FixOutcome { copied: true, ..Default::default() }).unwrap();
        assert_eq!(text(&m), format!("Copied, press {PASTE} to paste"));
        let m = fix_message(&FixOutcome { copied: true, learned: learned(1), ..Default::default() }).unwrap();
        assert_eq!(text(&m), format!("Learned hob0 → HAWB0 · Copied, press {PASTE}"));
    }

    #[test]
    fn a_long_term_is_cut_to_keep_the_capsule_on_screen() {
        let long = "W".repeat(40);
        let m = fix_message(&FixOutcome { learned: vec![("hob".into(), long.clone())], replaced: true, ..Default::default() }).unwrap();
        let cut = format!("{}…", "W".repeat(23));
        assert_eq!(text(&m), format!("Learned hob → {cut}"));
        assert_eq!(cut.chars().count(), 24);
        let m = fix_message(&FixOutcome { learned: vec![("é".repeat(30), "ok".into())], replaced: true, ..Default::default() }).unwrap();
        assert!(text(&m).starts_with(&format!("Learned {}… → ok", "é".repeat(23))), "chars, not bytes");
    }

    #[test]
    fn many_terms_show_two_and_a_count() {
        let m = fix_message(&FixOutcome { learned: learned(4), replaced: true, ..Default::default() }).unwrap();
        assert_eq!(text(&m), "Learned hob0 → HAWB0, hob1 → HAWB1 +2 more");
    }

    #[test]
    fn a_plain_replace_says_replaced_and_a_cancel_says_nothing() {
        let m = fix_message(&FixOutcome { replaced: true, ..Default::default() }).unwrap();
        assert_eq!(text(&m), "Replaced");
        assert_eq!(fix_message(&FixOutcome::default()), None);
    }
}
