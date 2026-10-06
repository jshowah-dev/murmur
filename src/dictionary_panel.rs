//! The dictionary editor's content: term list, term editor, test box and save bar. Draws into
//! any `egui::Ui`, so a future settings window can host it as a tab.

use crate::correction_ui::{AMBER, GREEN, MUTED, TEXT};
use crate::editor_kit::{has_comments, open_file, progress, reduced_motion, CMD, RED};
use crate::motion;
use eframe::egui::{
    self, Align, Button, CentralPanel, Frame, Key, Layout, Margin, Modifiers, Panel, RichText, ScrollArea, Sense, TextEdit, TextStyle,
    UiBuilder,
};
use murmur_lib::dictionary::{file_stamp, Dictionary, SaveOutcome, Stamp, Term};
use murmur_lib::dictionary_edit::{self as edit, Deleted, Issue};
use std::collections::HashMap;
use std::path::PathBuf;

const LIST_W: f32 = 200.0;
const SOUND_ALIKE_TIP: &str = "Also catch words that sound like this term, e.g. 'haub' for HAWB. \
For all-caps acronyms only the single-word 'Heard as' forms are used.";

#[derive(Debug, PartialEq)]
enum Banner {
    Conflict,
    Error(String),
}

/// A field in the term editor that should take keyboard focus on its next draw.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Focus {
    Written,
    Spoken(usize),
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
    focus: Option<Focus>,
    /// A term just added: its written-form errors wait until the user leaves the field or saves.
    fresh: Option<usize>,
    /// The list order, held still while a written form is typed so its row doesn't jump.
    frozen: Option<Vec<usize>>,
    /// The re-sort after typing: each term's row before it, and when the slide began.
    settle: Option<(HashMap<usize, usize>, f64)>,
    /// Scroll the selected row into view on the next draw.
    reveal: bool,
    /// The spoken form whose rewrite is shown under "Heard as": (term, spoken form).
    proof_at: Option<(usize, usize)>,
    /// The rewrite last shown there, and when it first appeared.
    proof_shown: Option<(String, f64)>,
    reduced_motion: bool,
    /// Part of every text field's id, bumped when items change position. egui keeps a field's
    /// undo history under its id, so without this Ctrl+Z could bring back another item's text.
    fields: u32,
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
            focus: None,
            fresh: None,
            frozen: None,
            settle: None,
            reveal: false,
            proof_at: None,
            proof_shown: None,
            reduced_motion: reduced_motion(),
            fields: 0,
        };
        p.reload();
        p
    }

    fn reload(&mut self) {
        // an index can point at a different term in the reloaded file, so follow the name
        let keep = self.selected.and_then(|i| self.working.get(i)).map(|t| t.written.clone());
        match Dictionary::load_stamped(&self.path) {
            Ok((d, stamp)) => {
                self.loaded = d.terms.clone();
                self.working = d.terms;
                self.stamp = stamp;
                self.load_error = None;
                self.has_comments = has_comments(&self.path);
                self.selected = keep.and_then(|w| self.working.iter().position(|t| t.written == w));
                self.undo = None;
                self.banner = None;
                self.forget_indices();
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

    /// The working copy, for the editor window's tests.
    #[cfg(test)]
    pub(crate) fn edit(&mut self) -> &mut Vec<Term> {
        &mut self.working
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
        // a save attempt is the user asking what's wrong, so a new term's errors show now
        self.fresh = None;
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

    /// Adds a term and puts the cursor in its written form. A search that found nothing becomes
    /// the written form, so the word isn't typed twice. The new row waits at the bottom of the
    /// list until its name is typed.
    fn new_term(&mut self) {
        let query = self.search.trim();
        let written = if edit::visible(&self.working, query).is_empty() { query.to_string() } else { String::new() };
        let mut order = edit::visible(&self.working, "");
        self.working.push(Term { written, spoken: vec![], phonetic: true });
        let i = self.working.len() - 1;
        order.push(i);
        self.frozen = Some(order);
        self.settle = None;
        self.selected = Some(i);
        self.fresh = Some(i);
        self.focus = Some(Focus::Written);
        self.reveal = true;
        self.search.clear();
    }

    /// Drops state that holds term indices, for when the indices shift.
    fn forget_indices(&mut self) {
        self.fresh = None;
        self.frozen = None;
        self.settle = None;
        self.proof_at = None;
        self.fields += 1;
    }

    fn remove_spoken(&mut self, i: usize, k: usize) {
        self.working[i].spoken.remove(k);
        self.proof_at = None;
        self.fields += 1;
    }

    /// Selects term `i`. The old term's written field isn't drawn again, so it never reports
    /// losing focus: settle its row here.
    fn select(&mut self, i: usize, now: f64) {
        if self.selected != Some(i) {
            self.unfreeze(now);
            self.selected = Some(i);
            self.proof_at = None;
        }
    }

    fn delete_selected(&mut self) {
        if let Some(i) = self.selected.filter(|&i| i < self.working.len()) {
            self.undo = Some(edit::delete(&mut self.working, i));
            self.selected = None;
            self.forget_indices();
        }
    }

    fn undo_delete(&mut self) {
        if let Some(d) = self.undo.take() {
            self.forget_indices();
            self.selected = Some(edit::undo(&mut self.working, d));
            self.reveal = true;
        }
    }

    /// The written form stopped being edited: re-sort the list, sliding each row from where it
    /// was so the user sees where their term went.
    fn unfreeze(&mut self, now: f64) {
        if let Some(before) = self.frozen.take() {
            let before: HashMap<usize, usize> = before.into_iter().enumerate().map(|(row, i)| (i, row)).collect();
            self.settle = (!self.reduced_motion).then_some((before, now));
            self.reveal = true;
        }
        self.fresh = None;
    }

    /// The list rows, in order: held still while a written form is typed, else A–Z and filtered.
    fn rows(&self) -> Vec<usize> {
        match &self.frozen {
            Some(o) => o.iter().copied().filter(|&i| i < self.working.len()).collect(),
            None => edit::visible(&self.working, &self.search),
        }
    }

    /// What the dictionary makes of spoken form `k` of term `i`, while it's still unsaved.
    fn proof(&self, i: usize, k: usize) -> Option<(String, String)> {
        let heard = self.working.get(i)?.spoken.get(k)?.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
        if heard.is_empty() {
            return None;
        }
        let out = edit::preview(&self.working, &heard);
        Some((heard, out))
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
        if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::S)) {
            self.save();
        }
        let fresh = self.fresh;
        let issues: Vec<Issue> =
            edit::validate(&self.working).into_iter().filter(|x| !(Some(x.term) == fresh && x.spoken.is_none())).collect();
        // Panels, not nested rows: each takes its own share of the window, so the footer can't be
        // pushed off-screen by a long list. Order matters: top and bottom first, then the sides.
        let bare = |bottom: i8| Frame::new().inner_margin(Margin { left: 0, right: 0, top: 4, bottom });

        Panel::top("dict-toolbar").frame(bare(8)).resizable(false).show(ui, |ui| {
            ui.horizontal(|ui| {
                let search = ui.add(TextEdit::singleline(&mut self.search).hint_text("Search…").desired_width(LIST_W));
                if search.changed() {
                    self.frozen = None;
                    self.settle = None;
                }
                let enter = search.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                let nothing_found = !self.search.trim().is_empty() && edit::visible(&self.working, &self.search).is_empty();
                if ui.button("+ New term").clicked() || (enter && nothing_found) {
                    self.new_term();
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.link("Open dictionary file").clicked() {
                        open_file(&self.path);
                    }
                });
            });
        });

        Panel::bottom("dict-footer").frame(bare(0)).resizable(false).show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Test (dictionary only)").size(12.0).color(MUTED));
                ui.add(TextEdit::singleline(&mut self.test).hint_text("Type or dictate a phrase").desired_width(f32::INFINITY));
            });
            if !self.test.trim().is_empty() {
                ui.label(RichText::new(format!("→ {}", edit::preview(&self.working, &self.test))).color(GREEN));
            }
            ui.add_space(6.0);
            self.footer(ui);
        });

        Panel::left("dict-list")
            .frame(Frame::new().inner_margin(Margin { left: 0, right: 8, top: 6, bottom: 6 }))
            .default_size(LIST_W)
            .size_range(140.0..=360.0)
            .show(ui, |ui| {
                ScrollArea::vertical().id_salt("terms").auto_shrink([false, false]).show(ui, |ui| self.term_list(ui, &issues));
            });

        CentralPanel::no_frame().show(ui, |ui| {
            Frame::new().inner_margin(Margin { left: 12, right: 0, top: 6, bottom: 6 }).show(ui, |ui| {
                ScrollArea::vertical().id_salt("term").auto_shrink([false, false]).show(ui, |ui| {
                    self.term_editor(ui, &issues);
                });
            });
        });
    }

    fn term_list(&mut self, ui: &mut egui::Ui, issues: &[Issue]) {
        let rows = self.rows();
        if rows.is_empty() && !self.search.trim().is_empty() {
            ui.label(RichText::new("No term matches.").color(MUTED));
            if ui.button(format!("+ Add '{}'", self.search.trim())).on_hover_text("Or press Enter in Search").clicked() {
                self.new_term();
            }
            return;
        }
        let h = ui.spacing().interact_size.y;
        let pitch = h + ui.spacing().item_spacing.y;
        let t = self.settle.as_ref().map(|(_, since)| progress(ui.ctx(), *since, motion::duration::EMPHASIS, motion::easing::STANDARD));
        if t == Some(1.0) {
            self.settle = None;
        }
        let mut clicked = None;
        for (row, &i) in rows.iter().enumerate() {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), h), Sense::hover());
            if self.reveal && self.selected == Some(i) {
                ui.scroll_to_rect(rect, None);
            }
            // during the re-sort each row travels from its old row to its new one
            let from = match (&self.settle, t) {
                (Some((before, _)), Some(t)) => before.get(&i).map_or(0.0, |&b| (b as f32 - row as f32) * pitch * (1.0 - t)),
                _ => 0.0,
            };
            let term = &self.working[i];
            let name = if term.written.trim().is_empty() { "(new term)".to_string() } else { term.written.clone() };
            let bad = issues.iter().any(|x| x.error && x.term == i);
            let text = RichText::new(name).color(if bad { RED } else { TEXT });
            let selected = self.selected == Some(i);
            // a child Ui, so the shifted row doesn't move the list's own layout cursor
            let mut row_ui = ui.new_child(UiBuilder::new().max_rect(rect.translate(egui::vec2(0.0, from))).layout(*ui.layout()));
            if row_ui.selectable_label(selected, text).clicked() {
                clicked = Some(i);
            }
        }
        self.reveal = false;
        if let Some(i) = clicked {
            self.select(i, ui.input(|x| x.time));
        }
    }

    fn term_editor(&mut self, ui: &mut egui::Ui, issues: &[Issue]) {
        let Some(i) = self.selected.filter(|&i| i < self.working.len()) else {
            ui.label(RichText::new("Select a term, or add one with + New term.").color(MUTED));
            return;
        };
        let show = |ui: &mut egui::Ui, x: &Issue| {
            ui.label(RichText::new(&x.message).size(12.0).color(if x.error { RED } else { AMBER }));
        };
        let focus = self.focus.take();
        let fields = self.fields;
        let (enter, backspace) = ui.input(|x| (x.key_pressed(Key::Enter), x.key_pressed(Key::Backspace)));
        let now = ui.input(|x| x.time);
        let mut remove_spoken = None;
        let mut insert_spoken = None;
        let mut next_focus = None;
        let mut proof_at = self.proof_at;
        let mut delete = false;
        let t = &mut self.working[i];

        ui.label(RichText::new("Written as").size(12.0).color(MUTED));
        let written = ui.add(TextEdit::singleline(&mut t.written).id_salt(("written", fields, i)).desired_width(f32::INFINITY));
        if focus == Some(Focus::Written) {
            written.request_focus();
        }
        issues.iter().filter(|x| x.term == i && x.spoken.is_none()).for_each(|x| show(ui, x));
        ui.add_space(6.0);

        ui.label(RichText::new("Heard as").size(12.0).color(MUTED));
        ui.horizontal_wrapped(|ui| {
            for (k, s) in t.spoken.iter_mut().enumerate() {
                let was_empty = s.is_empty();
                let r = ui.add(TextEdit::singleline(s).id_salt(("spoken", fields, i, k)).desired_width(110.0));
                if focus == Some(Focus::Spoken(k)) {
                    r.request_focus();
                }
                if r.has_focus() {
                    proof_at = Some((i, k));
                }
                if r.has_focus() && was_empty && backspace {
                    // Backspace in an empty form removes it and steps back
                    remove_spoken = Some(k);
                    next_focus = Some(if k > 0 { Focus::Spoken(k - 1) } else { Focus::Written });
                } else if r.lost_focus() && enter && !s.trim().is_empty() {
                    // Enter after a form starts the next one
                    insert_spoken = Some(k + 1);
                    next_focus = Some(Focus::Spoken(k + 1));
                }
                if ui.small_button("×").on_hover_text("Remove").clicked() {
                    remove_spoken = Some(k);
                }
            }
            if ui.small_button("+").on_hover_text("Add a spoken form (or press Enter)").clicked() {
                insert_spoken = Some(t.spoken.len());
                next_focus = Some(Focus::Spoken(t.spoken.len()));
            }
        });
        if written.lost_focus() && enter {
            // Enter after the written form moves on to how it sounds
            if t.spoken.is_empty() {
                insert_spoken = Some(0);
            }
            next_focus = Some(Focus::Spoken(0));
        }
        let spoken_count = t.spoken.len();
        if spoken_count > 0 {
            self.proof_line(ui, proof_at.filter(|&(pi, k)| pi == i && k < spoken_count), now);
        }
        issues.iter().filter(|x| x.term == i && x.spoken.is_some()).for_each(|x| show(ui, x));
        ui.add_space(6.0);

        let t = &mut self.working[i];
        ui.checkbox(&mut t.phonetic, "Also match sound-alikes").on_hover_text(SOUND_ALIKE_TIP);
        ui.add_space(10.0);
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            if ui.button("Delete term").clicked() {
                delete = true;
            }
        });

        self.proof_at = proof_at;
        if written.gained_focus() && self.frozen.is_none() {
            self.frozen = Some(self.rows());
        }
        if written.lost_focus() {
            self.unfreeze(now);
        }
        if let Some(k) = remove_spoken {
            self.remove_spoken(i, k);
        }
        if let Some(k) = insert_spoken {
            self.working[i].spoken.insert(k, String::new());
            self.fields += 1;
        }
        if next_focus.is_some() {
            self.focus = next_focus;
            ui.ctx().request_repaint();
        }
        if delete {
            self.delete_selected();
        }
    }

    /// Under "Heard as": what the dictionary now makes of the spoken form being typed, e.g.
    /// `hob → HAWB`. When the rewrite changes, it slides in from the spoken form's side. The line
    /// keeps its space while empty, so typing the first letter doesn't push the rest down.
    fn proof_line(&mut self, ui: &mut egui::Ui, at: Option<(usize, usize)>, now: f64) {
        let Some((heard, out)) = at.and_then(|(i, k)| self.proof(i, k)) else {
            self.proof_shown = None;
            ui.horizontal(|ui| ui.label(" "));
            return;
        };
        let (result, color) = if out == heard { ("no change yet".to_string(), MUTED) } else { (out, GREEN) };
        if self.proof_shown.as_ref().map(|(s, _)| s) != Some(&result) {
            self.proof_shown = Some((result.clone(), now));
        }
        let since = self.proof_shown.as_ref().map_or(now, |(_, t)| *t);
        let e = if self.reduced_motion || color == MUTED {
            1.0
        } else {
            progress(ui.ctx(), since, motion::duration::EMPHASIS, motion::easing::ENTER)
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new(&heard).color(MUTED));
            let font = TextStyle::Body.resolve(ui.style());
            let galley = ui.painter().layout_no_wrap(format!("→  {result}"), font, color.gamma_multiply(e));
            let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
            let from = -motion::distance::ENTER_PX * (1.0 - e);
            ui.painter().galley(rect.min + egui::vec2(from, 0.0), galley, color);
        });
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
            if ui.add_enabled(can, Button::new("Save")).on_hover_text(format!("{CMD}S")).clicked() {
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
    use std::path::Path;

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
    fn reload_keeps_the_selected_term_by_written_form() {
        let p = temp_file("reselect", TWO);
        let mut panel = DictionaryPanel::new(p.clone());
        panel.selected = Some(1);
        std::fs::write(&p, "[[term]]\nwritten = \"BOL\"\n\n[[term]]\nwritten = \"New\"\n").unwrap();
        panel.reload();
        assert_eq!(panel.selected, Some(0));
    }

    #[test]
    fn reload_clears_the_selection_when_the_term_is_gone() {
        let p = temp_file("unselect", TWO);
        let mut panel = DictionaryPanel::new(p.clone());
        panel.selected = Some(1);
        std::fs::write(&p, "[[term]]\nwritten = \"A\"\n\n[[term]]\nwritten = \"B\"\n").unwrap();
        panel.reload();
        assert_eq!(panel.selected, None);
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

    /// Every piece of text the panel draws in one frame, with where it lands.
    fn drawn_text(panel: &mut DictionaryPanel, size: egui::Vec2) -> Vec<(String, egui::Pos2)> {
        fn walk(shape: &egui::epaint::Shape, out: &mut Vec<(String, egui::Pos2)>) {
            match shape {
                egui::epaint::Shape::Text(t) => out.push((t.galley.text().to_string(), t.pos)),
                egui::epaint::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let ctx = egui::Context::default();
        let input = || egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), ..Default::default() };
        let mut output = None;
        // panels settle their sizes over the first frames
        for _ in 0..3 {
            output = Some(ctx.run_ui(input(), |ui| panel.ui(ui)));
        }
        let mut out = Vec::new();
        output.unwrap().shapes.iter().for_each(|c| walk(&c.shape, &mut out));
        out
    }

    #[test]
    fn layout_fits_the_window_with_a_vertical_list() {
        let many = (0..30).map(|i| format!("[[term]]\nwritten = \"Term{i:02}\"\n")).collect::<String>();
        let p = temp_file("layout", &many);
        let mut panel = DictionaryPanel::new(p);
        panel.selected = Some(0);
        panel.delete_selected();
        panel.selected = Some(0);
        panel.test = "a phrase".into();
        // default window and the minimum window, less the editor's 12 px margin
        for size in [egui::vec2(736.0, 536.0), egui::vec2(536.0, 396.0)] {
            let text = drawn_text(&mut panel, size);
            let at = |s: &str| text.iter().find(|(t, _)| t == s).map(|(_, p)| *p).unwrap_or_else(|| panic!("'{s}' not drawn at {size:?}"));
            for s in ["Save", "Undo delete", "Test (dictionary only)", "→ a phrase", "Written as", "Also match sound-alikes"] {
                let p = at(s);
                assert!(p.x >= 0.0 && p.y >= 0.0 && p.x < size.x && p.y < size.y, "'{s}' off-screen at {p:?} in {size:?}");
            }
            let (a, b) = (at("Term01"), at("Term02"));
            assert!(b.y > a.y && (b.x - a.x).abs() < 1.0, "list not vertical: {a:?} {b:?}");
        }
    }

    #[test]
    fn new_term_takes_the_search_only_when_it_found_nothing() {
        let p = temp_file("prefill", TWO);
        let mut panel = DictionaryPanel::new(p);
        panel.search = " PRO ".into();
        panel.new_term();
        assert_eq!(panel.working[2].written, "PRO");
        assert!(panel.search.is_empty());
        panel.search = "hob".into();
        panel.new_term();
        assert_eq!(panel.working[3].written, "");
    }

    #[test]
    fn new_term_waits_at_the_bottom_then_settles_into_place() {
        let p = temp_file("settle", TWO);
        let mut panel = DictionaryPanel::new(p);
        panel.reduced_motion = false;
        panel.new_term();
        panel.working[2].written = "AAA".into();
        assert_eq!(panel.rows(), vec![1, 0, 2], "row moved while typing");
        panel.unfreeze(0.0);
        assert_eq!(panel.rows(), vec![2, 1, 0]);
        assert_eq!(panel.settle.as_ref().unwrap().0[&2], 2, "slide starts from the old row");
    }

    #[test]
    fn the_slide_starts_with_every_row_where_it_was() {
        let p = temp_file("slide-start", TWO);
        let mut panel = DictionaryPanel::new(p);
        panel.reduced_motion = false;
        panel.new_term();
        panel.working[2].written = "AAA".into();
        // a start time in the future holds the slide at its first frame
        panel.unfreeze(f64::INFINITY);
        let text = drawn_text(&mut panel, egui::vec2(736.0, 536.0));
        let y = |s: &str| text.iter().find(|(t, _)| t == s).unwrap_or_else(|| panic!("'{s}' not drawn")).1.y;
        let (bol, hawb, aaa) = (y("BOL"), y("HAWB"), y("AAA"));
        assert!(bol < hawb && hawb < aaa, "rows out of their old order: {bol} {hawb} {aaa}");
        assert!(((hawb - bol) - (aaa - hawb)).abs() < 0.5, "rows not evenly spaced: {bol} {hawb} {aaa}");
    }

    #[test]
    fn reduced_motion_resorts_without_a_slide() {
        let p = temp_file("reduced", TWO);
        let mut panel = DictionaryPanel::new(p);
        panel.reduced_motion = true;
        panel.new_term();
        panel.working[2].written = "AAA".into();
        panel.unfreeze(0.0);
        assert!(panel.settle.is_none());
        assert_eq!(panel.rows(), vec![2, 1, 0]);
    }

    #[test]
    fn typing_the_first_spoken_letter_moves_nothing_below() {
        let p = temp_file("no-jump", TWO);
        let mut panel = DictionaryPanel::new(p);
        panel.selected = Some(0);
        panel.working[0].spoken.push(String::new());
        panel.proof_at = Some((0, 1));
        let y = |panel: &mut DictionaryPanel| {
            drawn_text(panel, egui::vec2(736.0, 536.0)).into_iter().find(|(t, _)| t == "Also match sound-alikes").unwrap().1.y
        };
        let before = y(&mut panel);
        panel.working[0].spoken[1] = "zqx".into();
        assert_eq!(y(&mut panel), before);
    }

    #[test]
    fn selecting_another_term_ends_the_freeze() {
        let p = temp_file("select-unfreeze", TWO);
        let mut panel = DictionaryPanel::new(p);
        panel.new_term();
        panel.working[2].written = "AAA".into();
        panel.select(0, 0.0);
        assert!(panel.frozen.is_none());
        assert_eq!(panel.rows()[0], 2);
    }

    #[test]
    fn a_new_terms_empty_name_error_waits_for_a_save_attempt() {
        let p = temp_file("fresh", TWO);
        let mut panel = DictionaryPanel::new(p);
        panel.new_term();
        let has = |panel: &mut DictionaryPanel| drawn_text(panel, egui::vec2(736.0, 536.0)).iter().any(|(t, _)| t.contains("can't be empty"));
        assert!(!has(&mut panel));
        assert!(!panel.save());
        assert!(has(&mut panel));
    }

    fn key(key: Key) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }
    }

    /// Runs one frame with `events`, returning every piece of text drawn.
    fn frame(ctx: &egui::Context, panel: &mut DictionaryPanel, events: Vec<egui::Event>) -> Vec<String> {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(736.0, 536.0));
        let out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| panel.ui(ui));
        fn walk(shape: &egui::epaint::Shape, out: &mut Vec<String>) {
            match shape {
                egui::epaint::Shape::Text(t) => out.push(t.galley.text().to_string()),
                egui::epaint::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let mut text = Vec::new();
        out.shapes.iter().for_each(|c| walk(&c.shape, &mut text));
        text
    }

    #[test]
    fn a_term_can_be_typed_without_the_mouse_and_proves_itself() {
        let p = temp_file("keyboard", TWO);
        let mut panel = DictionaryPanel::new(p);
        let ctx = egui::Context::default();
        frame(&ctx, &mut panel, vec![]);
        panel.new_term();
        frame(&ctx, &mut panel, vec![]);
        frame(&ctx, &mut panel, vec![egui::Event::Text("PRO".into())]);
        assert_eq!(panel.working[2].written, "PRO");
        // Enter moves to a first spoken form
        frame(&ctx, &mut panel, vec![key(Key::Enter)]);
        frame(&ctx, &mut panel, vec![]);
        let text = frame(&ctx, &mut panel, vec![egui::Event::Text("pro number".into())]);
        assert_eq!(panel.working[2].spoken, vec!["pro number"]);
        let text = if text.iter().any(|t| t.contains("→  PRO")) { text } else { frame(&ctx, &mut panel, vec![]) };
        assert!(text.iter().any(|t| t == "→  PRO"), "no proof line in {text:?}");
        // Enter starts the next form; Backspace in it removes it again
        frame(&ctx, &mut panel, vec![key(Key::Enter)]);
        frame(&ctx, &mut panel, vec![]);
        assert_eq!(panel.working[2].spoken, vec!["pro number", ""]);
        frame(&ctx, &mut panel, vec![key(Key::Backspace)]);
        assert_eq!(panel.working[2].spoken, vec!["pro number"]);
    }

    fn undo_key() -> egui::Event {
        egui::Event::Key { key: Key::Z, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND }
    }

    #[test]
    fn undo_in_a_field_never_brings_back_a_deleted_terms_text() {
        let p = temp_file("undo-history", TWO);
        let mut panel = DictionaryPanel::new(p);
        let ctx = egui::Context::default();
        // the written field at index 0 remembers "HAWB" as its first undo point
        panel.selected = Some(0);
        panel.focus = Some(Focus::Written);
        frame(&ctx, &mut panel, vec![]);
        frame(&ctx, &mut panel, vec![]);
        panel.delete_selected();
        // BOL moves up to index 0
        panel.selected = Some(0);
        panel.focus = Some(Focus::Written);
        frame(&ctx, &mut panel, vec![]);
        frame(&ctx, &mut panel, vec![undo_key()]);
        assert_eq!(panel.working[0].written, "BOL");
    }

    #[test]
    fn undo_in_a_spoken_field_never_brings_back_a_removed_forms_text() {
        let p = temp_file("undo-spoken", TWO);
        let mut panel = DictionaryPanel::new(p);
        panel.working[0].spoken.push("haub".into());
        let ctx = egui::Context::default();
        // the spoken field at position 0 remembers "hob" as its first undo point
        panel.selected = Some(0);
        panel.focus = Some(Focus::Spoken(0));
        frame(&ctx, &mut panel, vec![]);
        frame(&ctx, &mut panel, vec![]);
        panel.remove_spoken(0, 0);
        // "haub" moves up to position 0
        panel.focus = Some(Focus::Spoken(0));
        frame(&ctx, &mut panel, vec![]);
        frame(&ctx, &mut panel, vec![undo_key()]);
        assert_eq!(panel.working[0].spoken, vec!["haub"]);
    }

    #[test]
    fn comments_are_detected() {
        let p = temp_file("comments", &format!("# mine\n{TWO}"));
        assert!(DictionaryPanel::new(p).has_comments);
    }
}
