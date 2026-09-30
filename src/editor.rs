//! `murmur.exe --dictionary` / `--snippets`: the dictionary and snippets editor as its own
//! process, so dictation keeps working while it's open. One editor at a time; a second launch
//! brings the first forward.

use crate::correction_ui::{load_system_font, BG};
use crate::dictionary_panel::DictionaryPanel;
use crate::snippets_panel::SnippetsPanel;
use anyhow::Result;
use eframe::egui::{self, Frame, Id, Margin, Modal, Panel, RichText, ViewportCommand};
use std::path::PathBuf;
use std::sync::Arc;
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Threading::{CreateMutexW, OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, IsIconic, ShowWindow, SW_RESTORE};

const TITLE: &str = "Murmur — Dictionary & Snippets";
// unchanged from the dictionary-only editor, so an update still sees an editor left open by it
const MUTEX: &str = "Local\\Murmur.DictionaryEditor";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Dictionary,
    Snippets,
}

impl Tab {
    const ALL: [Tab; 2] = [Tab::Dictionary, Tab::Snippets];

    /// The command-line flag that opens the editor on this tab.
    pub fn flag(self) -> &'static str {
        match self {
            Tab::Dictionary => "--dictionary",
            Tab::Snippets => "--snippets",
        }
    }

    pub fn from_flag(s: &str) -> Option<Tab> {
        Tab::ALL.into_iter().find(|t| t.flag() == s)
    }

    fn label(self) -> &'static str {
        match self {
            Tab::Dictionary => "Dictionary",
            Tab::Snippets => "Snippets",
        }
    }
}

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
    dictionary: DictionaryPanel,
    snippets: SnippetsPanel,
    tab: Tab,
    asking: bool,
    closing: bool,
}

impl eframe::App for EditorApp {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        self.draw(ui);
    }
}

impl EditorApp {
    fn new(dictionary: PathBuf, snippets: PathBuf, tab: Tab) -> Self {
        EditorApp { dictionary: DictionaryPanel::new(dictionary), snippets: SnippetsPanel::new(snippets), tab, asking: false, closing: false }
    }

    fn is_dirty(&self, tab: Tab) -> bool {
        match tab {
            Tab::Dictionary => self.dictionary.is_dirty(),
            Tab::Snippets => self.snippets.is_dirty(),
        }
    }

    /// Saves every tab with edits. If one fails (its banner says why), shows it and returns false.
    fn save_all(&mut self) -> bool {
        let dictionary = self.dictionary.save();
        let snippets = self.snippets.save();
        if !dictionary {
            self.tab = Tab::Dictionary;
        } else if !snippets {
            self.tab = Tab::Snippets;
        }
        dictionary && snippets
    }

    fn question(&self) -> &'static str {
        match (self.is_dirty(Tab::Dictionary), self.is_dirty(Tab::Snippets)) {
            (true, true) => "Save changes to the dictionary and snippets?",
            (false, true) => "Save changes to the snippets?",
            _ => "Save changes to the dictionary?",
        }
    }

    /// The whole window, apart from eframe itself, so tests can drive it.
    fn draw(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let dirty = Tab::ALL.into_iter().any(|t| self.is_dirty(t));
        if ctx.input(|i| i.viewport().close_requested()) && !self.closing && dirty {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.asking = true;
        }
        Frame::new().fill(BG).inner_margin(Margin::same(12)).show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            // a panel, so the tab's own panels share out only the space below it
            let strip = Frame::new().inner_margin(Margin { left: 0, right: 0, top: 0, bottom: 6 });
            Panel::top("editor-tabs").frame(strip).resizable(false).show(ui, |ui| {
                ui.horizontal(|ui| {
                    for tab in Tab::ALL {
                        let label = if self.is_dirty(tab) { format!("{} •", tab.label()) } else { tab.label().to_string() };
                        if ui.selectable_label(self.tab == tab, RichText::new(label).size(15.0)).clicked() {
                            self.tab = tab;
                        }
                    }
                });
            });
            match self.tab {
                Tab::Dictionary => self.dictionary.ui(ui),
                Tab::Snippets => self.snippets.ui(ui),
            }
        });
        self.prompt(&ctx);
    }

    fn close(&mut self, ctx: &egui::Context) {
        self.closing = true;
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }

    /// The unsaved-changes prompt, shown while `asking`.
    fn prompt(&mut self, ctx: &egui::Context) {
        if !self.asking {
            return;
        }
        let question = self.question();
        let mut choice = None;
        let modal = Modal::new(Id::new("unsaved-changes")).show(ctx, |ui| {
            ui.label(question);
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
                // a failed save (invalid items, conflict, I/O) keeps the window open on that tab
                if self.save_all() {
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

pub fn run(tab: Tab) -> Result<()> {
    log::info!("editor starting on {tab:?}");
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
    let app = EditorApp::new(murmur_lib::dictionary::path(), murmur_lib::snippets::path(), tab);
    eframe::run_native(
        "murmur-dictionary",
        opts,
        Box::new(move |cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            load_system_font(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow::anyhow!("editor window: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use murmur_lib::dictionary::Term;
    use murmur_lib::snippets::Snippet;

    fn app(name: &str, tab: Tab) -> EditorApp {
        let dir = std::env::temp_dir().join(format!("murmur-editor-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        EditorApp::new(dir.join("dictionary.toml"), dir.join("snippets.toml"), tab)
    }

    /// Runs one frame of the whole window, returning every piece of text drawn and where.
    fn frame(ctx: &egui::Context, app: &mut EditorApp, size: egui::Vec2, events: Vec<egui::Event>, close: bool) -> Vec<(String, egui::Pos2)> {
        fn walk(shape: &egui::epaint::Shape, out: &mut Vec<(String, egui::Pos2)>) {
            match shape {
                egui::epaint::Shape::Text(t) => out.push((t.galley.text().to_string(), t.pos)),
                egui::epaint::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let mut input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), events, ..Default::default() };
        if close {
            input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap().events.push(egui::ViewportEvent::Close);
        }
        let out = ctx.run_ui(input, |ui| app.draw(ui));
        let mut text = Vec::new();
        out.shapes.iter().for_each(|c| walk(&c.shape, &mut text));
        text
    }

    const MIN: egui::Vec2 = egui::vec2(560.0, 420.0);

    #[test]
    fn tab_flags_round_trip() {
        for tab in [Tab::Dictionary, Tab::Snippets] {
            assert_eq!(Tab::from_flag(tab.flag()), Some(tab));
        }
        assert_eq!(Tab::from_flag("--snippets"), Some(Tab::Snippets));
        assert_eq!(Tab::from_flag("--other"), None);
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
        let mut app = app("esc", Tab::Dictionary);
        app.asking = true;
        frame(&ctx, &mut app, MIN, vec![], false);
        let esc = egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        frame(&ctx, &mut app, MIN, vec![esc], false);
        assert!(!app.asking);
        assert!(!app.closing);
    }

    #[test]
    fn closing_with_only_the_hidden_tab_dirty_asks() {
        let ctx = egui::Context::default();
        let mut app = app("hidden-dirty", Tab::Dictionary);
        app.snippets.edit().push(Snippet { trigger: "sig".into(), text: "x".into() });
        frame(&ctx, &mut app, MIN, vec![], true);
        assert!(app.asking);
        assert_eq!(app.question(), "Save changes to the snippets?");
    }

    #[test]
    fn closing_with_nothing_dirty_does_not_ask() {
        let ctx = egui::Context::default();
        let mut app = app("clean", Tab::Snippets);
        frame(&ctx, &mut app, MIN, vec![], true);
        assert!(!app.asking);
    }

    #[test]
    fn save_all_writes_every_valid_tab_and_shows_the_failing_one() {
        let mut app = app("save-all", Tab::Dictionary);
        app.dictionary.edit().push(Term { written: "HAWB".into(), spoken: vec!["hob".into()], phonetic: true });
        app.snippets.edit().push(Snippet { trigger: "".into(), text: "x".into() });
        assert_eq!(app.question(), "Save changes to the dictionary and snippets?");
        assert!(!app.save_all());
        assert_eq!(app.tab, Tab::Snippets);
        assert!(!app.dictionary.is_dirty(), "the valid tab was still saved");
        app.snippets.edit()[0].trigger = "sig".into();
        assert!(app.save_all());
    }

    #[test]
    fn both_tabs_fit_the_minimum_window() {
        for tab in [Tab::Dictionary, Tab::Snippets] {
            let ctx = egui::Context::default();
            let mut app = app("fit", tab);
            let text = (0..3).map(|_| frame(&ctx, &mut app, MIN, vec![], false)).last().unwrap();
            for s in ["Dictionary", "Snippets", "Save"] {
                let p = text.iter().find(|(t, _)| t == s).unwrap_or_else(|| panic!("'{s}' not drawn on {tab:?}")).1;
                assert!(p.x >= 0.0 && p.y >= 0.0 && p.x < MIN.x && p.y < MIN.y, "'{s}' off-screen at {p:?} on {tab:?}");
            }
        }
    }

    #[test]
    fn a_tab_with_edits_shows_a_dot() {
        let ctx = egui::Context::default();
        let mut app = app("dot", Tab::Dictionary);
        app.snippets.edit().push(Snippet { trigger: "sig".into(), text: "x".into() });
        let text = frame(&ctx, &mut app, MIN, vec![], false);
        assert!(text.iter().any(|(t, _)| t == "Snippets •"));
        assert!(text.iter().any(|(t, _)| t == "Dictionary"));
    }
}
