//! The snippets editor's content: snippet list, snippet editor, test box and save bar. Draws
//! into any `egui::Ui`; the editor window hosts it as a tab beside the dictionary.

use crate::correction_ui::{AMBER, GREEN, MUTED, TEXT};
use crate::editor_kit::{has_comments, open_file, progress, reduced_motion, RED};
use crate::motion;
use eframe::egui::{
    self, Align, Button, CentralPanel, Frame, Key, Layout, Margin, Modifiers, Panel, RichText, ScrollArea, Sense, TextEdit, UiBuilder,
};
use murmur_lib::config::{file_stamp, SaveOutcome, Stamp};
use murmur_lib::snippets::{Snippet, Snippets};
use murmur_lib::snippets_edit::{self as edit, Deleted, Field, Issue};
use std::collections::HashMap;
use std::path::PathBuf;

const LIST_W: f32 = 200.0;

#[derive(Debug, PartialEq)]
enum Banner {
    Conflict,
    Error(String),
}

/// A field in the snippet editor that should take keyboard focus on its next draw.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Focus {
    Trigger,
    Text,
}

pub struct SnippetsPanel {
    path: PathBuf,
    /// Set when the file couldn't be read; the panel then only offers Retry and Open file.
    load_error: Option<String>,
    loaded: Vec<Snippet>,
    working: Vec<Snippet>,
    stamp: Stamp,
    has_comments: bool,
    selected: Option<usize>,
    search: String,
    test: String,
    undo: Option<Deleted>,
    banner: Option<Banner>,
    focus: Option<Focus>,
    /// A snippet just added: its errors wait until the user leaves Paste, picks another
    /// snippet, or tries to save.
    fresh: Option<usize>,
    /// The list order, held still while a trigger is typed so its row doesn't jump.
    frozen: Option<Vec<usize>>,
    /// The re-sort after typing: each snippet's row before it, and when the slide began.
    settle: Option<(HashMap<usize, usize>, f64)>,
    /// Scroll the selected row into view on the next draw.
    reveal: bool,
    reduced_motion: bool,
    /// Part of every text field's id, bumped when items change position. egui keeps a field's
    /// undo history under its id, so without this Ctrl+Z could bring back another item's text.
    fields: u32,
}

impl SnippetsPanel {
    pub fn new(path: PathBuf) -> Self {
        let mut p = SnippetsPanel {
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
            reduced_motion: reduced_motion(),
            fields: 0,
        };
        p.reload();
        p
    }

    fn reload(&mut self) {
        // an index can point at a different snippet in the reloaded file, so follow the trigger
        let keep = self.selected.and_then(|i| self.working.get(i)).map(|s| s.trigger.clone());
        match Snippets::load_stamped(&self.path) {
            Ok((s, stamp)) => {
                self.loaded = s.snippets.clone();
                self.working = s.snippets;
                self.stamp = stamp;
                self.load_error = None;
                self.has_comments = has_comments(&self.path);
                self.selected = keep.and_then(|t| self.working.iter().position(|s| s.trigger == t));
                self.undo = None;
                self.banner = None;
                self.forget_indices();
            }
            Err(e) => {
                log::error!("snippets editor: {e:#}");
                self.load_error = Some(format!("{e:#}"));
            }
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.load_error.is_none() && self.working != self.loaded
    }

    /// The working copy, for the editor window's tests.
    #[cfg(test)]
    pub(crate) fn edit(&mut self) -> &mut Vec<Snippet> {
        &mut self.working
    }

    /// Issues to show now: a new snippet's errors are held back (see `fresh`).
    fn issues(&self) -> Vec<Issue> {
        edit::validate(&self.working).into_iter().filter(|x| !(x.error && Some(x.snippet) == self.fresh)).collect()
    }

    fn saved(&mut self, list: Vec<Snippet>, stamp: Stamp) {
        self.loaded = list.clone();
        self.working = list;
        self.stamp = stamp;
        self.has_comments = false;
        self.undo = None;
        self.banner = None;
    }

    /// Blocks a write while any snippet has an error, and says why.
    fn refuse_invalid(&mut self) -> bool {
        // a save attempt is the user asking what's wrong, so a new snippet's errors show now
        self.fresh = None;
        if edit::validate(&self.working).iter().any(|i| i.error) {
            self.banner = Some(Banner::Error("fix the items marked in red first".into()));
            return true;
        }
        false
    }

    /// Saves if valid and the file is unchanged on disk. Returns true when the file holds the
    /// editor's snippets. With nothing changed it writes nothing, so `.bak` keeps the older version.
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
        let s = Snippets { snippets: edit::normalize(self.working.clone()) };
        match s.save_if_unchanged(&self.path, self.stamp) {
            Ok(SaveOutcome::Saved(stamp)) => {
                self.saved(s.snippets, stamp);
                true
            }
            Ok(SaveOutcome::Conflict) => {
                self.banner = Some(Banner::Conflict);
                false
            }
            Err(e) => {
                log::error!("snippets editor save: {e:#}");
                self.banner = Some(Banner::Error(format!("{e:#}")));
                false
            }
        }
    }

    fn overwrite(&mut self) {
        if self.refuse_invalid() {
            return;
        }
        let s = Snippets { snippets: edit::normalize(self.working.clone()) };
        match s.save_to(&self.path) {
            Ok(()) => {
                let stamp = file_stamp(&self.path);
                self.saved(s.snippets, stamp);
            }
            Err(e) => self.banner = Some(Banner::Error(format!("{e:#}"))),
        }
    }

    /// Adds a snippet and puts the cursor in its trigger. A search that found nothing becomes the
    /// trigger, so the phrase isn't typed twice. The new row waits at the bottom of the list
    /// until its trigger is typed.
    fn new_snippet(&mut self) {
        let query = self.search.trim();
        let trigger = if edit::visible(&self.working, query).is_empty() { query.to_string() } else { String::new() };
        let mut order = edit::visible(&self.working, "");
        self.working.push(Snippet { trigger, text: String::new() });
        let i = self.working.len() - 1;
        order.push(i);
        self.frozen = Some(order);
        self.settle = None;
        self.selected = Some(i);
        self.fresh = Some(i);
        self.focus = Some(Focus::Trigger);
        self.reveal = true;
        self.search.clear();
    }

    /// Drops state that holds snippet indices, for when the indices shift.
    fn forget_indices(&mut self) {
        self.fresh = None;
        self.frozen = None;
        self.settle = None;
        self.fields += 1;
    }

    /// Selects snippet `i`. The old snippet's trigger field isn't drawn again, so it never
    /// reports losing focus: settle its row here.
    fn select(&mut self, i: usize, now: f64) {
        if self.selected != Some(i) {
            self.unfreeze(now);
            self.selected = Some(i);
            self.fresh = None;
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

    /// The trigger stopped being edited: re-sort the list, sliding each row from where it was
    /// so the user sees where their snippet went.
    fn unfreeze(&mut self, now: f64) {
        if let Some(before) = self.frozen.take() {
            let before: HashMap<usize, usize> = before.into_iter().enumerate().map(|(row, i)| (i, row)).collect();
            self.settle = (!self.reduced_motion).then_some((before, now));
            self.reveal = true;
        }
    }

    /// The list rows, in order: held still while a trigger is typed, else A–Z and filtered.
    fn rows(&self) -> Vec<usize> {
        match &self.frozen {
            Some(o) => o.iter().copied().filter(|&i| i < self.working.len()).collect(),
            None => edit::visible(&self.working, &self.search),
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        if let Some(err) = self.load_error.clone() {
            ui.label(RichText::new("snippets.toml couldn't be read").size(15.0).color(TEXT));
            ui.label(RichText::new(err).color(RED));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Retry").clicked() {
                    self.reload();
                }
                if ui.link("Open snippets file").clicked() {
                    open_file(&self.path);
                }
            });
            return;
        }
        if ui.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::S)) {
            self.save();
        }
        let issues = self.issues();
        // Panels, not nested rows: each takes its own share of the window, so the footer can't be
        // pushed off-screen by a long list. Order matters: top and bottom first, then the sides.
        let bare = |bottom: i8| Frame::new().inner_margin(Margin { left: 0, right: 0, top: 4, bottom });

        Panel::top("snip-toolbar").frame(bare(8)).resizable(false).show(ui, |ui| {
            ui.horizontal(|ui| {
                let search = ui.add(TextEdit::singleline(&mut self.search).hint_text("Search…").desired_width(LIST_W));
                if search.changed() {
                    self.frozen = None;
                    self.settle = None;
                }
                let enter = search.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                let nothing_found = !self.search.trim().is_empty() && edit::visible(&self.working, &self.search).is_empty();
                if ui.button("+ New snippet").clicked() || (enter && nothing_found) {
                    self.new_snippet();
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.link("Open snippets file").clicked() {
                        open_file(&self.path);
                    }
                });
            });
        });

        Panel::bottom("snip-footer").frame(bare(0)).resizable(false).show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Test (snippets only)").size(12.0).color(MUTED));
                ui.add(TextEdit::singleline(&mut self.test).hint_text("Type or dictate a phrase").desired_width(f32::INFINITY));
            });
            if !self.test.trim().is_empty() {
                // a long signature scrolls here rather than pushing Save off-screen
                ScrollArea::vertical().id_salt("snip-test").max_height(72.0).show(ui, |ui| {
                    ui.label(RichText::new(format!("→ {}", edit::preview(&self.working, &self.test))).color(GREEN));
                });
            }
            ui.add_space(6.0);
            self.footer(ui, &issues);
        });

        Panel::left("snip-list")
            .frame(Frame::new().inner_margin(Margin { left: 0, right: 8, top: 6, bottom: 6 }))
            .default_size(LIST_W)
            .size_range(140.0..=360.0)
            .show(ui, |ui| {
                ScrollArea::vertical().id_salt("snippets").auto_shrink([false, false]).show(ui, |ui| self.snippet_list(ui, &issues));
            });

        CentralPanel::no_frame().show(ui, |ui| {
            Frame::new().inner_margin(Margin { left: 12, right: 0, top: 6, bottom: 6 }).show(ui, |ui| {
                ScrollArea::vertical().id_salt("snippet").auto_shrink([false, false]).show(ui, |ui| {
                    self.snippet_editor(ui, &issues);
                });
            });
        });
    }

    fn snippet_list(&mut self, ui: &mut egui::Ui, issues: &[Issue]) {
        let rows = self.rows();
        if rows.is_empty() && !self.search.trim().is_empty() {
            ui.label(RichText::new("No snippet matches.").color(MUTED));
            if ui.button(format!("+ Add '{}'", self.search.trim())).on_hover_text("Or press Enter in Search").clicked() {
                self.new_snippet();
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
            let s = &self.working[i];
            let bad = issues.iter().any(|x| x.error && x.snippet == i);
            let text = if s.trigger.trim().is_empty() {
                RichText::new("(no trigger)").color(if bad { RED } else { MUTED })
            } else {
                RichText::new(s.trigger.trim()).color(if bad { RED } else { TEXT })
            };
            // a child Ui, so the shifted row doesn't move the list's own layout cursor
            let mut row_ui = ui.new_child(UiBuilder::new().max_rect(rect.translate(egui::vec2(0.0, from))).layout(*ui.layout()));
            if row_ui.selectable_label(self.selected == Some(i), text).clicked() {
                clicked = Some(i);
            }
        }
        self.reveal = false;
        if let Some(i) = clicked {
            self.select(i, ui.input(|x| x.time));
        }
    }

    fn snippet_editor(&mut self, ui: &mut egui::Ui, issues: &[Issue]) {
        let Some(i) = self.selected.filter(|&i| i < self.working.len()) else {
            ui.label(RichText::new("Select a snippet, or add one with + New snippet.").color(MUTED));
            return;
        };
        let show = |ui: &mut egui::Ui, x: &Issue| {
            ui.label(RichText::new(&x.message).size(12.0).color(if x.error { RED } else { AMBER }));
        };
        let focus = self.focus.take();
        let fields = self.fields;
        let (enter, now) = ui.input(|x| (x.key_pressed(Key::Enter), x.time));
        let mut delete = false;
        let s = &mut self.working[i];

        ui.label(RichText::new("Say").size(12.0).color(MUTED));
        let trigger =
            ui.add(TextEdit::singleline(&mut s.trigger).id_salt(("trigger", fields, i)).hint_text("e.g. my signature").desired_width(f32::INFINITY));
        if focus == Some(Focus::Trigger) {
            trigger.request_focus();
        }
        issues.iter().filter(|x| x.snippet == i && x.field == Field::Trigger).for_each(|x| show(ui, x));
        ui.add_space(6.0);

        ui.label(RichText::new("Paste").size(12.0).color(MUTED));
        // Tab isn't captured (egui's default), so it moves focus on instead of typing a tab
        let text = ui.add(TextEdit::multiline(&mut s.text).id_salt(("text", fields, i)).desired_rows(5).desired_width(f32::INFINITY));
        if focus == Some(Focus::Text) {
            text.request_focus();
        }
        issues.iter().filter(|x| x.snippet == i && x.field == Field::Text).for_each(|x| show(ui, x));
        ui.add_space(10.0);
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            if ui.button("Delete snippet").clicked() {
                delete = true;
            }
        });

        if trigger.gained_focus() && self.frozen.is_none() {
            self.frozen = Some(self.rows());
        }
        if trigger.lost_focus() {
            self.unfreeze(now);
        }
        if trigger.lost_focus() && enter {
            // Enter after the trigger moves on to the text
            self.focus = Some(Focus::Text);
            ui.ctx().request_repaint();
        }
        if text.lost_focus() && self.fresh == Some(i) {
            self.fresh = None;
        }
        if delete {
            self.delete_selected();
        }
    }

    fn footer(&mut self, ui: &mut egui::Ui, issues: &[Issue]) {
        let mut reload = false;
        let mut overwrite = false;
        match &self.banner {
            Some(Banner::Conflict) => {
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(AMBER, "snippets.toml changed outside the editor.");
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
            ui.label(RichText::new("Saving removes comments from snippets.toml").size(12.0).color(MUTED));
        }
        let n = edit::changes(&self.loaded, &self.working);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            // errors still held back for a new snippet don't disable Save: pressing it shows them
            let can = self.is_dirty() && !issues.iter().any(|x| x.error);
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
    use std::path::Path;

    fn temp_file(name: &str, contents: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("murmur-snippanel-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("snippets.toml");
        std::fs::write(&p, contents).unwrap();
        p
    }

    const TWO: &str = "[[snippet]]\ntrigger = \"my signature\"\ntext = \"\"\"\nBest,\nJeff\"\"\"\n\n[[snippet]]\ntrigger = \"my email\"\ntext = \"me@example.com\"\n";

    fn on_disk(p: &Path) -> Vec<Snippet> {
        Snippets::load_stamped(p).unwrap().0.snippets
    }

    fn key(key: Key) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }
    }

    /// Runs one frame of `size` with `events`, returning every piece of text drawn and where.
    fn frame(ctx: &egui::Context, panel: &mut SnippetsPanel, size: egui::Vec2, events: Vec<egui::Event>) -> Vec<(String, egui::Pos2)> {
        fn walk(shape: &egui::epaint::Shape, out: &mut Vec<(String, egui::Pos2)>) {
            match shape {
                egui::epaint::Shape::Text(t) => out.push((t.galley.text().to_string(), t.pos)),
                egui::epaint::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), events, ..Default::default() };
        let out = ctx.run_ui(input, |ui| panel.ui(ui));
        let mut text = Vec::new();
        out.shapes.iter().for_each(|c| walk(&c.shape, &mut text));
        text
    }

    const WINDOW: egui::Vec2 = egui::vec2(736.0, 500.0);
    /// The minimum window (560×420) less the 12 px margins and the tab strip.
    const SMALLEST: egui::Vec2 = egui::vec2(536.0, 360.0);

    /// Draws a few frames (panels settle their sizes) and returns the last frame's text.
    fn settled(panel: &mut SnippetsPanel, size: egui::Vec2) -> Vec<(String, egui::Pos2)> {
        let ctx = egui::Context::default();
        (0..3).map(|_| frame(&ctx, panel, size, vec![])).last().unwrap()
    }

    #[test]
    fn edit_and_save_trims_the_trigger_and_keeps_text_exactly() {
        let p = temp_file("save", TWO);
        let mut panel = SnippetsPanel::new(p.clone());
        panel.working[1].trigger = "  my mail ".into();
        panel.working[1].text = "\n me@example.com \n".into();
        assert!(panel.is_dirty());
        assert!(panel.save());
        assert!(!panel.is_dirty());
        assert_eq!(on_disk(&p)[1], Snippet { trigger: "my mail".into(), text: "\n me@example.com \n".into() });
    }

    #[test]
    fn second_save_is_not_a_conflict() {
        let p = temp_file("twice", TWO);
        let mut panel = SnippetsPanel::new(p);
        panel.working[0].text = "Cheers".into();
        assert!(panel.save());
        std::thread::sleep(std::time::Duration::from_millis(20));
        panel.working[0].text = "Thanks".into();
        assert!(panel.save());
        assert_eq!(panel.banner, None);
    }

    #[test]
    fn outside_write_gives_conflict_then_overwrite_wins() {
        let p = temp_file("conflict", TWO);
        let mut panel = SnippetsPanel::new(p.clone());
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&p, "[[snippet]]\ntrigger = \"outside\"\ntext = \"x\"\n").unwrap();
        panel.working[1].text = "mine".into();
        assert!(!panel.save());
        assert_eq!(panel.banner, Some(Banner::Conflict));
        assert_eq!(on_disk(&p)[0].trigger, "outside");
        panel.overwrite();
        assert_eq!(panel.banner, None);
        assert_eq!(on_disk(&p)[1].text, "mine");
    }

    #[test]
    fn conflict_then_reload_discards_edits() {
        let p = temp_file("reload", TWO);
        let mut panel = SnippetsPanel::new(p.clone());
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&p, "[[snippet]]\ntrigger = \"outside\"\ntext = \"x\"\n").unwrap();
        panel.working[0].text = "mine".into();
        assert!(!panel.save());
        panel.reload();
        assert_eq!(panel.working, vec![Snippet { trigger: "outside".into(), text: "x".into() }]);
        assert!(!panel.is_dirty());
    }

    #[test]
    fn invalid_snippets_block_save_and_leave_file_alone() {
        let p = temp_file("invalid", TWO);
        let mut panel = SnippetsPanel::new(p.clone());
        panel.new_snippet();
        assert!(!panel.save());
        assert!(matches!(panel.banner, Some(Banner::Error(_))));
        assert_eq!(on_disk(&p).len(), 2);
        panel.overwrite();
        assert_eq!(on_disk(&p).len(), 2, "overwrite refuses invalid snippets too");
    }

    #[test]
    fn broken_file_shows_error_and_never_saves() {
        let p = temp_file("broken", "[[snippet]\nbroken");
        let mut panel = SnippetsPanel::new(p.clone());
        assert!(panel.load_error.is_some());
        assert!(!panel.is_dirty());
        assert!(!panel.save());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "[[snippet]\nbroken");
        assert!(settled(&mut panel, WINDOW).iter().any(|(t, _)| t == "snippets.toml couldn't be read"));
    }

    #[test]
    fn missing_file_opens_empty_and_the_first_save_creates_it() {
        let p = temp_file("missing", "");
        std::fs::remove_file(&p).unwrap();
        let mut panel = SnippetsPanel::new(p.clone());
        assert!(panel.load_error.is_none());
        assert!(panel.working.is_empty());
        panel.working.push(Snippet { trigger: "sig".into(), text: "x".into() });
        assert!(panel.save());
        assert_eq!(on_disk(&p).len(), 1);
    }

    #[test]
    fn the_seed_file_opens_empty_with_the_comments_note() {
        let p = temp_file("seed", "# Murmur snippets\n# [[snippet]]\n# trigger = \"my signature\"\n");
        let mut panel = SnippetsPanel::new(p);
        assert!(panel.working.is_empty());
        assert!(panel.has_comments);
        panel.working.push(Snippet { trigger: "sig".into(), text: "x".into() });
        assert!(settled(&mut panel, WINDOW).iter().any(|(t, _)| t == "Saving removes comments from snippets.toml"));
        assert!(panel.save());
        assert!(!panel.has_comments);
    }

    #[test]
    fn delete_undo_and_save_clears_undo() {
        let p = temp_file("undo", TWO);
        let mut panel = SnippetsPanel::new(p.clone());
        panel.selected = Some(0);
        panel.delete_selected();
        assert_eq!(panel.working.len(), 1);
        panel.undo_delete();
        assert_eq!(panel.working[0].trigger, "my signature");
        assert_eq!(panel.selected, Some(0));
        panel.selected = Some(1);
        panel.delete_selected();
        assert!(panel.save());
        assert!(panel.undo.is_none());
        assert_eq!(on_disk(&p).len(), 1);
    }

    #[test]
    fn save_without_changes_leaves_file_and_backup_alone() {
        let original = format!("# mine\n{TWO}");
        let p = temp_file("clean-save", &original);
        let mut panel = SnippetsPanel::new(p.clone());
        assert!(panel.save());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), original);
        assert!(!p.with_extension("toml.bak").exists());
    }

    #[test]
    fn new_snippet_takes_the_search_only_when_it_found_nothing() {
        let p = temp_file("prefill", TWO);
        let mut panel = SnippetsPanel::new(p);
        panel.search = " my phone ".into();
        panel.new_snippet();
        assert_eq!(panel.working[2].trigger, "my phone");
        assert!(panel.search.is_empty());
        panel.search = "email".into();
        panel.new_snippet();
        assert_eq!(panel.working[3].trigger, "");
    }

    #[test]
    fn new_snippet_waits_at_the_bottom_then_settles_into_place() {
        let p = temp_file("settle", TWO);
        let mut panel = SnippetsPanel::new(p);
        panel.reduced_motion = false;
        panel.new_snippet();
        panel.working[2].trigger = "aaa".into();
        assert_eq!(panel.rows(), vec![1, 0, 2], "row moved while typing");
        panel.unfreeze(0.0);
        assert_eq!(panel.rows(), vec![2, 1, 0]);
        assert_eq!(panel.settle.as_ref().unwrap().0[&2], 2, "slide starts from the old row");
    }

    #[test]
    fn the_slide_starts_with_every_row_where_it_was() {
        let p = temp_file("slide-start", TWO);
        let mut panel = SnippetsPanel::new(p);
        panel.reduced_motion = false;
        panel.new_snippet();
        panel.working[2].trigger = "aaa".into();
        // a start time in the future holds the slide at its first frame
        panel.unfreeze(f64::INFINITY);
        let text = settled(&mut panel, WINDOW);
        let y = |s: &str| text.iter().find(|(t, _)| t == s).unwrap_or_else(|| panic!("'{s}' not drawn")).1.y;
        let (email, signature, aaa) = (y("my email"), y("my signature"), y("aaa"));
        assert!(email < signature && signature < aaa, "rows out of their old order: {email} {signature} {aaa}");
        assert!(((signature - email) - (aaa - signature)).abs() < 0.5, "rows not evenly spaced: {email} {signature} {aaa}");
    }

    #[test]
    fn reduced_motion_resorts_without_a_slide() {
        let p = temp_file("reduced", TWO);
        let mut panel = SnippetsPanel::new(p);
        panel.reduced_motion = true;
        panel.new_snippet();
        panel.working[2].trigger = "aaa".into();
        panel.unfreeze(0.0);
        assert!(panel.settle.is_none());
        assert_eq!(panel.rows(), vec![2, 1, 0]);
    }

    #[test]
    fn selecting_another_snippet_ends_the_freeze() {
        let p = temp_file("select-unfreeze", TWO);
        let mut panel = SnippetsPanel::new(p);
        panel.new_snippet();
        panel.working[2].trigger = "aaa".into();
        panel.select(0, 0.0);
        assert!(panel.frozen.is_none());
        assert_eq!(panel.rows()[0], 2);
    }

    #[test]
    fn layout_fits_the_window_with_a_vertical_list() {
        let many = (0..30).map(|i| format!("[[snippet]]\ntrigger = \"snip {i:02}\"\ntext = \"x\"\n")).collect::<String>();
        let p = temp_file("layout", &many);
        let mut panel = SnippetsPanel::new(p);
        panel.selected = Some(0);
        panel.delete_selected();
        panel.selected = Some(0);
        panel.test = "a phrase".into();
        for size in [WINDOW, SMALLEST] {
            let text = settled(&mut panel, size);
            let at = |s: &str| text.iter().find(|(t, _)| t == s).map(|(_, p)| *p).unwrap_or_else(|| panic!("'{s}' not drawn at {size:?}"));
            for s in ["Save", "Undo delete", "Test (snippets only)", "→ a phrase", "Say", "Paste", "Delete snippet"] {
                let p = at(s);
                assert!(p.x >= 0.0 && p.y >= 0.0 && p.x < size.x && p.y < size.y, "'{s}' off-screen at {p:?} in {size:?}");
            }
            let (a, b) = (at("snip 01"), at("snip 02"));
            assert!(b.y > a.y && (b.x - a.x).abs() < 1.0, "list not vertical: {a:?} {b:?}");
        }
    }

    #[test]
    fn long_preview_keeps_save_on_screen() {
        let long = (0..40).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        let p = temp_file("long", &format!("[[snippet]]\ntrigger = \"sig\"\ntext = \"\"\"\n{long}\"\"\"\n"));
        let mut panel = SnippetsPanel::new(p);
        panel.test = "sig".into();
        let text = settled(&mut panel, SMALLEST);
        let save = text.iter().find(|(t, _)| t == "Save").expect("Save not drawn").1;
        assert!(save.y < SMALLEST.y, "Save pushed off-screen to {save:?}");
    }

    #[test]
    fn a_new_snippets_errors_wait_for_a_save_attempt() {
        let p = temp_file("fresh", TWO);
        let mut panel = SnippetsPanel::new(p);
        panel.new_snippet();
        let has = |panel: &mut SnippetsPanel| settled(panel, WINDOW).iter().any(|(t, _)| t == "Trigger needs at least one word");
        assert!(!has(&mut panel));
        assert!(!panel.save());
        assert!(has(&mut panel));
    }

    #[test]
    fn a_snippet_can_be_typed_without_the_mouse_and_proves_itself() {
        let p = temp_file("keyboard", TWO);
        let mut panel = SnippetsPanel::new(p);
        let ctx = egui::Context::default();
        frame(&ctx, &mut panel, WINDOW, vec![]);
        panel.new_snippet();
        frame(&ctx, &mut panel, WINDOW, vec![]);
        frame(&ctx, &mut panel, WINDOW, vec![egui::Event::Text("sig".into())]);
        assert_eq!(panel.working[2].trigger, "sig");
        // Enter in Say moves to Paste, where Enter is a new line
        frame(&ctx, &mut panel, WINDOW, vec![key(Key::Enter)]);
        frame(&ctx, &mut panel, WINDOW, vec![]);
        frame(&ctx, &mut panel, WINDOW, vec![egui::Event::Text("Best,".into()), key(Key::Enter), egui::Event::Text("Jeff".into())]);
        assert_eq!(panel.working[2].text, "Best,\nJeff");
        panel.test = "thanks sig".into();
        let text = frame(&ctx, &mut panel, WINDOW, vec![]);
        assert!(text.iter().any(|(t, _)| t == "→ thanks Best,\nJeff"), "no preview in {text:?}");
    }

    fn undo_key() -> egui::Event {
        egui::Event::Key { key: Key::Z, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND }
    }

    #[test]
    fn undo_in_a_field_never_brings_back_a_deleted_snippets_text() {
        let p = temp_file("undo-history", TWO);
        let mut panel = SnippetsPanel::new(p);
        let ctx = egui::Context::default();
        // the field at row 0 remembers "my signature" as its first undo point
        panel.selected = Some(0);
        panel.focus = Some(Focus::Trigger);
        frame(&ctx, &mut panel, WINDOW, vec![]);
        frame(&ctx, &mut panel, WINDOW, vec![]);
        panel.delete_selected();
        // "my email" moves up to index 0
        panel.selected = Some(0);
        panel.focus = Some(Focus::Trigger);
        frame(&ctx, &mut panel, WINDOW, vec![]);
        frame(&ctx, &mut panel, WINDOW, vec![undo_key()]);
        assert_eq!(panel.working[0].trigger, "my email");
    }

    #[test]
    fn tab_in_paste_moves_focus_instead_of_typing_a_tab() {
        let p = temp_file("tab", TWO);
        let mut panel = SnippetsPanel::new(p);
        let ctx = egui::Context::default();
        panel.selected = Some(1);
        panel.focus = Some(Focus::Text);
        frame(&ctx, &mut panel, WINDOW, vec![]);
        frame(&ctx, &mut panel, WINDOW, vec![key(Key::Tab)]);
        // the focus has moved on by the next frame, so this lands elsewhere
        frame(&ctx, &mut panel, WINDOW, vec![egui::Event::Text("x".into())]);
        assert_eq!(panel.working[1].text, "me@example.com");
    }
}
