use super::restore::{Board, Restorer, RESTORE_AFTER};
use crate::history::{InjectMethod, InjectRecord};
use anyhow::{anyhow, Result};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{NSPasteboard, NSPasteboardItem, NSPasteboardTypeString, NSPasteboardWriting};
use objc2_core_graphics::{CGEvent, CGEventFlags, CGEventTapLocation, CGKeyCode};
use objc2_foundation::{NSArray, NSData, NSString};
use std::thread::sleep;
use std::time::{Duration, Instant};

// kVK_ANSI_V and kVK_ANSI_Z from HIToolbox's Events.h
const KEY_V: CGKeyCode = 0x09;
const KEY_Z: CGKeyCode = 0x06;

/// The app in front, by process id: what a dictation lands in.
pub fn foreground_hwnd() -> isize {
    crate::platform::frontmost_pid()
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

struct Pasteboard;

/// Each pasteboard item as its (type, bytes) pairs.
type Items = Vec<Vec<(String, Vec<u8>)>>;

impl Board for Pasteboard {
    type Saved = Items;
    fn save(&self) -> Result<Option<Items>> {
        let Some(items) = NSPasteboard::generalPasteboard().pasteboardItems() else { return Ok(None) };
        let saved: Items = items
            .iter()
            .map(|item| item.types().iter().filter_map(|t| item.dataForType(&t).map(|d| (t.to_string(), d.to_vec()))).collect::<Vec<_>>())
            .filter(|types| !types.is_empty())
            .collect();
        Ok((!saved.is_empty()).then_some(saved))
    }
    fn put_back(&self, saved: &Items) -> Result<()> {
        let pb = NSPasteboard::generalPasteboard();
        pb.clearContents();
        let items: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = saved
            .iter()
            .map(|types| {
                let item = NSPasteboardItem::new();
                for (t, bytes) in types {
                    item.setData_forType(&NSData::with_bytes(bytes), &NSString::from_str(t));
                }
                ProtocolObject::from_retained(item)
            })
            .collect();
        if pb.writeObjects(&NSArray::from_retained_slice(&items)) {
            Ok(())
        } else {
            Err(anyhow!("the clipboard didn't take its old contents back"))
        }
    }
    fn write(&self, text: &str) -> Result<()> {
        set_clipboard_text(text)
    }
    fn count(&self) -> i64 {
        NSPasteboard::generalPasteboard().changeCount() as i64
    }
}

static CLIPBOARD: Restorer<Pasteboard> = Restorer::new(Pasteboard, RESTORE_AFTER);

/// Save the clipboard, set our text, Cmd+V, restore a little later. Without the Accessibility
/// permission macOS drops the Cmd+V, so the text is left on the clipboard for you to paste.
pub fn paste(text: &str) -> Result<InjectRecord> {
    let swapped = CLIPBOARD.put(text)?;
    if !crate::platform::accessibility_trusted(false) {
        return Err(anyhow!("Accessibility isn't allowed for Murmur, so the text was left on the clipboard"));
    }
    sleep(Duration::from_millis(30));
    let pid = foreground_hwnd();
    command(KEY_V);
    CLIPBOARD.restore_later(swapped);
    Ok(record(pid))
}

/// Before quitting: the clipboard a paste borrowed goes back.
pub fn finish() {
    CLIPBOARD.finish();
}

/// Undo the last edit in the foreground app.
pub fn undo() {
    command(KEY_Z);
}

/// Undo the previous paste in the target app, then paste `text`.
#[allow(dead_code)] // fix-last's card, phase 2 on macOS
pub fn undo_then_paste(text: &str) -> Result<InjectRecord> {
    undo();
    sleep(Duration::from_millis(100));
    paste(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Uses the real clipboard, so it's run by hand: `cargo test --lib -- --ignored pasteboard`.
    /// Whatever was on the clipboard is put back at the end.
    #[test]
    #[ignore]
    fn pasteboard_keeps_formats_that_arent_text() {
        let yours = Pasteboard.save().unwrap();
        let pb = NSPasteboard::generalPasteboard();
        pb.clearContents();
        let item = NSPasteboardItem::new();
        item.setData_forType(&NSData::with_bytes(b"\x89PNG fake"), &NSString::from_str("public.png"));
        assert!(pb.writeObjects(&NSArray::from_retained_slice(&[ProtocolObject::from_retained(item)])));

        let saved = Pasteboard.save().unwrap().expect("the png was saved");
        Pasteboard.write("dictated").unwrap();
        Pasteboard.put_back(&saved).unwrap();
        let back = Pasteboard.save().unwrap();

        match &yours {
            Some(y) => Pasteboard.put_back(y).unwrap(),
            None => {
                pb.clearContents();
            }
        }
        assert_eq!(back, Some(vec![vec![("public.png".to_string(), b"\x89PNG fake".to_vec())]]));
    }
}
