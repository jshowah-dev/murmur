//! The dictionary editor's content: term list, term editor, test box and save bar. Draws into
//! any `egui::Ui`, so a future settings window can host it as a tab.

use crate::correction_ui::{AMBER, GREEN, MUTED, TEXT};
use eframe::egui::{self, Align, Button, Key, Layout, Modifiers, RichText, ScrollArea, TextEdit};
use murmur_lib::dictionary::{file_stamp, Dictionary, SaveOutcome, Stamp, Term};
use murmur_lib::dictionary_edit::{self as edit, Deleted, Issue};
use std::path::{Path, PathBuf};

const RED: egui::Color32 = egui::Color32::from_rgb(0xE0, 0x6C, 0x6C);
const LIST_W: f32 = 200.0;
const FOOTER_H: f32 = 150.0;
const SOUND_ALIKE_TIP: &str = "Also catch words that sound like this term, e.g. 'haub' for HAWB. \
For all-caps acronyms only the single-word 'Heard as' forms are used.";

#[derive(Debug, PartialEq)]
enum Banner {
    Conflict,
    Error(String),
}

pub struct DictionaryPanel {
    path: PathBuf,
    /// Set when the file couldn't be read; the panel then only offers Retry and Open file.
    load_error: Option<String>,
    loaded: Vec<Term>,
    working: Vec<Term>,
    stamp: Stamp,
    has_comments: bool,
    selected: Option<usize>,
    search: String,
    test: String,
    undo: Option<Deleted>,
    banner: Option<Banner>,
}

fn open_file(p: &Path) {
    let _ = std::process::Command::new("explorer.exe").arg(p).spawn();
}

impl DictionaryPanel {
    pub fn new(path: PathBuf) -> Self {
        let mut p = DictionaryPanel {
            path,
            load_error: None,
            loaded: vec![],
            working: vec![],
            stamp: None,
            has_comments: false,
            selected: None,
            search: String::new(),
            test: String::new(),
            undo: None,
            banner: None,
        };
        p.reload();
        p
    }

    fn reload(&mut self) {
        match Dictionary::load_stamped(&self.path) {
            Ok((d, stamp)) => {
                self.loaded = d.terms.clone();
                self.working = d.terms;
                self.stamp = stamp;
                self.load_error = None;
                self.has_comments = std::fs::read_to_string(&self.path)
                    .map(|s| s.lines().any(|l| l.trim_start().starts_with('#')))
                    .unwrap_or(false);
                self.selected = self.selected.filter(|&i| i < self.working.len());
                self.undo = None;
                self.banner = None;
            }
            Err(e) => {
                log::error!("dictionary editor: {e:#}");
                self.load_error = Some(format!("{e:#}"));
            }
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.load_error.is_none() && self.working != self.loaded
    }

    fn has_errors(&self) -> bool {
        edit::validate(&self.working).iter().any(|i| i.error)
    }

    fn saved(&mut self, terms: Vec<Term>, stamp: Stamp) {
        self.loaded = terms.clone();
        self.working = terms;
        self.stamp = stamp;
        self.has_comments = false;
        self.undo = None;
        self.banner = None;
    }

    /// Blocks a write while any term has an error, and says why.
    fn refuse_invalid(&mut self) -> bool {
        if self.has_errors() {
            self.banner = Some(Banner::Error("fix the items marked in red first".into()));
            return true;
        }
        false
    }

    /// Saves if valid and the file is unchanged on disk. Returns true when the file holds the
    /// editor's terms. With nothing changed it writes nothing, so `.bak` keeps the older version.
    pub fn save(&mut self) -> bool {
        if self.load_error.is_some() {
            return false;
        }
        if !self.is_dirty() {
            return true;
        }
        if self.refuse_invalid() {
            return false;
        }
        let d = Dictionary::from_terms(edit::normalize(self.working.clone()));
        match d.save_if_unchanged(&self.path, self.stamp) {
            Ok(SaveOutcome::Saved(stamp)) => {
                self.saved(d.terms, stamp);
                true
            }
            Ok(SaveOutcome::Conflict) => {
                self.banner = Some(Banner::Conflict);
                false
            }
            Err(e) => {
                log::error!("dictionary editor save: {e:#}");
                self.banner = Some(Banner::Error(format!("{e:#}")));
                false
            }
        }
    }

    fn overwrite(&mut self) {
        if self.refuse_invalid() {
            return;
        }
        let d = Dictionary::from_terms(edit::normalize(self.working.clone()));
        match d.save_to(&self.path) {
            Ok(()) => {
                let stamp = file_stamp(&self.path);
                self.saved(d.terms, stamp);
            }
            Err(e) => self.banner = Some(Banner::Error(format!("{e:#}"))),
        }
    }

    fn new_term(&mut self) {
        self.working.push(Term { written: String::new(), spoken: vec![], phonetic: true });
        self.selected = Some(self.working.len() - 1);
        self.search.clear();
    }

    fn delete_selected(&mut self) {
        if let Some(i) = self.selected.filter(|&i| i < self.working.len()) {
            self.undo = Some(edit::delete(&mut self.working, i));
            self.selected = None;
        }
    }

    fn undo_delete(&mut self) {
        if let Some(d) = self.undo.take() {
            self.selected = Some(edit::undo(&mut self.working, d));
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        if let Some(err) = self.load_error.clone() {
            ui.label(RichText::new("dictionary.toml couldn't be read").size(15.0).color(TEXT));
            ui.label(RichText::new(err).color(RED));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Retry").clicked() {
                    self.reload();
                }
                if ui.link("Open dictionary file").clicked() {
                    open_file(&self.path);
                }
            });
            return;
        }
        if ui.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::S)) {
            self.save();
        }
        let issues = edit::validate(&self.working);

        ui.horizontal(|ui| {
            ui.add(TextEdit::singleline(&mut self.search).hint_text("Search…").desired_width(LIST_W));
            if ui.button("+ New term").clicked() {
                self.new_term();
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.link("Open dictionary file").clicked() {
                    open_file(&self.path);
                }
            });
        });
        ui.separator();

        let list_h = (ui.available_height() - FOOTER_H).max(120.0);
        ui.horizontal_top(|ui| {
            ui.allocate_ui(egui::vec2(LIST_W, list_h), |ui| {
                ScrollArea::vertical().id_salt("terms").max_height(list_h).auto_shrink([false, false]).show(ui, |ui| {
                    ui.set_width(LIST_W);
                    for i in edit::visible(&self.working, &self.search) {
                        let t = &self.working[i];
                        let name = if t.written.trim().is_empty() { "(new term)".to_string() } else { t.written.clone() };
                        let bad = issues.iter().any(|x| x.error && x.term == i);
                        let text = RichText::new(name).color(if bad { RED } else { TEXT });
                        if ui.selectable_label(self.selected == Some(i), text).clicked() {
                            self.selected = Some(i);
                        }
                    }
                });
            });
            ui.separator();
            ui.vertical(|ui| {
                ui.set_min_height(list_h);
                self.term_editor(ui, &issues);
            });
        });
        ui.separator();

        ui.horizontal(|ui| {
            ui.label(RichText::new("Test (dictionary only)").size(12.0).color(MUTED));
            ui.add(TextEdit::singleline(&mut self.test).hint_text("Type or dictate a phrase").desired_width(f32::INFINITY));
        });
        if !self.test.trim().is_empty() {
            ui.label(RichText::new(format!("→ {}", edit::preview(&self.working, &self.test))).color(GREEN));
        }
        ui.add_space(6.0);
        self.footer(ui);
    }

    fn term_editor(&mut self, ui: &mut egui::Ui, issues: &[Issue]) {
        let Some(i) = self.selected.filter(|&i| i < self.working.len()) else {
            ui.label(RichText::new("Select a term, or add one with + New term.").color(MUTED));
            return;
        };
        let show = |ui: &mut egui::Ui, x: &Issue| {
            ui.label(RichText::new(&x.message).size(12.0).color(if x.error { RED } else { AMBER }));
        };
        let mut remove_spoken = None;
        let mut delete = false;
        let t = &mut self.working[i];

        ui.label(RichText::new("Written as").size(12.0).color(MUTED));
        ui.add(TextEdit::singleline(&mut t.written).desired_width(f32::INFINITY));
        issues.iter().filter(|x| x.term == i && x.spoken.is_none()).for_each(|x| show(ui, x));
        ui.add_space(6.0);

        ui.label(RichText::new("Heard as").size(12.0).color(MUTED));
        ui.horizontal_wrapped(|ui| {
            for (k, s) in t.spoken.iter_mut().enumerate() {
                ui.add(TextEdit::singleline(s).desired_width(110.0));
                if ui.small_button("×").on_hover_text("Remove").clicked() {
                    remove_spoken = Some(k);
                }
            }
            if ui.small_button("+").on_hover_text("Add a spoken form").clicked() {
                t.spoken.push(String::new());
            }
        });
        issues.iter().filter(|x| x.term == i && x.spoken.is_some()).for_each(|x| show(ui, x));
        ui.add_space(6.0);

        ui.checkbox(&mut t.phonetic, "Also match sound-alikes").on_hover_text(SOUND_ALIKE_TIP);
        ui.add_space(10.0);
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            if ui.button("Delete term").clicked() {
                delete = true;
            }
        });

        if let Some(k) = remove_spoken {
            self.working[i].spoken.remove(k);
        }
        if delete {
            self.delete_selected();
        }
    }

    fn footer(&mut self, ui: &mut egui::Ui) {
        let mut reload = false;
        let mut overwrite = false;
        match &self.banner {
            Some(Banner::Conflict) => {
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(AMBER, "dictionary.toml changed outside the editor (Fix-last or a hand edit).");
                    reload = ui.button("Reload").on_hover_text("Discard my edits").clicked();
                    overwrite = ui.button("Overwrite").clicked();
                });
            }
            Some(Banner::Error(e)) => {
                ui.colored_label(RED, format!("Couldn't save: {e}"));
            }
            None => {}
        }
        if self.has_comments {
            ui.label(RichText::new("Saving removes comments from dictionary.toml").size(12.0).color(MUTED));
        }
        let n = edit::changes(&self.loaded, &self.working);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let can = self.is_dirty() && !self.has_errors();
            if ui.add_enabled(can, Button::new("Save")).on_hover_text("Ctrl+S").clicked() {
                self.save();
            }
            if n > 0 {
                ui.label(RichText::new(format!("{n} unsaved change{}", if n == 1 { "" } else { "s" })).color(MUTED));
            }
            if self.undo.is_some() && ui.button("Undo delete").clicked() {
                self.undo_delete();
            }
        });
        if reload {
            self.reload();
        }
        if overwrite {
            self.overwrite();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str, contents: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("murmur-panel-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("dictionary.toml");
        std::fs::write(&p, contents).unwrap();
        p
    }

    const TWO: &str = "[[term]]\nwritten = \"HAWB\"\nspoken = [\"hob\"]\n\n[[term]]\nwritten = \"BOL\"\nspoken = [\"bee oh el\"]\n";

    fn on_disk(p: &Path) -> Vec<Term> {
        Dictionary::load_stamped(p).unwrap().0.terms
    }

    #[test]
    fn edit_and_save_writes_normalized_terms() {
        let p = temp_file("save", TWO);
        let mut panel = DictionaryPanel::new(p.clone());
        panel.working[0].spoken.push(" Haub ".into());
        assert!(panel.is_dirty());
        assert!(panel.save());
        assert!(!panel.is_dirty());
        assert_eq!(on_disk(&p)[0].spoken, vec!["hob", "haub"]);
    }

    #[test]
    fn second_save_is_not_a_conflict() {
        let p = temp_file("twice", TWO);
        let mut panel = DictionaryPanel::new(p.clone());
        panel.working[0].written = "HAWBX".into();
        assert!(panel.save());
        std::thread::sleep(std::time::Duration::from_millis(20));
        panel.working[0].written = "HAWB".into();
        assert!(panel.save());
        assert_eq!(panel.banner, None);
    }

    #[test]
    fn outside_write_gives_conflict_then_overwrite_wins() {
        let p = temp_file("conflict", TWO);
        let mut panel = DictionaryPanel::new(p.clone());
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&p, "[[term]]\nwritten = \"Outside\"\n").unwrap();
        panel.working[1].written = "BOLX".into();
        assert!(!panel.save());
        assert_eq!(panel.banner, Some(Banner::Conflict));
        assert_eq!(on_disk(&p)[0].written, "Outside");
        panel.overwrite();
        assert_eq!(panel.banner, None);
        assert_eq!(on_disk(&p)[1].written, "BOLX");
    }

    #[test]
    fn conflict_then_reload_discards_edits() {
        let p = temp_file("reload", TWO);
        let mut panel = DictionaryPanel::new(p.clone());
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&p, "[[term]]\nwritten = \"Outside\"\n").unwrap();
        panel.working[0].written = "Mine".into();
        assert!(!panel.save());
        panel.reload();
        assert_eq!(panel.working.len(), 1);
        assert_eq!(panel.working[0].written, "Outside");
        assert!(!panel.is_dirty());
    }

    #[test]
    fn invalid_terms_block_save_and_leave_file_alone() {
        let p = temp_file("invalid", TWO);
        let mut panel = DictionaryPanel::new(p.clone());
        panel.new_term();
        assert!(!panel.save());
        assert!(matches!(panel.banner, Some(Banner::Error(_))));
        assert_eq!(on_disk(&p).len(), 2);
    }

    #[test]
    fn broken_file_shows_error_and_never_saves() {
        let p = temp_file("broken", "[[term]\nbroken");
        let mut panel = DictionaryPanel::new(p.clone());
        assert!(panel.load_error.is_some());
        assert!(!panel.is_dirty());
        assert!(!panel.save());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "[[term]\nbroken");
    }

    #[test]
    fn delete_undo_and_save_clears_undo() {
        let p = temp_file("undo", TWO);
        let mut panel = DictionaryPanel::new(p.clone());
        panel.selected = Some(0);
        panel.delete_selected();
        assert_eq!(panel.working.len(), 1);
        panel.undo_delete();
        assert_eq!(panel.working[0].written, "HAWB");
        assert_eq!(panel.selected, Some(0));
        panel.selected = Some(1);
        panel.delete_selected();
        assert!(panel.save());
        assert!(panel.undo.is_none());
        assert_eq!(on_disk(&p).len(), 1);
    }

    #[test]
    fn overwrite_refuses_invalid_terms() {
        let p = temp_file("overwrite-invalid", TWO);
        let mut panel = DictionaryPanel::new(p.clone());
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&p, "[[term]]\nwritten = \"Outside\"\n").unwrap();
        panel.working[0].written = "Mine".into();
        assert!(!panel.save());
        panel.new_term();
        panel.overwrite();
        assert!(matches!(panel.banner, Some(Banner::Error(_))));
        assert_eq!(on_disk(&p)[0].written, "Outside");
    }

    #[test]
    fn save_without_changes_leaves_file_and_backup_alone() {
        let original = format!("# mine\n{TWO}");
        let p = temp_file("clean-save", &original);
        let mut panel = DictionaryPanel::new(p.clone());
        assert!(panel.save());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), original);
        assert!(!p.with_extension("toml.bak").exists());
    }

    #[test]
    fn comments_are_detected() {
        let p = temp_file("comments", &format!("# mine\n{TWO}"));
        assert!(DictionaryPanel::new(p).has_comments);
    }
}
