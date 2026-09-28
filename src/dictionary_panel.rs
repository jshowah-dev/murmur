//! The dictionary editor's content: term list, term editor, test box and save bar. Draws into
//! any `egui::Ui`, so a future settings window can host it as a tab.

use murmur_lib::dictionary::{file_stamp, Dictionary, SaveOutcome, Stamp, Term};
use murmur_lib::dictionary_edit::{self as edit, Deleted};
use std::path::{Path, PathBuf};

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

    /// Saves if valid and the file is unchanged on disk. Returns true when written.
    pub fn save(&mut self) -> bool {
        if self.load_error.is_some() {
            return false;
        }
        if self.has_errors() {
            self.banner = Some(Banner::Error("fix the items marked in red first".into()));
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
    fn comments_are_detected() {
        let p = temp_file("comments", &format!("# mine\n{TWO}"));
        assert!(DictionaryPanel::new(p).has_comments);
    }
}
