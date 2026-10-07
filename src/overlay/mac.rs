//! The pill on macOS: drawn by the shared `Pill` and `render`, shown in a `platform::Panel`
//! above the Dock. It lets clicks through except while the cursor is over it, where a
//! right-click opens the menu bar item's menu. No hover hint yet.

use super::{nearness, render_scaled, OverlayState, Pill};
use crate::platform::{self, Panel, Rect};
use anyhow::Result;
use std::sync::atomic::{AtomicBool, Ordering};

static LIVE: AtomicBool = AtomicBool::new(false);

/// Whether the pill shows the live state now: listening, hands-free or processing.
pub fn live() -> bool {
    LIVE.load(Ordering::Relaxed)
}

/// Points between the bottom of the usable screen (the Dock's top) and the pill, as on Windows.
const LIFT: f32 = 24.0;

pub struct Overlay {
    panel: Panel,
    pill: Pill,
    /// where the pill was last drawn: (x, y, w, h) points from the main display's top-left
    at: (f32, f32, f32, f32),
}

impl Overlay {
    pub fn create() -> Result<Overlay> {
        Ok(Overlay { panel: Panel::new()?, pill: Pill::new(crate::editor_kit::reduced_motion()), at: (0.0, 0.0, 0.0, 0.0) })
    }

    /// Bottom-centre of the screen you're working on; the bottom edge stays put as the pill grows.
    fn paint(&mut self) {
        let ((sx, sy, sw, sh), scale) = platform::work_area();
        let (w, h, pixels) = render_scaled(self.pill.state, self.pill.look, scale);
        let (w, h) = (w as f32, h as f32);
        self.at = ((sx + sw / 2.0 - w / 2.0).round(), (sy + sh - h - LIFT).round(), w, h);
        let size = (crate::canvas::pixels(w as i32, scale) as usize, crate::canvas::pixels(h as i32, scale) as usize);
        self.panel.show(self.at, &pixels, size, scale);
    }

    /// The pill's centre, where the mote takes off.
    pub fn centre_physical(&self) -> (f32, f32) {
        let (x, y, w, h) = self.at;
        (x + w / 2.0, y + h / 2.0)
    }

    /// The pill's top centre: where a message goes when there's no caret.
    pub fn above_physical(&self) -> (f32, f32) {
        let (x, y, w, _) = self.at;
        (x + w / 2.0, y)
    }

    /// The view the menu is shown in.
    pub fn hwnd(&self) -> platform::Window {
        self.panel.view()
    }

    pub fn set(&mut self, state: OverlayState) {
        LIVE.store(!state.is_resting(), Ordering::Relaxed);
        self.pill.set(state);
        self.paint();
    }

    /// Re-paint the resting pill so it follows the screen you're working on.
    pub fn refresh_resting(&mut self) {
        if self.pill.state.is_resting() {
            self.paint();
        }
    }

    pub fn animate(&mut self) {
        let (x, y, w, h) = self.at;
        let (cx, cy) = platform::cursor();
        self.panel.set_clickable(cx >= x && cx < x + w && cy >= y && cy < y + h);
        let near = || {
            let pill = Rect { left: x as i32, top: y as i32, right: (x + w) as i32, bottom: (y + h) as i32 };
            nearness((cx as i32, cy as i32), pill)
        };
        if self.pill.step(false, near) {
            self.paint();
        }
    }

    pub fn set_quiet(&mut self, quiet: bool) {
        if self.pill.set_quiet(quiet) {
            self.paint();
        }
    }

    /// One soft pulse of the resting dot: your words landed.
    pub fn pulse(&mut self) {
        self.pill.pulse();
    }

    /// True once per right-click on the pill.
    pub fn take_right_click(&self) -> bool {
        platform::take_pill_right_click()
    }

    /// Hands AppKit its waiting events. Quitting goes through the menu, so this never ends the loop.
    pub fn pump_once(&mut self) -> bool {
        platform::pump();
        true
    }
}
