use crate::history::{InjectMethod, InjectRecord};
use anyhow::{anyhow, Result};
use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
use objc2_core_graphics::{CGEvent, CGEventFlags, CGEventTapLocation, CGKeyCode};
use objc2_foundation::NSString;
use std::thread::sleep;
use std::time::{Duration, Instant};

// kVK_ANSI_V and kVK_ANSI_Z from HIToolbox's Events.h
const KEY_V: CGKeyCode = 0x09;
const KEY_Z: CGKeyCode = 0x06;

/// The app in front, by process id: what a dictation lands in.
pub fn foreground_hwnd() -> isize {
    crate::platform::frontmost_pid()
}

fn read_text() -> Option<String> {
    unsafe { NSPasteboard::generalPasteboard().stringForType(NSPasteboardTypeString) }.map(|s| s.to_string())
}

pub fn set_clipboard_text(text: &str) -> Result<()> {
    let pb = NSPasteboard::generalPasteboard();
    pb.clearContents();
    if unsafe { pb.setString_forType(&NSString::from_str(text), NSPasteboardTypeString) } {
        Ok(())
    } else {
        Err(anyhow!("the clipboard didn't take the text"))
    }
}

/// Cmd + `key`, sent as if typed. The flags are set on each event, so a modifier still held
/// (the push-to-talk key) doesn't change the shortcut.
fn command(key: CGKeyCode) {
    for down in [true, false] {
        let ev = CGEvent::new_keyboard_event(None, key, down);
        CGEvent::set_flags(ev.as_deref(), CGEventFlags::MaskCommand);
        CGEvent::post(CGEventTapLocation::HIDEventTap, ev.as_deref());
    }
}

fn record(pid: isize) -> InjectRecord {
    InjectRecord { hwnd: pid, at: Instant::now(), method: InjectMethod::Paste }
}

/// Save clipboard text, set ours, Cmd+V, restore. Without the Accessibility permission macOS drops
/// the Cmd+V, so the text is left on the clipboard for you to paste.
pub fn paste(text: &str) -> Result<InjectRecord> {
    let saved = read_text();
    set_clipboard_text(text)?;
    if !crate::platform::accessibility_trusted(false) {
        return Err(anyhow!("Accessibility isn't allowed for Murmur, so the text was left on the clipboard"));
    }
    sleep(Duration::from_millis(30));
    let pid = foreground_hwnd();
    command(KEY_V);
    sleep(Duration::from_millis(150));
    if let Some(prev) = saved {
        let _ = set_clipboard_text(&prev);
    }
    Ok(record(pid))
}

/// Undo the previous paste in the target app, then paste `text`.
#[allow(dead_code)] // fix-last's card, phase 2 on macOS
pub fn undo_then_paste(text: &str) -> Result<InjectRecord> {
    command(KEY_Z);
    sleep(Duration::from_millis(100));
    paste(text)
}
