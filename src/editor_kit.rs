//! Pieces both editor tabs share.

use eframe::egui;
use std::path::Path;

pub(crate) const RED: egui::Color32 = egui::Color32::from_rgb(0xE0, 0x6C, 0x6C);

pub(crate) fn open_file(p: &Path) {
    let _ = std::process::Command::new("explorer.exe").arg(p).spawn();
}

/// Whether the file has a comment line, which saving from the editor would drop.
pub(crate) fn has_comments(p: &Path) -> bool {
    std::fs::read_to_string(p).map(|s| s.lines().any(|l| l.trim_start().starts_with('#'))).unwrap_or(false)
}
