//! The pill until its macOS window lands (phase 3): nothing on screen. It keeps AppKit's events
//! flowing, as the Windows pill pumps the thread's messages, and tells the menu bar item whether
//! Murmur is listening.

use super::OverlayState;
use anyhow::Result;
use std::sync::atomic::{AtomicBool, Ordering};

static LIVE: AtomicBool = AtomicBool::new(false);

/// Whether the pill would be showing the live state now: listening, hands-free or processing.
pub fn live() -> bool {
    LIVE.load(Ordering::Relaxed)
}

pub struct Overlay;

impl Overlay {
    pub fn create() -> Result<Overlay> {
        Ok(Overlay)
    }

    pub fn centre_physical(&self) -> (f32, f32) {
        (0.0, 0.0)
    }

    pub fn above_physical(&self) -> (f32, f32) {
        (0.0, 0.0)
    }

    pub fn hwnd(&self) -> crate::platform::Window {
        crate::platform::Window::default()
    }

    pub fn set(&mut self, state: OverlayState) {
        LIVE.store(!state.is_resting(), Ordering::Relaxed);
    }

    pub fn refresh_resting(&mut self) {}

    pub fn animate(&mut self) {}

    pub fn set_quiet(&mut self, _quiet: bool) {}

    pub fn pulse(&mut self) {}

    pub fn take_right_click(&self) -> bool {
        false
    }

    /// Hands AppKit its waiting events. Quitting goes through the menu, so this never ends the loop.
    pub fn pump_once(&mut self) -> bool {
        crate::platform::pump();
        true
    }
}
