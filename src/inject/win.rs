use super::restore::{Board, Restorer, RESTORE_AFTER};
use crate::history::{InjectMethod, InjectRecord};
use anyhow::{anyhow, Result};
use std::thread::sleep;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData, GetClipboardSequenceNumber, OpenClipboard,
    SetClipboardData,
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

/// Formats whose handle is a GDI object or a window message, not memory that can be copied.
/// Windows makes a CF_DIB copy of a bitmap and keeps it, so an image still comes back.
fn is_memory_format(f: u32) -> bool {
    // CF_BITMAP, CF_METAFILEPICT, CF_PALETTE, CF_ENHMETAFILE, CF_OWNERDISPLAY, CF_DSPBITMAP,
    // CF_DSPMETAFILEPICT, CF_DSPENHMETAFILE, then the CF_GDIOBJFIRST..CF_GDIOBJLAST range
    !matches!(f, 2 | 3 | 9 | 14 | 0x80 | 0x82 | 0x83 | 0x8E) && !(0x300..=0x3FF).contains(&f)
}

/// Every memory format on the open clipboard, copied out.
fn save_locked() -> Vec<(u32, Vec<u8>)> {
    let mut saved = Vec::new();
    let mut f = 0;
    loop {
        f = unsafe { EnumClipboardFormats(f) };
        if f == 0 {
            return saved;
        }
        if !is_memory_format(f) {
            continue;
        }
        unsafe {
            let Ok(h) = GetClipboardData(f) else { continue };
            let g = HGLOBAL(h.0);
            let p = GlobalLock(g) as *const u8;
            if p.is_null() {
                continue;
            }
            saved.push((f, std::slice::from_raw_parts(p, GlobalSize(g)).to_vec()));
            let _ = GlobalUnlock(g);
        }
    }
}

/// Puts `bytes` on the open clipboard as format `f`; the clipboard owns the memory once it's set.
fn set_locked(f: u32, bytes: &[u8]) -> Result<()> {
    unsafe {
        let h: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1))?;
        let p = GlobalLock(h) as *mut u8;
        if p.is_null() {
            let _ = GlobalFree(Some(h));
            return Err(anyhow!("GlobalLock"));
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        let _ = GlobalUnlock(h);
        if let Err(e) = SetClipboardData(f, Some(HANDLE(h.0))) {
            let _ = GlobalFree(Some(h));
            return Err(e.into());
        }
    }
    Ok(())
}

pub fn set_clipboard_text(text: &str) -> Result<()> {
    let wide: Vec<u8> = text.encode_utf16().chain(std::iter::once(0)).flat_map(u16::to_le_bytes).collect();
    let _c = Clipboard::open()?;
    unsafe { EmptyClipboard()? };
    set_locked(CF_UNICODETEXT, &wide)
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

struct SystemClipboard;

impl Board for SystemClipboard {
    type Saved = Vec<(u32, Vec<u8>)>;
    fn save(&self) -> Result<Option<Self::Saved>> {
        let _c = Clipboard::open()?;
        let saved = save_locked();
        Ok((!saved.is_empty()).then_some(saved))
    }
    fn put_back(&self, saved: &Self::Saved) -> Result<()> {
        let _c = Clipboard::open()?;
        unsafe { EmptyClipboard()? };
        for (f, bytes) in saved {
            // one format failing (a private one an app won't take back) shouldn't lose the rest
            if let Err(e) = set_locked(*f, bytes) {
                log::warn!("clipboard format {f} didn't go back: {e}");
            }
        }
        Ok(())
    }
    fn write(&self, text: &str) -> Result<()> {
        set_clipboard_text(text)
    }
    fn count(&self) -> i64 {
        unsafe { GetClipboardSequenceNumber() as i64 }
    }
}

static CLIPBOARD: Restorer<SystemClipboard> = Restorer::new(SystemClipboard, RESTORE_AFTER);

/// Save the clipboard, set our text, Ctrl+V, restore a little later. Types the text in when the
/// clipboard can't be opened, read or written.
pub fn paste(text: &str) -> Result<InjectRecord> {
    let swapped = match CLIPBOARD.put(text) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("{e}; falling back to unicode typing");
            let hwnd = foreground_hwnd();
            type_unicode(text);
            return Ok(record(hwnd, InjectMethod::Typed));
        }
    };
    sleep(Duration::from_millis(30));
    let hwnd = foreground_hwnd();
    chord(VK_V);
    CLIPBOARD.restore_later(swapped);
    Ok(record(hwnd, InjectMethod::Paste))
}

/// Undo the last edit in the foreground app.
pub fn undo() {
    chord(VK_Z);
}

/// Undo the previous paste in the target app, then paste `text`.
pub fn undo_then_paste(text: &str) -> Result<InjectRecord> {
    undo();
    sleep(Duration::from_millis(100));
    paste(text)
}
