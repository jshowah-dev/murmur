#![allow(dead_code)]
use crate::history::InjectRecord;
use anyhow::{anyhow, Result};
use std::thread::sleep;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
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
        let mut len = 0;
        while *p.add(len) != 0 {
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
            return Err(anyhow!("GlobalLock"));
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len());
        let _ = GlobalUnlock(h);
        SetClipboardData(CF_UNICODETEXT, Some(HANDLE(h.0)))?;
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
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
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
            SendInput(chunk, std::mem::size_of::<INPUT>() as i32);
        }
    }
}

fn record(text: &str) -> InjectRecord {
    InjectRecord { hwnd: foreground_hwnd(), len: text.chars().count(), at: Instant::now() }
}

/// Save clipboard text, set ours, Ctrl+V, restore. Falls back to unicode typing when the clipboard is unavailable.
pub fn paste(text: &str) -> Result<InjectRecord> {
    let saved = match Clipboard::open() {
        Ok(_c) => {
            let saved = read_text_locked();
            write_text_locked(text)?;
            saved
        }
        Err(e) => {
            log::warn!("{e}; falling back to unicode typing");
            type_unicode(text);
            return Ok(record(text));
        }
    };
    sleep(Duration::from_millis(30));
    chord(VK_V);
    sleep(Duration::from_millis(150));
    if let Some(prev) = saved {
        if let Ok(_c) = Clipboard::open() {
            let _ = write_text_locked(&prev);
        }
    }
    Ok(record(text))
}

/// Undo the previous paste in the target app, then paste `text`.
pub fn undo_then_paste(text: &str) -> Result<InjectRecord> {
    chord(VK_Z);
    sleep(Duration::from_millis(100));
    paste(text)
}
