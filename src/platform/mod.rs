//! The OS calls the app makes outside the pipeline: keys, Murmur's own windows, the app in
//! front, single instance. One half per system, with the same functions.

#[cfg(windows)]
mod win;
#[cfg(windows)]
pub use win::*;

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
pub use mac::*;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// One of Murmur's own windows (an egui viewport): its HWND on Windows, its NSView on macOS.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Window(pub isize);

pub fn window_of(cc: &eframe::CreationContext) -> Window {
    match cc.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Win32(h)) => Window(h.hwnd.get()),
        Ok(RawWindowHandle::AppKit(h)) => Window(h.ns_view.as_ptr() as isize),
        _ => Window::default(),
    }
}

/// The lock file for a single-instance `name` like "Local\\Murmur.SingleInstance": its last part.
#[cfg_attr(windows, allow(dead_code))]
fn lock_name(name: &str) -> String {
    format!("{}.lock", name.rsplit(['\\', '.']).next().unwrap_or(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lock_is_named_after_the_last_part_of_the_mutex() {
        assert_eq!(lock_name("Local\\Murmur.SingleInstance"), "SingleInstance.lock");
        assert_eq!(lock_name("Local\\Murmur.DictionaryEditor"), "DictionaryEditor.lock");
    }
}
