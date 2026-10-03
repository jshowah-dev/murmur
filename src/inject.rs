use crate::history::{InjectMethod, InjectRecord};
use anyhow::{anyhow, Result};
use std::thread::sleep;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VIRTUAL_KEY, VK_CONTROL,
    VK_V, VK_Z,
};
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

const CF_UNICODETEXT: u32 = 13;

pub fn foreground_hwnd() -> isize {
    unsafe { GetForegroundWindow().0 as isize }
}

struct Clipboard;

impl Clipboard {
    fn open() -> Result<Clipboard> {
        for _ in 0..3 {
            if unsafe { OpenClipboard(None) }.is_ok() {
                return Ok(Clipboard);
            }
            sleep(Duration::from_millis(50));
        }
        Err(anyhow!("clipboard busy"))
    }
}

impl Drop for Clipboard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

fn read_text_locked() -> Option<String> {
    unsafe {
        if IsClipboardFormatAvailable(CF_UNICODETEXT).is_err() {
            return None;
        }
        let h = GetClipboardData(CF_UNICODETEXT).ok()?;
        let p = GlobalLock(HGLOBAL(h.0)) as *const u16;
        if p.is_null() {
            return None;
        }
        let max = GlobalSize(HGLOBAL(h.0)) / 2;
        let mut len = 0;
        while len < max && *p.add(len) != 0 {
            len += 1;
        }
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, len));
        let _ = GlobalUnlock(HGLOBAL(h.0));
        Some(s)
    }
}

fn write_text_locked(text: &str) -> Result<()> {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        EmptyClipboard()?;
        let h: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2)?;
        let p = GlobalLock(h) as *mut u16;
        if p.is_null() {
            let _ = GlobalFree(Some(h));
            return Err(anyhow!("GlobalLock"));
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len());
        let _ = GlobalUnlock(h);
        if let Err(e) = SetClipboardData(CF_UNICODETEXT, Some(HANDLE(h.0))) {
            let _ = GlobalFree(Some(h));
            return Err(e.into());
        }
    }
    Ok(())
}

pub fn set_clipboard_text(text: &str) -> Result<()> {
    let _c = Clipboard::open()?;
    write_text_locked(text)
}

fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { Default::default() }, time: 0, dwExtraInfo: 0 },
        },
    }
}

fn chord(vk: VIRTUAL_KEY) {
    let inputs = [key(VK_CONTROL, false), key(vk, false), key(vk, true), key(VK_CONTROL, true)];
    unsafe {
        let sent = SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
        if sent as usize != inputs.len() {
            log::warn!("SendInput sent {sent} of {} inputs", inputs.len());
        }
    }
}

fn type_unicode(text: &str) {
    let mut inputs = Vec::with_capacity(text.len() * 2);
    for u in text.encode_utf16() {
        for up in [false, true] {
            inputs.push(INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(0),
                        wScan: u,
                        dwFlags: if up { KEYEVENTF_UNICODE | KEYEVENTF_KEYUP } else { KEYEVENTF_UNICODE },
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            });
        }
    }
    for chunk in inputs.chunks(64) {
        unsafe {
            let sent = SendInput(chunk, std::mem::size_of::<INPUT>() as i32);
            if sent as usize != chunk.len() {
                log::warn!("SendInput sent {sent} of {} inputs", chunk.len());
            }
        }
    }
}

fn record(hwnd: isize, method: InjectMethod) -> InjectRecord {
    InjectRecord { hwnd, at: Instant::now(), method }
}

/// Save clipboard text, set ours, Ctrl+V, restore. Types the text in when the clipboard can't be opened or written.
pub fn paste(text: &str) -> Result<InjectRecord> {
    // None: the clipboard can't be used, so type the text in instead
    let saved = match Clipboard::open() {
        Ok(_c) => {
            let saved = read_text_locked();
            match write_text_locked(text) {
                Ok(()) => Some(saved),
                Err(e) => {
                    log::warn!("{e}; falling back to unicode typing");
                    if let Some(prev) = &saved {
                        let _ = write_text_locked(prev);
                    }
                    None
                }
            }
        }
        Err(e) => {
            log::warn!("{e}; falling back to unicode typing");
            None
        }
    };
    let Some(saved) = saved else {
        let hwnd = foreground_hwnd();
        type_unicode(text);
        return Ok(record(hwnd, InjectMethod::Typed));
    };
    sleep(Duration::from_millis(30));
    let hwnd = foreground_hwnd();
    chord(VK_V);
    sleep(Duration::from_millis(150));
    if let Some(prev) = saved {
        if let Ok(_c) = Clipboard::open() {
            let _ = write_text_locked(&prev);
        }
    }
    Ok(record(hwnd, InjectMethod::Paste))
}

/// Undo the previous paste in the target app, then paste `text`.
pub fn undo_then_paste(text: &str) -> Result<InjectRecord> {
    chord(VK_Z);
    sleep(Duration::from_millis(100));
    paste(text)
}
