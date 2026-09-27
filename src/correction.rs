use crate::correction_ui;
use crate::dictionary::Dictionary;
use crate::history::{History, InjectMethod};
use crate::inject;
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

pub fn fix_last(history: &mut History, dict: &Arc<Mutex<Dictionary>>, tray: &Tray) {
    let Some(last) = history.last().cloned() else {
        tray.notify("Fix last", "nothing to fix yet");
        return;
    };
    let target_hwnd = inject::foreground_hwnd();
    let snapshot = dict.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let heard_at = last.inject.as_ref().map(|r| r.at);
    let Some(edited) = correction_ui::show(&last.cleaned, heard_at, snapshot, HWND(target_hwnd as *mut _)) else { return };
    let edited = edited.trim().to_string();
    if edited == last.cleaned {
        return;
    }
    let learned = {
        let mut d = dict.lock().unwrap_or_else(|e| e.into_inner());
        match Dictionary::load_or_seed() {
            Ok(fresh) => *d = fresh,
            Err(e) => {
                log::error!("reload dictionary: {e}");
                tray.notify("Dictionary", "not loaded — fix dictionary.toml before corrections are saved");
                drop(d);
                paste_or_copy_correction(history, tray, &last, target_hwnd, edited);
                return;
            }
        }
        let l = d.learn(&last.cleaned, &edited);
        if !l.is_empty() {
            if let Err(e) = d.save() {
                log::error!("save dictionary: {e}");
            }
        }
        l
    };
    if !learned.is_empty() {
        let summary = learned.iter().map(|t| format!("{} → {}", t.spoken.last().cloned().unwrap_or_default(), t.written)).collect::<Vec<_>>().join(", ");
        tray.notify("Learned", &summary);
    }
    paste_or_copy_correction(history, tray, &last, target_hwnd, edited);
}

fn paste_or_copy_correction(history: &mut History, tray: &Tray, last: &crate::history::Entry, target_hwnd: isize, edited: String) {
    let fresh = last
        .inject
        .as_ref()
        .map(|r| r.hwnd == target_hwnd && r.at.elapsed() < Duration::from_secs(60) && r.method == InjectMethod::Paste)
        .unwrap_or(false);
    if fresh {
        // the dialog took focus; hand it back to the target before undo+paste
        unsafe { bring_to_front(HWND(target_hwnd as *mut _)) };
        std::thread::sleep(Duration::from_millis(150));
        if inject::foreground_hwnd() != target_hwnd {
            let _ = inject::set_clipboard_text(&edited);
            tray.notify("Corrected text copied", "target window changed; paste it yourself");
            history.replace_last_cleaned(edited);
            return;
        }
        match inject::undo_then_paste(&edited) {
            Ok(_) => history.replace_last_cleaned(edited),
            Err(e) => tray.notify("Replace failed", &e.to_string()),
        }
    } else {
        let _ = inject::set_clipboard_text(&edited);
        tray.notify("Corrected text copied", "last dictation is older than a minute or was not pasted; paste it yourself");
        history.replace_last_cleaned(edited);
    }
}
