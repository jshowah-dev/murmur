//! `murmur.exe --dictionary`: the dictionary editor as its own process, so dictation keeps
//! working while it's open. One editor at a time; a second launch brings the first forward.

use crate::correction_ui::{load_system_font, BG};
use crate::dictionary_panel::DictionaryPanel;
use anyhow::Result;
use eframe::egui::{self, Frame, Id, Margin, Modal, ViewportCommand};
use std::sync::Arc;
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Threading::{CreateMutexW, OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, IsIconic, ShowWindow, SW_RESTORE};

const TITLE: &str = "Murmur — Dictionary";
const MUTEX: &str = "Local\\Murmur.DictionaryEditor";

fn mutex_exists(name: &str) -> bool {
    match unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, &HSTRING::from(name)) } {
        Ok(h) => {
            let _ = unsafe { CloseHandle(h) };
            true
        }
        Err(_) => false,
    }
}

/// Whether an editor process is running, from outside it: an update would force-close it.
pub fn is_open() -> bool {
    mutex_exists(MUTEX)
}

enum CloseChoice {
    Save,
    Discard,
    Cancel,
}

struct EditorApp {
    panel: DictionaryPanel,
    asking: bool,
    closing: bool,
}

impl eframe::App for EditorApp {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.viewport().close_requested()) && !self.closing && self.panel.is_dirty() {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.asking = true;
        }
        Frame::new().fill(BG).inner_margin(Margin::same(12)).show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            self.panel.ui(ui);
        });
        self.prompt(&ctx);
    }
}

impl EditorApp {
    fn close(&mut self, ctx: &egui::Context) {
        self.closing = true;
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }

    /// The unsaved-changes prompt, shown while `asking`.
    fn prompt(&mut self, ctx: &egui::Context) {
        if !self.asking {
            return;
        }
        let mut choice = None;
        let modal = Modal::new(Id::new("unsaved-changes")).show(ctx, |ui| {
            ui.label("Save changes to the dictionary?");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    choice = Some(CloseChoice::Save);
                }
                if ui.button("Discard").clicked() {
                    choice = Some(CloseChoice::Discard);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(CloseChoice::Cancel);
                }
            });
        });
        // Esc or a click outside the prompt
        if choice.is_none() && modal.should_close() {
            choice = Some(CloseChoice::Cancel);
        }
        match choice {
            Some(CloseChoice::Save) => {
                self.asking = false;
                // a failed save (invalid terms, conflict, I/O) keeps the window open with its banner
                if self.panel.save() {
                    self.close(ctx);
                }
            }
            Some(CloseChoice::Discard) => {
                self.asking = false;
                self.close(ctx);
            }
            Some(CloseChoice::Cancel) => self.asking = false,
            None => {}
        }
    }
}

/// Brings an already-open editor to the front. If its window isn't up yet, does nothing.
fn focus_existing() {
    unsafe {
        if let Ok(hwnd) = FindWindowW(PCWSTR::null(), &HSTRING::from(TITLE)) {
            // SW_RESTORE would also un-maximize, so only use it on a minimized window
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            crate::correction::bring_to_front(hwnd);
        }
    }
}

pub fn run() -> Result<()> {
    log::info!("dictionary editor starting");
    let _instance = unsafe {
        let m = CreateMutexW(None, false, &HSTRING::from(MUTEX))?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            focus_existing();
            return Ok(());
        }
        m
    };
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/murmur.png")).unwrap_or_default();
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(TITLE)
            .with_icon(Arc::new(icon))
            .with_inner_size([760.0, 560.0])
            .with_min_inner_size([560.0, 420.0]),
        centered: true,
        ..Default::default()
    };
    let panel = DictionaryPanel::new(murmur_lib::dictionary::path());
    eframe::run_native(
        "murmur-dictionary",
        opts,
        Box::new(move |cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            load_system_font(&cc.egui_ctx);
            Ok(Box::new(EditorApp { panel, asking: false, closing: false }))
        }),
    )
    .map_err(|e| anyhow::anyhow!("dictionary editor window: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asking_app() -> EditorApp {
        let path = std::env::temp_dir().join(format!("murmur-editor-{}", std::process::id())).join("dictionary.toml");
        EditorApp { panel: DictionaryPanel::new(path), asking: true, closing: false }
    }

    fn frame(ctx: &egui::Context, app: &mut EditorApp, events: Vec<egui::Event>) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(760.0, 560.0))),
            events,
            ..Default::default()
        };
        let _ = ctx.run_ui(input, |ui| app.prompt(ui.ctx()));
    }

    #[test]
    fn a_held_mutex_is_seen_from_outside() {
        let name = format!("Local\\Murmur.Test.{}", std::process::id());
        assert!(!mutex_exists(&name));
        let held = unsafe { CreateMutexW(None, false, &HSTRING::from(name.as_str())) }.unwrap();
        assert!(mutex_exists(&name));
        unsafe { CloseHandle(held) }.unwrap();
        assert!(!mutex_exists(&name));
    }

    #[test]
    fn escape_dismisses_the_prompt_like_cancel() {
        let ctx = egui::Context::default();
        let mut app = asking_app();
        frame(&ctx, &mut app, vec![]);
        let esc = egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        frame(&ctx, &mut app, vec![esc]);
        assert!(!app.asking);
        assert!(!app.closing);
    }
}
