use crate::dictionary::Dictionary;
use crate::history::{History, InjectMethod};
use crate::inject;
use crate::tray::Tray;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, SetFocus, VK_CONTROL, VK_ESCAPE, VK_RETURN};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetForegroundWindow, GetMessageW, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, SetForegroundWindow, SetWindowPos, HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOMOVE, SWP_NOSIZE,
    PostQuitMessage, RegisterClassW, SendMessageW, SetWindowTextW, ShowWindow, TranslateMessage, BS_DEFPUSHBUTTON, ES_AUTOVSCROLL,
    ES_MULTILINE, ES_WANTRETURN, MSG, SW_SHOW, WM_COMMAND, WM_DESTROY, WM_KEYDOWN, WNDCLASSW, WS_BORDER, WS_CAPTION, WS_CHILD,
    WS_OVERLAPPED, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL, WM_SETFONT,
};
use windows::Win32::Graphics::Gdi::{GetStockObject, DEFAULT_GUI_FONT};

const ID_OK: usize = 1;
const ID_CANCEL: usize = 2;

static RESULT: Mutex<Option<String>> = Mutex::new(None);
static EDIT: AtomicIsize = AtomicIsize::new(0);

fn edit_hwnd() -> HWND {
    HWND(EDIT.load(Ordering::SeqCst) as *mut _)
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn edit_text() -> String {
    let edit = edit_hwnd();
    let len = GetWindowTextLengthW(edit) as usize;
    let mut buf = vec![0u16; len + 1];
    let n = GetWindowTextW(edit, &mut buf) as usize;
    String::from_utf16_lossy(&buf[..n]).replace("\r\n", "\n")
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_COMMAND => {
            match (wp.0 & 0xFFFF) as usize {
                ID_OK => {
                    *RESULT.lock().unwrap_or_else(|e| e.into_inner()) = Some(edit_text());
                    let _ = DestroyWindow(hwnd);
                }
                ID_CANCEL => {
                    let _ = DestroyWindow(hwnd);
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// Windows only lets the process that received the last input activate a window. The hotkey
/// press went to the target app, so attach to its input queue for the activation call;
/// without this the dialog opens behind the target and merely flashes on the taskbar.
unsafe fn bring_to_front(hwnd: HWND) {
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

/// Modal-ish edit dialog on the calling (main) thread. Returns the edited text, or None on cancel.
fn show_dialog(initial: &str) -> Option<String> {
    unsafe {
        *RESULT.lock().unwrap_or_else(|e| e.into_inner()) = None;
        let hinst = GetModuleHandleW(None).ok()?;
        let class = wide("MurmurFix");
        let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: hinst.into(), lpszClassName: PCWSTR(class.as_ptr()), ..Default::default() };
        let _ = RegisterClassW(&wc); // 0 on re-registration is fine
        let hwnd = CreateWindowExW(
            Default::default(), PCWSTR(class.as_ptr()), PCWSTR(wide("Murmur — fix last").as_ptr()),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
            200, 200, 560, 300, None, None, Some(hinst.into()), None,
        ).ok()?;
        let font = GetStockObject(DEFAULT_GUI_FONT);
        let edit = CreateWindowExW(
            Default::default(), PCWSTR(wide("EDIT").as_ptr()), PCWSTR::null(),
            windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_BORDER.0 | WS_VSCROLL.0 | WS_TABSTOP.0 | ES_MULTILINE as u32 | ES_AUTOVSCROLL as u32 | ES_WANTRETURN as u32),
            10, 10, 525, 200, Some(hwnd), None, Some(hinst.into()), None,
        ).ok()?;
        EDIT.store(edit.0 as isize, Ordering::SeqCst);
        let _ = SendMessageW(edit, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
        let _ = SetWindowTextW(edit, PCWSTR(wide(&initial.replace('\n', "\r\n")).as_ptr()));
        for (label, id, x) in [("OK (Ctrl+Enter)", ID_OK, 330), ("Cancel (Esc)", ID_CANCEL, 440)] {
            let b = CreateWindowExW(
                Default::default(), PCWSTR(wide("BUTTON").as_ptr()), PCWSTR(wide(label).as_ptr()),
                windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | if id == ID_OK { BS_DEFPUSHBUTTON as u32 } else { 0 }),
                x, 220, 100, 28, Some(hwnd), Some(windows::Win32::UI::WindowsAndMessaging::HMENU(id as *mut _)), Some(hinst.into()), None,
            ).ok()?;
            let _ = SendMessageW(b, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
        }
        let _ = ShowWindow(hwnd, SW_SHOW);
        bring_to_front(hwnd);
        log::info!("fix-last dialog shown, in front: {}", GetForegroundWindow() == hwnd);
        let _ = SetFocus(Some(edit_hwnd()));

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.message == WM_KEYDOWN {
                let ctrl = (GetKeyState(VK_CONTROL.0 as i32) as u16 & 0x8000) != 0;
                if msg.wParam.0 as u16 == VK_RETURN.0 && ctrl {
                    *RESULT.lock().unwrap_or_else(|e| e.into_inner()) = Some(edit_text());
                    let _ = DestroyWindow(hwnd);
                    continue;
                }
                if msg.wParam.0 as u16 == VK_ESCAPE.0 {
                    let _ = DestroyWindow(hwnd);
                    continue;
                }
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        RESULT.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
}

pub fn fix_last(history: &mut History, dict: &Arc<Mutex<Dictionary>>, tray: &Tray) {
    let Some(last) = history.last().cloned() else {
        tray.notify("Fix last", "nothing to fix yet");
        return;
    };
    let target_hwnd = inject::foreground_hwnd();
    let Some(edited) = show_dialog(&last.cleaned) else { return };
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
        // give focus back to the target before undo+paste
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
        tray.notify("Corrected text copied", "target window changed; paste it yourself");
        history.replace_last_cleaned(edited);
    }
}
