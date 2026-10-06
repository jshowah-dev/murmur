use super::Window;
use anyhow::{Context, Result};
use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSApplication, NSApplicationActivationOptions, NSApplicationActivationPolicy, NSEventMask, NSRunningApplication, NSWorkspace,
};
use objc2_core_graphics::{CGEventSource, CGEventSourceStateID};
use objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSDictionary, NSNumber, NSString};
use std::fs::{File, TryLockError};
use std::path::Path;

/// A screen rectangle in Win32's shape, so the shared code needs one kind of rectangle.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

// Windows virtual-key codes, which config.toml's key names map to on both systems
const VK_SHIFT: u16 = 0x10;
const VK_CONTROL: u16 = 0x11;
const VK_MENU: u16 = 0x12;
const VK_CAPITAL: u16 = 0x14;
const VK_ESCAPE: u16 = 0x1B;
const VK_LWIN: u16 = 0x5B;
const VK_RWIN: u16 = 0x5C;
const VK_F1: u16 = 0x70;
const VK_LSHIFT: u16 = 0xA0;
const VK_RSHIFT: u16 = 0xA1;
const VK_LCONTROL: u16 = 0xA2;
const VK_RCONTROL: u16 = 0xA3;
const VK_LMENU: u16 = 0xA4;
const VK_RMENU: u16 = 0xA5;

/// The device-dependent bits of the modifier flags (IOKit's NX_DEVICE*KEYMASK): one per side.
/// macOS reports these without the Input Monitoring permission that per-key state needs.
fn modifier_bits(vk: u16) -> Option<u64> {
    const LCTL: u64 = 0x0001;
    const LSHIFT: u64 = 0x0002;
    const RSHIFT: u64 = 0x0004;
    const LCMD: u64 = 0x0008;
    const RCMD: u64 = 0x0010;
    const LALT: u64 = 0x0020;
    const RALT: u64 = 0x0040;
    const RCTL: u64 = 0x2000;
    Some(match vk {
        VK_SHIFT => LSHIFT | RSHIFT,
        VK_CONTROL => LCTL | RCTL,
        VK_MENU => LALT | RALT,
        VK_LSHIFT => LSHIFT,
        VK_RSHIFT => RSHIFT,
        VK_LCONTROL => LCTL,
        VK_RCONTROL => RCTL,
        VK_LMENU => LALT,
        VK_RMENU => RALT,
        VK_LWIN => LCMD,
        VK_RWIN => RCMD,
        _ => return None,
    })
}

/// The Mac key code for a non-modifier key Murmur watches: Esc, Caps Lock, F1 to F20.
fn key_code(vk: u16) -> Option<u16> {
    // kVK_F1 … kVK_F20 from HIToolbox's Events.h; they are not in order
    const F_KEYS: [u16; 20] =
        [0x7A, 0x78, 0x63, 0x76, 0x60, 0x61, 0x62, 0x64, 0x65, 0x6D, 0x67, 0x6F, 0x69, 0x6B, 0x71, 0x6A, 0x40, 0x4F, 0x50, 0x5A];
    match vk {
        VK_ESCAPE => Some(0x35),
        VK_CAPITAL => Some(0x39),
        _ => F_KEYS.get(vk.checked_sub(VK_F1)? as usize).copied(),
    }
}

/// Whether the key with this Windows virtual-key code is held right now. Modifiers need no
/// permission; other keys read as up until Murmur has Input Monitoring.
pub fn key_down(vk: u16) -> bool {
    if let Some(bits) = modifier_bits(vk) {
        return CGEventSource::flags_state(CGEventSourceStateID::HIDSystemState).0 & bits != 0;
    }
    key_code(vk).is_some_and(|code| CGEventSource::key_state(CGEventSourceStateID::CombinedSessionState, code))
}

// TODO(macos phase 2): Murmur's egui windows ask for always-on-top in their ViewportBuilder.
pub fn keep_on_top(_w: Window) {}

// TODO(macos phase 2): setFrameTopLeftPoint on the NSView's window, flipped from top-left coordinates.
pub fn move_to(_w: Window, _x: i32, _y: i32) {}

// TODO(macos phase 2): the NSView's window frame, flipped to top-left screen coordinates.
pub fn window_rect(_w: Window) -> Option<Rect> {
    None
}

pub fn is_minimized(_w: Window) -> bool {
    false
}

/// Murmur's windows all belong to one app, so "in front" means Murmur is the active app.
pub fn is_foreground(_w: Window) -> bool {
    MainThreadMarker::new().is_some_and(|mtm| NSApplication::sharedApplication(mtm).isActive())
}

/// Makes Murmur the active app, so its window takes the keyboard.
pub fn raise(_w: Window) {
    if let Some(mtm) = MainThreadMarker::new() {
        NSApplication::sharedApplication(mtm).activate();
    }
}

/// Gives the focus back to the app a dictation went to (`inject::foreground_hwnd`, a pid).
pub fn focus_target(target: isize) {
    if let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(target as libc::pid_t) {
        app.activateWithOptions(NSApplicationActivationOptions::empty());
    }
}

// TODO(macos phase 2): the editor's second launch could bring the first forward.
pub fn raise_titled(_title: &str) {}

pub fn open_path(p: &Path) {
    let _ = std::process::Command::new("open").arg(p).spawn();
}

/// System Settings › Accessibility › Display › Reduce motion.
pub fn reduced_motion() -> bool {
    NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

/// Held for as long as this process is the one instance of `name`: an exclusive lock on a file.
pub struct Instance(#[allow(dead_code)] File);

fn lock_file(name: &str) -> Result<File> {
    // tests take and drop instances with made-up names; those lock files go in the temp folder
    let dir = if cfg!(test) { std::env::temp_dir() } else { murmur_lib::config::config_dir() };
    std::fs::create_dir_all(&dir).context("create config dir")?;
    let path = dir.join(super::lock_name(name));
    File::options().create(true).truncate(false).write(true).open(&path).with_context(|| format!("open {}", path.display()))
}

/// Claims the single instance `name`; None when another process already has it.
pub fn single_instance(name: &str) -> Result<Option<Instance>> {
    let f = lock_file(name)?;
    match f.try_lock() {
        Ok(()) => Ok(Some(Instance(f))),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(e)) => Err(e).context("lock"),
    }
}

/// Whether some process holds the single instance `name`, from outside it.
pub fn instance_exists(name: &str) -> bool {
    lock_file(name).is_ok_and(|f| matches!(f.try_lock(), Err(TryLockError::WouldBlock)))
}

/// Makes Murmur a menu bar app (no Dock icon) and gets AppKit ready for `pump`. Call first thing,
/// on the main thread.
pub fn init_app() {
    let mtm = MainThreadMarker::new().expect("init_app on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    app.finishLaunching();
}

/// Hands AppKit the events waiting for it (the menu bar item's clicks among them), without waiting.
pub fn pump() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);
    let past = NSDate::distantPast();
    while let Some(ev) = unsafe { app.nextEventMatchingMask_untilDate_inMode_dequeue(NSEventMask::Any, Some(&past), NSDefaultRunLoopMode, true) } {
        app.sendEvent(&ev);
    }
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrustedWithOptions(options: *const NSDictionary<NSString, NSNumber>) -> bool;
}

/// Whether Murmur may post the Cmd+V that pastes. With `prompt`, macOS shows its "allow in
/// System Settings" dialog when it may not.
pub fn accessibility_trusted(prompt: bool) -> bool {
    // the value of ApplicationServices' kAXTrustedCheckOptionPrompt
    let key = NSString::from_str("AXTrustedCheckOptionPrompt");
    let options = NSDictionary::from_slices(&[&*key], &[&*NSNumber::numberWithBool(prompt)]);
    unsafe { AXIsProcessTrustedWithOptions(&*options) }
}

/// The app in front, by process id.
pub fn frontmost_pid() -> isize {
    NSWorkspace::sharedWorkspace().frontmostApplication().map_or(0, |a| a.processIdentifier() as isize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_side_of_a_modifier_has_its_own_bit() {
        assert_eq!(modifier_bits(VK_RMENU), Some(0x40));
        assert_eq!(modifier_bits(VK_LMENU), Some(0x20));
        assert_eq!(modifier_bits(VK_MENU), Some(0x60));
        assert_eq!(modifier_bits(VK_RCONTROL), Some(0x2000));
        assert_eq!(modifier_bits(VK_ESCAPE), None);
    }

    #[test]
    fn f_keys_and_esc_have_mac_key_codes() {
        assert_eq!(key_code(VK_ESCAPE), Some(0x35));
        assert_eq!(key_code(VK_F1), Some(0x7A));
        assert_eq!(key_code(VK_F1 + 12), Some(0x69)); // F13
        assert_eq!(key_code(VK_F1 + 20), None); // F21: none on a Mac
        assert_eq!(key_code(0x41), None);
    }
}
