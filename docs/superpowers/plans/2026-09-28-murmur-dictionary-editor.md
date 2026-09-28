# Murmur Dictionary Editor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A "Dictionary…" tray item opens a separate editor window for adding, editing, deleting and testing dictionary terms, and saved edits apply to the next dictation without a restart.

**Architecture:** `murmur.exe --dictionary` runs a separate editor process (its own single-instance mutex) that hosts a reusable egui `DictionaryPanel`. The panel edits a working copy, validates it with egui-free functions in the library, and saves with a modified-time check so it never silently clobbers a Fix-last or hand edit. The dictation pipeline reloads `dictionary.toml` when its modified time changes, just as it already does for `snippets.toml`.

**Tech Stack:** Rust 2021, eframe/egui 0.36.2 (`glow`, `default_fonts`), `windows` 0.62, `toml` 1, `serde`.

**Spec:** `docs/superpowers/specs/2026-09-28-murmur-dictionary-editor-design.md`

## Global Constraints

- Branch `feat/dictionary-editor` (already created from `main` at `e2cfb76`; the spec is commit `9b7408b`). Never push, merge, tag or release: Jeff does that.
- Tests: `cargo test --release --locked`. Baseline lib 69, bin 42, integration 1; all must still pass.
- Commit messages: conventional prefix (`feat:`, `fix:`, `docs:`, `test:`), ending with the trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` (repo convention).
- UI: dark visuals, colours from `src/correction_ui.rs` (`BG`, `BORDER`, `TEXT`, `MUTED`, `GREEN`, `AMBER`), `load_system_font`. **Never a repaint loop** (`request_repaint` every frame starves the overlay and tray in the main process; the editor is its own process but keep the rule).
- Editor window title is exactly `Murmur — Dictionary` (em dash). The single-instance lookup finds it by that title.
- Tray label is exactly `Dictionary…` (ellipsis character, as in `History…`).
- The editor's raw-file link label is exactly `Open dictionary file`.
- The checkbox label is exactly `Also match sound-alikes`, default on.
- No test, fixture or doc uses the contents of Jeff's real `dictionary.toml`. Use the terms from the existing `dictionary.rs` tests (HAWB, BOL, Delgado) or made-up ones.
- Smoke tests and anything that reads or writes the real `%APPDATA%\Murmur`: the Claude desktop app's shell is MSIX-virtualized. Launch Murmur from Explorer (or have Jeff launch it) and read real AppData files only through an Explorer-launched `.cmd` that copies them under `C:\Users\JeffLocal\git\murmur\target\smoke\`. Unit tests may use `std::env::temp_dir()`.

## Review Focus

1. **A new, still-empty term while testing a phrase.** Expected: the test result is the phrase unchanged. (An empty written form would otherwise match every punctuation-only token.) Test: `preview_ignores_terms_with_empty_written_form` in Task 3.
2. **The same spoken form typed with different case or spacing on two terms** (`"Hob "` on one, `"hob"` on another). Expected: flagged as shared, Save blocked. Test: `shared_spoken_ignores_case_and_spaces` in Task 3.
3. **Two saves in a row from the editor with nothing else touching the file.** Expected: both succeed, no false conflict (the stamp is refreshed after each save). Test: `second_save_is_not_a_conflict` in Task 4.
4. **The editor opened on a broken `dictionary.toml`.** Expected: error + Retry + Open dictionary file; nothing can be saved over it. Test: `broken_file_shows_error_and_never_saves` in Task 4.
5. **`dictionary.toml` deleted while Murmur runs.** Expected: dictation keeps the current terms, no error notice. Test: `missing_file_keeps_current_terms` in Task 2.

---

## File Structure

| File | Status | Responsibility |
|---|---|---|
| `src/dictionary.rs` | modify | Add `Stamp`, `SaveOutcome`, `pub fn path`, `file_stamp`, `save_to`, `load_stamped`, `save_if_unchanged`, `DictionaryFile` (live reload tracker). |
| `src/dictionary_edit.rs` | create (lib) | egui-free editing logic: `Issue`, `validate`, `normalize`, `visible`, `Deleted`, `delete`, `undo`, `changes`, `preview`. |
| `src/lib.rs` | modify | `pub mod dictionary_edit;` |
| `src/pipeline.rs` | modify | Hold a `DictionaryFile`; refresh it before each clean. |
| `src/dictionary_panel.rs` | create (bin) | `DictionaryPanel`: working-copy state, save/reload/overwrite, and `fn ui(&mut self, ui: &mut egui::Ui)`. |
| `src/dictionary_editor.rs` | create (bin) | The editor process: mutex, focus-existing, eframe window, close-with-unsaved prompt. |
| `src/correction_ui.rs` | modify | `AMBER` becomes `pub(crate)`. |
| `src/tray.rs` | modify | `Dictionary…` label, `TrayEvent::EditDictionary`. |
| `src/main.rs` | modify | `--dictionary` dispatch before the app mutex; spawn the editor from the tray. |
| `README.md` | modify | Tray table row and Dictionary section. |

---

### Task 1: Stamped load and save

**Files:**
- Modify: `src/dictionary.rs` (imports at top; `fn path` line 43; `save` lines 104-118; tests module)

**Interfaces:**
- Consumes: nothing new.
- Produces (all `pub` in `murmur_lib::dictionary`):
  - `pub type Stamp = Option<std::time::SystemTime>;`
  - `#[derive(Debug, PartialEq)] pub enum SaveOutcome { Saved(Stamp), Conflict }`
  - `pub fn path() -> PathBuf`
  - `pub fn file_stamp(p: &Path) -> Stamp`
  - `impl Dictionary { pub fn save_to(&self, p: &Path) -> Result<()>; pub fn load_stamped(p: &Path) -> Result<(Dictionary, Stamp)>; pub fn save_if_unchanged(&self, p: &Path, stamp: Stamp) -> Result<SaveOutcome>; }`
  - `save()` keeps its signature and behaviour (`self.save_to(&path())`).

- [ ] **Step 1: Write the failing tests** (append inside `mod tests` in `src/dictionary.rs`)

```rust
    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("murmur-dict-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("dictionary.toml")
    }

    #[test]
    fn save_if_unchanged_saves_when_stamp_matches() {
        let p = temp_path("match");
        dict().save_to(&p).unwrap();
        let (mut d, stamp) = Dictionary::load_stamped(&p).unwrap();
        assert!(stamp.is_some());
        d.terms.push(Term { written: "Kowalczyk".into(), spoken: vec![], phonetic: true });
        let out = d.save_if_unchanged(&p, stamp).unwrap();
        assert!(matches!(out, SaveOutcome::Saved(Some(_))));
        let (back, _) = Dictionary::load_stamped(&p).unwrap();
        assert_eq!(back.terms.len(), 4);
        assert!(p.with_extension("toml.bak").exists());
    }

    #[test]
    fn save_if_unchanged_reports_conflict_after_outside_write() {
        let p = temp_path("conflict");
        dict().save_to(&p).unwrap();
        let (d, stamp) = Dictionary::load_stamped(&p).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&p, "[[term]]\nwritten = \"Outside\"\n").unwrap();
        assert_eq!(d.save_if_unchanged(&p, stamp).unwrap(), SaveOutcome::Conflict);
        let (back, _) = Dictionary::load_stamped(&p).unwrap();
        assert_eq!(back.terms[0].written, "Outside");
    }

    #[test]
    fn save_if_unchanged_refuses_when_not_loaded_cleanly() {
        let p = temp_path("unloaded");
        dict().save_to(&p).unwrap();
        let stamp = file_stamp(&p);
        assert!(Dictionary::empty_unloaded().save_if_unchanged(&p, stamp).is_err());
        assert_eq!(Dictionary::load_stamped(&p).unwrap().0.terms.len(), 3);
    }

    #[test]
    fn load_stamped_missing_file_is_empty_with_no_stamp() {
        let p = temp_path("missing");
        let (d, stamp) = Dictionary::load_stamped(&p).unwrap();
        assert!(d.terms.is_empty());
        assert_eq!(stamp, None);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --release --locked --lib dictionary::tests`
Expected: compile errors: `save_to`, `load_stamped`, `save_if_unchanged`, `file_stamp`, `SaveOutcome` not found.

- [ ] **Step 3: Implement**

In `src/dictionary.rs`, change the imports:

```rust
use std::path::{Path, PathBuf};
use std::time::SystemTime;
```

After `fn default_loaded_cleanly`, add:

```rust
/// A file's modified time, `None` when it doesn't exist. Used to notice writes by another process.
pub type Stamp = Option<SystemTime>;

#[derive(Debug, PartialEq)]
pub enum SaveOutcome {
    Saved(Stamp),
    /// The file changed since it was loaded; nothing was written.
    Conflict,
}

pub fn file_stamp(p: &Path) -> Stamp {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}
```

Make `fn path()` public: `pub fn path() -> PathBuf {`.

Replace `save` with:

```rust
    pub fn save(&self) -> Result<()> {
        self.save_to(&path())
    }

    pub fn save_to(&self, p: &Path) -> Result<()> {
        if !self.loaded_cleanly {
            return Err(anyhow!("dictionary was not loaded cleanly; fix dictionary.toml first"));
        }
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        if p.exists() {
            if let Err(e) = std::fs::copy(p, p.with_extension("toml.bak")) {
                log::warn!("failed to back up dictionary.toml: {e}");
            }
        }
        let tmp = p.with_extension("toml.tmp");
        std::fs::write(&tmp, self.to_toml()?).context("write dictionary.toml.tmp")?;
        std::fs::rename(&tmp, p).context("rename dictionary.toml.tmp")
    }

    /// Load for editing. The stamp is read before the file, so a write landing mid-read shows
    /// up later as a conflict rather than being silently overwritten.
    pub fn load_stamped(p: &Path) -> Result<(Self, Stamp)> {
        let stamp = file_stamp(p);
        if stamp.is_none() {
            return Ok((Self::from_terms(vec![]), None));
        }
        let d = Self::from_toml(&std::fs::read_to_string(p).context("read dictionary.toml")?)?;
        Ok((d, stamp))
    }

    /// Save only if the file is still the version stamped at load (or at the last save).
    pub fn save_if_unchanged(&self, p: &Path, stamp: Stamp) -> Result<SaveOutcome> {
        if file_stamp(p) != stamp {
            return Ok(SaveOutcome::Conflict);
        }
        self.save_to(p)?;
        Ok(SaveOutcome::Saved(file_stamp(p)))
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --release --locked --lib dictionary::tests`
Expected: all dictionary tests pass (16 existing + 4 new).

- [ ] **Step 5: Commit**

```bash
git add src/dictionary.rs
git commit -m "feat: stamped dictionary load and conflict-checked save" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Live reload in the pipeline

**Files:**
- Modify: `src/dictionary.rs` (new `DictionaryFile` after `impl Dictionary`; tests)
- Modify: `src/pipeline.rs` (import line 3; `State` struct around line 45; the refresh at lines 145-147; `State` construction line 187)

**Interfaces:**
- Consumes: `Stamp`, `file_stamp`, `path()` from Task 1.
- Produces: `pub struct DictionaryFile`, `pub fn DictionaryFile::new(path: PathBuf) -> Self`, `pub fn refresh(&mut self, shared: &std::sync::Mutex<Dictionary>) -> Option<String>`.

- [ ] **Step 1: Write the failing tests** (append inside `mod tests` in `src/dictionary.rs`)

```rust
    #[test]
    fn dictionary_file_reloads_on_change_and_keeps_last_good() {
        let p = temp_path("reload");
        dict().save_to(&p).unwrap();
        let shared = std::sync::Mutex::new(dict());
        let mut f = DictionaryFile::new(p.clone());
        // unchanged since new(): not re-read
        assert_eq!(f.refresh(&shared), None);
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&p, "[[term]]\nwritten = \"Kowalczyk\"\nspoken = [\"kowalski\"]\n").unwrap();
        assert_eq!(f.refresh(&shared), None);
        assert_eq!(shared.lock().unwrap().apply("call kowalski"), "call Kowalczyk");
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&p, "[[term]\nbroken").unwrap();
        assert!(f.refresh(&shared).is_some());
        // reported once per bad version
        assert_eq!(f.refresh(&shared), None);
        assert_eq!(shared.lock().unwrap().terms[0].written, "Kowalczyk");
    }

    #[test]
    fn dictionary_file_revives_after_failed_startup() {
        let p = temp_path("revive");
        std::fs::write(&p, "[[term]\nbroken").unwrap();
        let shared = std::sync::Mutex::new(Dictionary::empty_unloaded());
        let mut f = DictionaryFile::new(p.clone());
        std::thread::sleep(std::time::Duration::from_millis(20));
        dict().save_to(&p).unwrap();
        assert_eq!(f.refresh(&shared), None);
        let d = shared.lock().unwrap();
        assert_eq!(d.terms.len(), 3);
        // the reloaded copy is saveable again
        d.save_to(&p).unwrap();
    }

    #[test]
    fn missing_file_keeps_current_terms() {
        let p = temp_path("gone");
        dict().save_to(&p).unwrap();
        let shared = std::sync::Mutex::new(dict());
        let mut f = DictionaryFile::new(p.clone());
        std::fs::remove_file(&p).unwrap();
        assert_eq!(f.refresh(&shared), None);
        assert_eq!(shared.lock().unwrap().terms.len(), 3);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --release --locked --lib dictionary::tests`
Expected: compile error: `DictionaryFile` not found.

- [ ] **Step 3: Implement `DictionaryFile`** (in `src/dictionary.rs`, after the closing `}` of `impl Dictionary`, before `#[cfg(test)]`)

```rust
/// dictionary.toml as the pipeline sees it: reloaded into the shared dictionary when its
/// modified time changes (the editor, Fix-last or a hand edit). A bad edit keeps the last good terms.
pub struct DictionaryFile {
    path: PathBuf,
    stamp: Stamp,
}

impl DictionaryFile {
    /// Starts from the file's current stamp: the shared dictionary was loaded from it at startup.
    pub fn new(path: PathBuf) -> Self {
        let stamp = file_stamp(&path);
        DictionaryFile { path, stamp }
    }

    /// Reload if the file changed since the last check. Returns an error message once per bad edit.
    pub fn refresh(&mut self, shared: &std::sync::Mutex<Dictionary>) -> Option<String> {
        let stamp = file_stamp(&self.path);
        if stamp == self.stamp {
            return None;
        }
        self.stamp = stamp;
        // deleted: keep what we have; it comes back on the next save or restart
        if stamp.is_none() {
            return None;
        }
        match std::fs::read_to_string(&self.path).map_err(anyhow::Error::from).and_then(|s| Dictionary::from_toml(&s)) {
            Ok(d) => {
                log::info!("reloaded {} dictionary terms", d.terms.len());
                *shared.lock().unwrap_or_else(|e| e.into_inner()) = d;
                None
            }
            Err(e) => {
                log::error!("dictionary.toml: {e:#}");
                Some(format!("dictionary.toml not loaded, keeping the previous terms: {e}"))
            }
        }
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --release --locked --lib dictionary::tests`
Expected: PASS (23 dictionary tests).

- [ ] **Step 5: Wire it into the pipeline**

In `src/pipeline.rs`:
- line 3 (`use crate::dictionary::Dictionary;`) becomes `use crate::dictionary::{self, Dictionary, DictionaryFile};`. `crate::dictionary` resolves because `main.rs` imports `murmur_lib::dictionary` into the bin crate root.
- In `struct State`, after `snippets: SnippetFile,` add `dict_file: DictionaryFile,`.
- Replace lines 145-147:

```rust
        if let Some(e) = self.snippets.refresh() {
            let _ = self.tx.send(PipelineMsg::Error(e));
        }
        if let Some(e) = self.dict_file.refresh(&self.dict) {
            let _ = self.tx.send(PipelineMsg::Error(e));
        }
```

- In the `State { ... }` construction (line 187), after `snippets: SnippetFile::new(snippets::path()),` add `dict_file: DictionaryFile::new(dictionary::path()),`.

- [ ] **Step 6: Build and run the whole suite**

Run: `cargo test --release --locked`
Expected: lib 76 (69 + 7), bin 42, integration 1, all pass.

- [ ] **Step 7: Commit**

```bash
git add src/dictionary.rs src/pipeline.rs
git commit -m "feat: reload dictionary.toml when it changes on disk" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Editing logic (egui-free)

**Files:**
- Create: `src/dictionary_edit.rs`
- Modify: `src/lib.rs` (add `pub mod dictionary_edit;` after `pub mod dictionary;`)

**Interfaces:**
- Consumes: `Term`, `Dictionary::from_terms`, `Dictionary::apply` (`crate::dictionary`), `phonetic::is_common`.
- Produces (`murmur_lib::dictionary_edit`):
  - `#[derive(Debug, Clone, PartialEq)] pub struct Issue { pub term: usize, pub spoken: Option<usize>, pub error: bool, pub message: String }`
  - `pub fn validate(terms: &[Term]) -> Vec<Issue>`
  - `pub fn normalize(terms: Vec<Term>) -> Vec<Term>`
  - `pub fn visible(terms: &[Term], query: &str) -> Vec<usize>`
  - `#[derive(Debug, Clone, PartialEq)] pub struct Deleted { pub index: usize, pub term: Term }`
  - `pub fn delete(terms: &mut Vec<Term>, index: usize) -> Deleted`
  - `pub fn undo(terms: &mut Vec<Term>, d: Deleted) -> usize` (returns the index the term went back to)
  - `pub fn changes(loaded: &[Term], working: &[Term]) -> usize`
  - `pub fn preview(terms: &[Term], text: &str) -> String`

- [ ] **Step 1: Write the file with tests first** (`src/dictionary_edit.rs`: the tests, plus stub signatures that `todo!()` so it compiles)

```rust
//! Editing rules for the dictionary editor, kept free of egui so they can be unit-tested.

use crate::dictionary::{Dictionary, Term};
use crate::phonetic;

#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    pub term: usize,
    /// Which spoken form, when the issue is about one.
    pub spoken: Option<usize>,
    /// Errors block Save; warnings don't.
    pub error: bool,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Deleted {
    pub index: usize,
    pub term: Term,
}

pub fn validate(terms: &[Term]) -> Vec<Issue> { todo!() }
pub fn normalize(terms: Vec<Term>) -> Vec<Term> { todo!() }
pub fn visible(terms: &[Term], query: &str) -> Vec<usize> { todo!() }
pub fn delete(terms: &mut Vec<Term>, index: usize) -> Deleted { todo!() }
pub fn undo(terms: &mut Vec<Term>, d: Deleted) -> usize { todo!() }
pub fn changes(loaded: &[Term], working: &[Term]) -> usize { todo!() }
pub fn preview(terms: &[Term], text: &str) -> String { todo!() }

#[cfg(test)]
mod tests {
    use super::*;

    fn t(written: &str, spoken: &[&str]) -> Term {
        Term { written: written.into(), spoken: spoken.iter().map(|s| s.to_string()).collect(), phonetic: true }
    }

    fn errors(terms: &[Term]) -> Vec<Issue> {
        validate(terms).into_iter().filter(|i| i.error).collect()
    }

    #[test]
    fn clean_list_has_no_issues() {
        assert!(validate(&[t("HAWB", &["hob"]), t("BOL", &["bee oh el"])]).is_empty());
    }

    #[test]
    fn empty_written_is_an_error() {
        let e = errors(&[t("  ", &[])]);
        assert_eq!(e.len(), 1);
        assert_eq!((e[0].term, e[0].spoken), (0, None));
    }

    #[test]
    fn duplicate_written_ignoring_case_flags_both() {
        let e = errors(&[t("HAWB", &[]), t("hawb ", &[])]);
        assert_eq!(e.iter().map(|i| i.term).collect::<Vec<_>>(), vec![0, 1]);
    }

    #[test]
    fn shared_spoken_ignores_case_and_spaces() {
        let e = errors(&[t("HAWB", &["Hob "]), t("Hobart", &["hob"])]);
        assert_eq!(e.iter().map(|i| (i.term, i.spoken)).collect::<Vec<_>>(), vec![(0, Some(0)), (1, Some(0))]);
    }

    #[test]
    fn empty_spoken_fields_are_ignored() {
        assert!(validate(&[t("HAWB", &[""]), t("BOL", &["  "])]).is_empty());
    }

    #[test]
    fn common_word_spoken_is_a_warning_only() {
        let issues = validate(&[t("Call", &["all"])]);
        assert_eq!(issues.len(), 1);
        assert!(!issues[0].error);
        assert_eq!(issues[0].spoken, Some(0));
        assert!(issues[0].message.contains("'all'"));
    }

    #[test]
    fn normalize_trims_dedupes_and_lowercases_spoken() {
        let n = normalize(vec![t(" HAWB ", &["Hob", " hob", "", "  bee   oh el "])]);
        assert_eq!(n, vec![t("HAWB", &["hob", "bee oh el"])]);
    }

    #[test]
    fn normalize_keeps_phonetic_flag() {
        let mut x = t("BOL", &[]);
        x.phonetic = false;
        assert!(!normalize(vec![x])[0].phonetic);
    }

    #[test]
    fn visible_sorts_ignoring_case_and_filters_written_and_spoken() {
        let terms = [t("jira", &["jerry"]), t("BOL", &["bee oh el"]), t("HAWB", &["hob"])];
        assert_eq!(visible(&terms, ""), vec![1, 2, 0]);
        assert_eq!(visible(&terms, "JER"), vec![0]);
        assert_eq!(visible(&terms, " hawb "), vec![2]);
        assert!(visible(&terms, "zzz").is_empty());
    }

    #[test]
    fn new_empty_term_sorts_first() {
        assert_eq!(visible(&[t("BOL", &[]), t("", &[])], ""), vec![1, 0]);
    }

    #[test]
    fn delete_then_undo_restores_position() {
        let mut terms = vec![t("A1", &[]), t("B2", &[]), t("C3", &[])];
        let before = terms.clone();
        let d = delete(&mut terms, 1);
        assert_eq!(terms.len(), 2);
        assert_eq!(undo(&mut terms, d), 1);
        assert_eq!(terms, before);
    }

    #[test]
    fn undo_clamps_index_when_list_shrank() {
        let mut terms = vec![t("A1", &[]), t("B2", &[])];
        let d = delete(&mut terms, 1);
        terms.clear();
        assert_eq!(undo(&mut terms, d), 0);
    }

    #[test]
    fn changes_counts_added_removed_and_edited_terms() {
        let loaded = vec![t("A1", &[]), t("B2", &[]), t("C3", &[])];
        assert_eq!(changes(&loaded, &loaded), 0);
        let mut edited = loaded.clone();
        edited[0].written = "A9".into();
        assert_eq!(changes(&loaded, &edited), 1);
        let mut added = loaded.clone();
        added.push(t("D4", &[]));
        assert_eq!(changes(&loaded, &added), 1);
        let mut edit_and_delete = edited.clone();
        edit_and_delete.remove(1);
        assert_eq!(changes(&loaded, &edit_and_delete), 2);
    }

    #[test]
    fn preview_applies_unsaved_terms() {
        assert_eq!(preview(&[t("HAWB", &[" Hob "])], "send the hob"), "send the HAWB");
    }

    #[test]
    fn preview_ignores_terms_with_empty_written_form() {
        assert_eq!(preview(&[t("", &["hob"]), t("  ", &[])], "send the hob — now"), "send the hob — now");
    }
}
```

- [ ] **Step 2: Register the module and run the tests to verify they fail**

Add `pub mod dictionary_edit;` to `src/lib.rs` after `pub mod dictionary;`.

Run: `cargo test --release --locked --lib dictionary_edit`
Expected: every test panics with `not yet implemented`.

- [ ] **Step 3: Implement** (replace the seven `todo!()` stubs)

```rust
fn fold(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

pub fn validate(terms: &[Term]) -> Vec<Issue> {
    let mut out = Vec::new();
    for (i, t) in terms.iter().enumerate() {
        let w = fold(&t.written);
        if w.is_empty() {
            out.push(Issue { term: i, spoken: None, error: true, message: "Written form can't be empty".into() });
        } else if (0..terms.len()).any(|j| j != i && fold(&terms[j].written) == w) {
            out.push(Issue { term: i, spoken: None, error: true, message: format!("'{}' is already a term", t.written.trim()) });
        }
        for (k, s) in t.spoken.iter().enumerate() {
            let s = fold(s);
            if s.is_empty() {
                continue;
            }
            if let Some(j) = (0..terms.len()).find(|&j| j != i && terms[j].spoken.iter().any(|x| fold(x) == s)) {
                out.push(Issue {
                    term: i,
                    spoken: Some(k),
                    error: true,
                    message: format!("'{s}' is also a spoken form of '{}'", terms[j].written.trim()),
                });
            }
            if !s.contains(' ') && phonetic::is_common(&s) {
                out.push(Issue {
                    term: i,
                    spoken: Some(k),
                    error: false,
                    message: format!("'{s}' is a common word: this will rewrite '{s}' everywhere"),
                });
            }
        }
    }
    out
}

/// What Save writes: trimmed written form; spoken forms trimmed, single-spaced, lower-cased,
/// empties dropped and duplicates removed.
pub fn normalize(terms: Vec<Term>) -> Vec<Term> {
    terms
        .into_iter()
        .map(|t| {
            let mut spoken: Vec<String> = Vec::new();
            for s in t.spoken {
                let s = fold(&s);
                if !s.is_empty() && !spoken.contains(&s) {
                    spoken.push(s);
                }
            }
            Term { written: t.written.trim().to_string(), spoken, phonetic: t.phonetic }
        })
        .collect()
}

/// Indices of the terms to list: matching `query` (written or spoken, ignoring case), A–Z by written form.
pub fn visible(terms: &[Term], query: &str) -> Vec<usize> {
    let q = fold(query);
    let mut v: Vec<usize> = (0..terms.len())
        .filter(|&i| q.is_empty() || fold(&terms[i].written).contains(&q) || terms[i].spoken.iter().any(|s| fold(s).contains(&q)))
        .collect();
    v.sort_by_key(|&i| (fold(&terms[i].written), i));
    v
}

pub fn delete(terms: &mut Vec<Term>, index: usize) -> Deleted {
    Deleted { index, term: terms.remove(index) }
}

pub fn undo(terms: &mut Vec<Term>, d: Deleted) -> usize {
    let at = d.index.min(terms.len());
    terms.insert(at, d.term);
    at
}

/// Terms added, removed or edited; an edit counts once (it's one term missing on each side).
pub fn changes(loaded: &[Term], working: &[Term]) -> usize {
    fn missing(a: &[Term], b: &[Term]) -> usize {
        let mut pool: Vec<&Term> = b.iter().collect();
        a.iter()
            .filter(|t| match pool.iter().position(|x| x == t) {
                Some(p) => {
                    pool.swap_remove(p);
                    false
                }
                None => true,
            })
            .count()
    }
    missing(working, loaded).max(missing(loaded, working))
}

/// `text` as the (unsaved) terms would rewrite it. Terms with no written form are skipped:
/// an empty phrase would match every punctuation-only token.
pub fn preview(terms: &[Term], text: &str) -> String {
    let usable = normalize(terms.to_vec()).into_iter().filter(|t| !t.written.is_empty()).collect();
    Dictionary::from_terms(usable).apply(text)
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --release --locked --lib dictionary_edit`
Expected: 15 passed. If `common_word_spoken_is_a_warning_only` fails because `all` is not in `src/common_words.txt`, check with `grep -x all src/common_words.txt` and switch the test to a word that is listed. Don't edit the word list.

- [ ] **Step 5: Commit**

```bash
git add src/dictionary_edit.rs src/lib.rs
git commit -m "feat: dictionary editing rules (validate, normalize, filter, undo)" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: The dictionary panel

**Files:**
- Create: `src/dictionary_panel.rs`
- Modify: `src/correction_ui.rs` (`const AMBER` becomes `pub(crate) const AMBER`)
- Modify: `src/main.rs` (add `mod dictionary_panel;` to the `mod` list)

**Interfaces:**
- Consumes: Task 1 (`Dictionary::load_stamped`, `save_if_unchanged`, `save_to`, `file_stamp`, `SaveOutcome`, `Stamp`), Task 3 (all of `dictionary_edit`), `correction_ui::{AMBER, GREEN, MUTED, TEXT}`.
- Produces: `pub struct DictionaryPanel` with `pub fn new(path: PathBuf) -> Self`, `pub fn is_dirty(&self) -> bool`, `pub fn save(&mut self) -> bool` (true when written), `pub fn ui(&mut self, ui: &mut egui::Ui)`.

- [ ] **Step 1: Write the file with its state-machine tests** (UI drawing comes in Step 5; write everything except `fn ui`/`fn term_editor`/`fn footer` now)

```rust
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
```

- [ ] **Step 2: Register the module and make `AMBER` visible**

In `src/main.rs`, add `mod dictionary_panel;` after `mod correction_ui;`. In `src/correction_ui.rs` change `const AMBER: Color32` to `pub(crate) const AMBER: Color32`.

Add `#[allow(dead_code)]` on `impl DictionaryPanel` for now only if the build fails on dead-code warnings as errors (it shouldn't: warnings are not errors in this repo). Remove it in Task 5.

- [ ] **Step 3: Run the tests to verify they pass**

Run: `cargo test --release --locked --bin murmur dictionary_panel`
Expected: 8 passed. (TDD note: the state logic is written with the tests in Step 1 because it's thin glue over Tasks 1 and 3, which are already tested. If a test fails, fix the panel code, not the test, unless the test contradicts the spec.)

- [ ] **Step 4: Commit the state logic**

```bash
git add src/dictionary_panel.rs src/correction_ui.rs src/main.rs
git commit -m "feat: dictionary panel state (load, save, conflict, undo)" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 5: Add the UI** (append to `impl DictionaryPanel`)

```rust
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
```

If `cargo build` reports an egui 0.36 API mismatch (for example a renamed method), fix it against the source in `~/.cargo/registry/src/*/egui-0.36.2/src/` and keep the behaviour. Don't swap the layout for a different design.

- [ ] **Step 6: Build and test**

Run: `cargo test --release --locked`
Expected: lib 91 (69 + 7 + 15), bin 50 (42 + 8), integration 1, all pass. Warnings about unused `ui` are fine until Task 5.

- [ ] **Step 7: Commit**

```bash
git add src/dictionary_panel.rs
git commit -m "feat: dictionary panel UI" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Editor process, tray entry, README

**Files:**
- Create: `src/dictionary_editor.rs`
- Modify: `src/main.rs` (mod list; `main()` start; tray match at line 190; new tests module)
- Modify: `src/tray.rs` (enum; label line 49; id mapping line 59)
- Modify: `README.md` (tray table row, Dictionary section)

**Interfaces:**
- Consumes: `DictionaryPanel` (Task 4), `murmur_lib::dictionary::path()` (Task 1), `correction_ui::{load_system_font, BG}`, `correction::bring_to_front` (`pub(crate) unsafe fn bring_to_front(hwnd: HWND)`).
- Produces: `pub fn dictionary_editor::run() -> anyhow::Result<()>`, `const EDITOR_ARG: &str = "--dictionary"`, `fn wants_editor(args: impl Iterator<Item = String>) -> bool` in `main.rs`, `TrayEvent::EditDictionary`.

- [ ] **Step 1: Write the failing argument test** (append to the end of `src/main.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> impl Iterator<Item = String> {
        a.iter().map(|s| s.to_string()).collect::<Vec<_>>().into_iter()
    }

    #[test]
    fn dictionary_flag_selects_the_editor() {
        assert!(wants_editor(args(&["murmur.exe", "--dictionary"])));
        assert!(!wants_editor(args(&["murmur.exe"])));
        assert!(!wants_editor(args(&["murmur.exe", "--other"])));
    }
}
```

Run: `cargo test --release --locked --bin murmur dictionary_flag`
Expected: compile error, `wants_editor` not found.

- [ ] **Step 2: Create `src/dictionary_editor.rs`**

```rust
//! `murmur.exe --dictionary`: the dictionary editor as its own process, so dictation keeps
//! working while it's open. One editor at a time; a second launch brings the first forward.

use crate::correction_ui::{load_system_font, BG};
use crate::dictionary_panel::DictionaryPanel;
use anyhow::Result;
use eframe::egui::{self, Frame, Id, Margin, Modal, ViewportCommand};
use std::sync::Arc;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, ShowWindow, SW_RESTORE};

const TITLE: &str = "Murmur — Dictionary";

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

impl EditorApp {
    fn close(&mut self, ctx: &egui::Context) {
        self.closing = true;
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }
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
        if self.asking {
            let mut choice = None;
            Modal::new(Id::new("unsaved-changes")).show(&ctx, |ui| {
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
            match choice {
                Some(CloseChoice::Save) => {
                    self.asking = false;
                    // a failed save (invalid terms, conflict, I/O) keeps the window open with its banner
                    if self.panel.save() {
                        self.close(&ctx);
                    }
                }
                Some(CloseChoice::Discard) => {
                    self.asking = false;
                    self.close(&ctx);
                }
                Some(CloseChoice::Cancel) => self.asking = false,
                None => {}
            }
        }
    }
}

/// Brings an already-open editor to the front. If its window isn't up yet, does nothing.
fn focus_existing() {
    unsafe {
        if let Ok(hwnd) = FindWindowW(PCWSTR::null(), w!("Murmur — Dictionary")) {
            let _ = ShowWindow(hwnd, SW_RESTORE);
            crate::correction::bring_to_front(hwnd);
        }
    }
}

pub fn run() -> Result<()> {
    log::info!("dictionary editor starting");
    let _instance = unsafe {
        let m = CreateMutexW(None, false, w!("Local\\Murmur.DictionaryEditor"))?;
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
```

- [ ] **Step 3: Wire up `main.rs`**

- Add `mod dictionary_editor;` after `mod correction_ui;` (next to `mod dictionary_panel;`).
- After `fn open_path`, add:

```rust
const EDITOR_ARG: &str = "--dictionary";

fn wants_editor(mut args: impl Iterator<Item = String>) -> bool {
    args.nth(1).as_deref() == Some(EDITOR_ARG)
}
```

- In `main()`, directly after `init_logging();` and before the `murmur ... starting` log line:

```rust
    // the editor is its own process, so it must not take the app's single-instance mutex
    if wants_editor(std::env::args()) {
        return dictionary_editor::run();
    }
```

- Replace line 190 (`TrayEvent::OpenDictionary => open_path(...)`) with:

```rust
                TrayEvent::EditDictionary => {
                    // a separate process, so dictation keeps working while it's open
                    let spawned = std::env::current_exe().and_then(|exe| std::process::Command::new(exe).arg(EDITOR_ARG).spawn());
                    if let Err(e) = spawned {
                        log::error!("dictionary editor: {e}");
                        tray.notify("Murmur", &format!("Couldn't open the dictionary editor: {e}"));
                    }
                }
```

- [ ] **Step 4: Update the tray** (`src/tray.rs`)

- Enum: rename `OpenDictionary` to `EditDictionary`.
- Line 49: `let dict = MenuItem::new("Dictionary…", true, None);`
- Line 59: `(dict.id().clone(), TrayEvent::EditDictionary),`

- [ ] **Step 5: Build and run the whole suite**

Run: `cargo test --release --locked`
Expected: lib 91, bin 51, integration 1, all pass, no warnings about unused `ui`/`is_dirty` (remove any `#[allow(dead_code)]` added in Task 4).

Run: `cargo build --release --locked`
Expected: builds `target\release\murmur.exe`.

- [ ] **Step 6: Update the README**

In `README.md`, the tray table row:

```markdown
| Open dictionary | Edit `dictionary.toml` |
```

becomes

```markdown
| Dictionary… | Add, edit, delete and test dictionary terms |
```

Replace the `### Dictionary` section's text and example (everything from `### Dictionary` up to, not including, `### Snippets`) with:

````markdown
### Dictionary

The dictionary maps what you say to what should be written. Open it from the tray with **Dictionary…**: pick a term to edit it, **+ New term** to add one, and type or dictate into the test box to see what the dictionary does to a phrase. Changes apply to the next dictation once you save; dictation keeps working while the editor is open. Fix-last adds entries automatically.

- **Written as:** the text Murmur writes.
- **Heard as:** what the speech model tends to hear instead, e.g. `cooper netties` for Kubernetes.
- **Also match sound-alikes:** also catch words that sound like the term. On by default.

The terms live in `%APPDATA%\Murmur\dictionary.toml` (the editor's **Open dictionary file** link opens it). You can edit the file by hand; saving from the editor removes comments from it.

```toml
[[term]]
written = "Kubernetes"
spoken = ["cooper netties"]
phonetic = true   # also match words that sound alike (default)
```
````

- [ ] **Step 7: Commit**

```bash
git add src/dictionary_editor.rs src/main.rs src/tray.rs README.md
git commit -m "feat: dictionary editor window from the tray" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Smoke test on the real machine (with Jeff)

No code unless a step fails. A clean build is not a smoke test; this task is.

- [ ] **Step 1: Prepare.** Jeff quits the installed Murmur (tray → Quit). Jeff starts `C:\Users\JeffLocal\git\murmur\target\release\murmur.exe` by double-clicking it in Explorer. It must be launched from Explorer, not from Claude's shell, so it sees the real `%APPDATA%`.
- [ ] **Step 2: Run the spec's six checks** (spec § Testing, "Manual smoke") and record pass/fail for each:
  1. Tray → Dictionary… opens the editor; a second click brings the same window to the front (only one editor in Task Manager).
  2. Dictate into Notepad while the editor is open: it works.
  3. Add a term (e.g. written `Zanzibar`, heard as `zanzi bar`) → Save → dictate "zanzi bar": `Zanzibar`, no restart.
  4. Make an unsaved edit, run Fix-last on a dictation that teaches a term, then Save in the editor: conflict banner. Reload discards the edit; repeat and Overwrite keeps it.
  5. Delete a term → Undo delete restores it. Edit, then close the window → Save / Discard / Cancel prompt; each works.
  6. Break `dictionary.toml` by hand (Open dictionary file, delete a `]`, save) → dictate: one tray notice, previous terms still apply. Fix the file → dictate a newly added term: it applies.
- [ ] **Step 3: Check the log.** An Explorer-launched `.cmd` copies `%APPDATA%\Murmur\murmur.log` to `target\smoke\murmur.log`. Read it for `reloaded N dictionary terms`, `dictionary editor starting`, and no unexpected errors. Delete the copy afterwards.
- [ ] **Step 4: Clean up.** Remove the test term (`Zanzibar`) in the editor. Jeff quits the dev build and restarts the installed Murmur.
- [ ] **Step 5: Report.** Report each check as passed or failed, with the log evidence. Then stop: the version bump, push, PR and release are Jeff's call.
