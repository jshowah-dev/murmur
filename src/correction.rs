use crate::correction_ui::{self, rgb, AMBER, MUTED, SPOKEN, TEXT};
use crate::dictionary::Dictionary;
use crate::history::{History, InjectMethod};
use crate::inject;
use crate::mote::Message;
use crate::tray::Tray;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow, SetWindowPos, HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOMOVE,
    SWP_NOSIZE,
};

/// Windows only lets the process that received the last input activate a window. The hotkey
/// press went to the target app, so attach to its input queue for the activation call;
/// without this the dialog opens behind the target and merely flashes on the taskbar.
pub(crate) unsafe fn bring_to_front(hwnd: HWND) {
    unsafe {
        let fg = GetForegroundWindow();
        let fg_tid = GetWindowThreadProcessId(fg, None);
        let me = GetCurrentThreadId();
        let attached = fg_tid != 0 && fg_tid != me && AttachThreadInput(fg_tid, me, true).as_bool();
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
        let _ = SetWindowPos(hwnd, Some(HWND_NOTOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
        let _ = SetForegroundWindow(hwnd);
        if attached {
            let _ = AttachThreadInput(fg_tid, me, false);
        }
    }
}

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
    /// the card's centre when it closed, physical pixels
    pub card: Option<(f32, f32)>,
}

/// Terms named in a "Learned" message; any more are counted.
const NAMED: usize = 2;

/// What the mote says after fix-last, or None when the landing says it all.
pub(crate) fn fix_message(o: &FixOutcome) -> Option<Message> {
    if o.nothing_to_fix {
        return Some(Message::plain("Nothing to fix yet"));
    }
    let (text, muted) = (rgb(TEXT), rgb(MUTED));
    let mut spans = Vec::new();
    if !o.learned.is_empty() {
        spans.push(("Learned ".to_string(), text));
        for (i, (spoken, written)) in o.learned.iter().take(NAMED).enumerate() {
            if i > 0 {
                spans.push((", ".into(), muted));
            }
            spans.push((spoken.clone(), rgb(SPOKEN)));
            spans.push((" → ".into(), muted));
            spans.push((written.clone(), rgb(AMBER)));
        }
        if o.learned.len() > NAMED {
            spans.push((format!(" +{} more", o.learned.len() - NAMED), muted));
        }
    }
    match (spans.is_empty(), o.copied) {
        (true, false) => return None,
        (true, true) => spans.push(("Copied, press Ctrl+V to paste".into(), text)),
        (false, true) => spans.push((" · Copied, press Ctrl+V".into(), text)),
        (false, false) => {}
    }
    Some(Message(spans))
}

pub fn fix_last(history: &mut History, dict: &Arc<Mutex<Dictionary>>, tray: &Tray) -> FixOutcome {
    let Some(last) = history.last().cloned() else {
        return FixOutcome { nothing_to_fix: true, ..Default::default() };
    };
    let target_hwnd = inject::foreground_hwnd();
    let snapshot = dict.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let heard_at = last.inject.as_ref().map(|r| r.at);
    let (edited, card) = correction_ui::show(&last.cleaned, heard_at, snapshot, HWND(target_hwnd as *mut _));
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
                log::error!("reload dictionary: {e}");
                tray.notify("Dictionary", "not loaded — fix dictionary.toml before corrections are saved");
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
        unsafe { bring_to_front(HWND(target_hwnd as *mut _)) };
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
        assert_eq!(text(&m), "Copied, press Ctrl+V to paste");
        let m = fix_message(&FixOutcome { copied: true, learned: learned(1), ..Default::default() }).unwrap();
        assert_eq!(text(&m), "Learned hob0 → HAWB0 · Copied, press Ctrl+V");
    }

    #[test]
    fn many_terms_show_two_and_a_count() {
        let m = fix_message(&FixOutcome { learned: learned(4), replaced: true, ..Default::default() }).unwrap();
        assert_eq!(text(&m), "Learned hob0 → HAWB0, hob1 → HAWB1 +2 more");
    }

    #[test]
    fn a_plain_replace_or_a_cancel_says_nothing() {
        assert_eq!(fix_message(&FixOutcome { replaced: true, ..Default::default() }), None);
        assert_eq!(fix_message(&FixOutcome::default()), None);
    }
}
