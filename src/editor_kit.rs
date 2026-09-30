//! Pieces both editor tabs share.

use crate::motion;
use eframe::egui;
use std::path::Path;
use std::time::Duration;

pub(crate) const RED: egui::Color32 = egui::Color32::from_rgb(0xE0, 0x6C, 0x6C);

pub(crate) fn open_file(p: &Path) {
    let _ = std::process::Command::new("explorer.exe").arg(p).spawn();
}

/// Whether the file has a comment line, which saving from the editor would drop.
pub(crate) fn has_comments(p: &Path) -> bool {
    std::fs::read_to_string(p).map(|s| s.lines().any(|l| l.trim_start().starts_with('#'))).unwrap_or(false)
}

/// Windows' "Show animations in Windows" setting, off meaning reduced motion.
pub(crate) fn reduced_motion() -> bool {
    use windows::core::BOOL;
    use windows::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS};
    let mut on = BOOL(1);
    let ok = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, Some(&mut on as *mut BOOL as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)) };
    ok.is_ok() && !on.as_bool()
}

/// A cubic-bezier easing token evaluated at `x` in 0..=1.
pub(crate) fn ease([x1, y1, x2, y2]: [f32; 4], x: f32) -> f32 {
    let at = |a: f32, b: f32, t: f32| 3.0 * a * t * (1.0 - t).powi(2) + 3.0 * b * t * t * (1.0 - t) + t.powi(3);
    // x(t) rises monotonically for easing curves, so bisect for the t that gives x
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..20 {
        let m = (lo + hi) / 2.0;
        if at(x1, x2, m) < x {
            lo = m;
        } else {
            hi = m;
        }
    }
    at(y1, y2, (lo + hi) / 2.0)
}

/// How far (0..=1, eased) a motion that began at `since` has run; repaints until it's done.
pub(crate) fn progress(ctx: &egui::Context, since: f64, d: Duration, curve: [f32; 4]) -> f32 {
    let x = ((ctx.input(|i| i.time) - since) as f32 / motion::scaled(d).as_secs_f32()).clamp(0.0, 1.0);
    if x < 1.0 {
        ctx.request_repaint();
    }
    ease(curve, x)
}
