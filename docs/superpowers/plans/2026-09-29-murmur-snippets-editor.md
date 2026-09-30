# Murmur Snippets Editor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The dictionary editor window gains a Snippets tab for adding, editing, deleting and testing snippets, opened from a new "Snippets…" tray item; saved edits apply to the next dictation without a restart.

**Architecture:** The existing `--dictionary` editor process becomes a two-tab window (`src/editor.rs`) hosting `DictionaryPanel` and a new `SnippetsPanel`. The snippets panel mirrors the dictionary panel: a working copy validated by egui-free functions in the library (`snippets_edit.rs`), saved through a modified-time check shared with the dictionary (`config::write_if_unchanged`). The pipeline already reloads `snippets.toml` on a changed modified time, so it needs no change.

**Tech Stack:** Rust 2021, eframe/egui 0.36.2 (`glow`, `default_fonts`), `windows` 0.62, `toml` 1.1, `serde`.

**Spec:** `docs/superpowers/specs/2026-09-29-murmur-snippets-editor-design.md`

## Global Constraints

- Branch `feat/snippets-editor` (from `main` at `89d9e5b`; the spec is commit `164d097`). Never push, merge, tag or release: Jeff does that.
- **Build/test command.** MSVC isn't on PATH in this shell (its registration is lost), so plain `cargo` fails with `cl.exe not found` / `link: extra operand`. Always run tests through the vcvars wrapper `target\murmur-test.cmd`, which passes its arguments to `cargo test --release --locked`:
  `cmd //c "C:\Users\JeffLocal\git\murmur\target\murmur-test.cmd" <extra args>` (from Git Bash). If the wrapper is missing, recreate it:
  ```
  @echo off
  call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul
  cd /d C:\Users\JeffLocal\git\murmur
  cargo test --release --locked %*
  ```
  A `sherpa-onnx-sys ... os error 32` means a Murmur dev build is running from `target\release`: stop and ask Jeff to quit it.
- Baseline: lib 114, bin 74, integration 1. All must still pass after every task.
- Commit messages: conventional prefix (`feat:`, `refactor:`, `docs:`, `test:`), ending with the trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` (repo convention).
- UI: dark visuals, colours from `src/correction_ui.rs` (`BG`, `TEXT`, `MUTED`, `GREEN`, `AMBER`) and `RED` from `src/editor_kit.rs`. **Never a repaint loop** (no unconditional `request_repaint` every frame).
- Window title is exactly `Murmur — Dictionary & Snippets` (em dash). The mutex name stays exactly `Local\Murmur.DictionaryEditor`.
- Tray labels are exactly `Dictionary…` and `Snippets…` (ellipsis character).
- Snippets tab copy, exactly: toolbar `+ New snippet`, link `Open snippets file`, field labels `Say` and `Paste`, button `Delete snippet`, test label `Test (snippets only)`, empty list row `(no trigger)`, comments note `Saving removes comments from snippets.toml`, conflict banner `snippets.toml changed outside the editor.`
- Snippet text is saved **exactly as typed** (no trim, no newline changes). Only the trigger is trimmed.
- No test, fixture or doc uses the contents of Jeff's real `snippets.toml` or `dictionary.toml`. Use made-up snippets (`my signature`, `sig`, `my email`).
- Smoke tests that touch the real `%APPDATA%\Murmur`: the Claude desktop app's shell is MSIX-virtualized. Jeff launches the installed build from Explorer; read real AppData files only through an Explorer-launched `.cmd` that copies them under `C:\Users\JeffLocal\git\murmur\target\smoke\`. Unit tests use `std::env::temp_dir()`.

## Review Focus

1. **A signature with blank first/last lines or a `"""` inside it.** Expected: saved and reloaded byte-for-byte. Test: `multi_line_text_round_trips_exactly` in Task 2.
2. **Two triggers that differ only by case or punctuation** (`My email.` and `my email`). Expected: both flagged as duplicates, Save blocked. Test: `duplicate_triggers_ignore_case_and_punctuation` in Task 3.
3. **Tab pressed inside the Paste field.** Expected: focus moves on; no `\t` lands in the pasted text. Test: `tab_in_paste_moves_focus_instead_of_typing_a_tab` in Task 4.
4. **Closing the window while only the tab you're not looking at has edits.** Expected: the Save / Discard / Cancel prompt still appears. Test: `closing_with_only_the_hidden_tab_dirty_asks` in Task 5.
5. **A long multi-line snippet expanded in the test box at the minimum window size.** Expected: Save stays on screen (the preview scrolls). Test: `long_preview_keeps_save_on_screen` in Task 4.

---

## File Structure

| File | Status | Responsibility |
|---|---|---|
| `src/config.rs` | modify | Add `Stamp`, `SaveOutcome`, `file_stamp`, `write_with_backup`, `write_if_unchanged` (shared stamped save). |
| `src/dictionary.rs` | modify | Re-export `Stamp`, `SaveOutcome`, `file_stamp` from `config`; `save_to` uses `write_with_backup`. |
| `src/dictionary_edit.rs` | modify | `changes` becomes generic over `T: PartialEq`. |
| `src/snippets.rs` | modify | `Serialize`; `words`; `to_toml`, `load_stamped`, `save_to`, `save_if_unchanged`. |
| `src/snippets_edit.rs` | create (lib) | egui-free editing rules: `validate`, `normalize`, `visible`, `delete`/`undo`, `changes`, `preview`. |
| `src/lib.rs` | modify | `pub mod snippets_edit;` |
| `src/editor_kit.rs` | create (bin) | `RED`, `open_file`, `has_comments`, shared by both panels. |
| `src/dictionary_panel.rs` | modify | Use `editor_kit`; add a test-only `edit()` hook. |
| `src/snippets_panel.rs` | create (bin) | `SnippetsPanel`: list, editor, test box, save bar. |
| `src/editor.rs` | rename from `src/dictionary_editor.rs` | Two-tab window shell, `Tab`, close prompt over both panels. |
| `src/main.rs` | modify | `wants_editor -> Option<Tab>`, `open_editor`, tray event, update notice, module list. |
| `src/tray.rs` | modify | `OpenSnippets` → `EditSnippets`, label `Snippets…`. |
| `README.md` | modify | Tray row and Snippets section. |

---

### Task 1: Shared stamped save in `config.rs`

**Files:**
- Modify: `src/config.rs` (top imports; new items after `config_dir`; tests module at the end)
- Modify: `src/dictionary.rs:1-46` (imports, `Stamp`/`SaveOutcome`/`file_stamp` definitions), `src/dictionary.rs:123-138` (`save_to`)
- Modify: `src/dictionary_edit.rs:99-114` (`changes`)

**Interfaces:**
- Produces:
  - `pub type config::Stamp = Option<std::time::SystemTime>;`
  - `pub enum config::SaveOutcome { Saved(Stamp), Conflict }` (derives `Debug, PartialEq`)
  - `pub fn config::file_stamp(p: &Path) -> Stamp`
  - `pub fn config::write_with_backup(p: &Path, contents: &str) -> anyhow::Result<()>`
  - `pub fn config::write_if_unchanged(p: &Path, stamp: Stamp, contents: &str) -> anyhow::Result<SaveOutcome>`
  - `dictionary::{Stamp, SaveOutcome, file_stamp}` still resolve (re-export).
  - `pub fn dictionary_edit::changes<T: PartialEq>(loaded: &[T], working: &[T]) -> usize`

- [ ] **Step 1: Write the failing tests** — append inside `mod tests` in `src/config.rs`:

```rust
    fn backup_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("murmur-config-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn write_with_backup_keeps_the_previous_version_and_no_tmp() {
        let p = backup_dir("backup").join("x.toml");
        write_with_backup(&p, "one").unwrap();
        assert!(!p.with_extension("toml.bak").exists(), "nothing to back up on the first write");
        write_with_backup(&p, "two").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "two");
        assert_eq!(std::fs::read_to_string(p.with_extension("toml.bak")).unwrap(), "one");
        assert!(!p.with_extension("toml.tmp").exists());
    }

    #[test]
    fn write_if_unchanged_saves_on_a_matching_stamp_and_conflicts_after_an_outside_write() {
        let p = backup_dir("stamp").join("x.toml");
        assert!(matches!(write_if_unchanged(&p, None, "new").unwrap(), SaveOutcome::Saved(Some(_))), "a missing file saves");
        let stamp = file_stamp(&p);
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&p, "outside").unwrap();
        assert_eq!(write_if_unchanged(&p, stamp, "mine").unwrap(), SaveOutcome::Conflict);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "outside");
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cmd //c "C:\Users\JeffLocal\git\murmur\target\murmur-test.cmd" --lib config::`
Expected: compile error, `cannot find function write_with_backup`.

- [ ] **Step 3: Implement.** In `src/config.rs`, add `use std::time::SystemTime;` to the imports, and after `pub fn config_dir()`:

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

/// Writes `contents` through a `.tmp` and a rename, keeping the previous version as `.bak`.
pub fn write_with_backup(p: &Path, contents: &str) -> Result<()> {
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if p.exists() {
        if let Err(e) = std::fs::copy(p, p.with_extension("toml.bak")) {
            log::warn!("failed to back up {name}: {e}");
        }
    }
    let tmp = p.with_extension("toml.tmp");
    std::fs::write(&tmp, contents).with_context(|| format!("write {name}.tmp"))?;
    std::fs::rename(&tmp, p).with_context(|| format!("rename {name}.tmp"))
}

/// Writes only if the file is still the version stamped at load (or at the last save).
pub fn write_if_unchanged(p: &Path, stamp: Stamp, contents: &str) -> Result<SaveOutcome> {
    if file_stamp(p) != stamp {
        return Ok(SaveOutcome::Conflict);
    }
    write_with_backup(p, contents)?;
    Ok(SaveOutcome::Saved(file_stamp(p)))
}
```

In `src/dictionary.rs`: delete the `Stamp` type, the `SaveOutcome` enum and `fn file_stamp` (lines ~34-46, including their doc comments), delete `use std::time::SystemTime;`, and add `pub use crate::config::{file_stamp, SaveOutcome, Stamp};` after the `use crate::config::config_dir;` line. Replace the body of `save_to` with:

```rust
    pub fn save_to(&self, p: &Path) -> Result<()> {
        if !self.loaded_cleanly {
            return Err(anyhow!("dictionary was not loaded cleanly; fix dictionary.toml first"));
        }
        crate::config::write_with_backup(p, &self.to_toml()?)
    }
```

If `Context` is now unused in `dictionary.rs`, the compiler warns; remove it from the `anyhow` import only if it does.

In `src/dictionary_edit.rs`, make `changes` generic (body unchanged apart from the types):

```rust
/// Items added, removed or edited; an edit counts once (it's one item missing on each side).
pub fn changes<T: PartialEq>(loaded: &[T], working: &[T]) -> usize {
    fn missing<T: PartialEq>(a: &[T], b: &[T]) -> usize {
        let mut pool: Vec<&T> = b.iter().collect();
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
```

- [ ] **Step 4: Run the full suite**

Run: `cmd //c "C:\Users\JeffLocal\git\murmur\target\murmur-test.cmd"`
Expected: lib 116 (114 + 2), bin 74, integration 1, all pass, no new warnings.

- [ ] **Step 5: Commit**

```bash
git add src/config.rs src/dictionary.rs src/dictionary_edit.rs
git commit -m "refactor: share the stamped save between editors

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Snippets save and load

**Files:**
- Modify: `src/snippets.rs` (derives, new functions, `mark` uses `words`, tests)

**Interfaces:**
- Consumes: `config::{Stamp, SaveOutcome, file_stamp, write_with_backup, write_if_unchanged}` (Task 1).
- Produces:
  - `Snippet` and `Snippets` derive `Serialize` (`Snippets` keeps `#[serde(rename = "snippet", default)]` and adds `skip_serializing_if = "Vec::is_empty"`).
  - `pub fn snippets::words(trigger: &str) -> Vec<String>` — the lower-cased, punctuation-stripped words `mark` matches.
  - `impl Snippets { pub fn to_toml(&self) -> Result<String>; pub fn load_stamped(p: &Path) -> Result<(Snippets, Stamp)>; pub fn save_to(&self, p: &Path) -> Result<()>; pub fn save_if_unchanged(&self, p: &Path, stamp: Stamp) -> Result<SaveOutcome>; }`

- [ ] **Step 1: Write the failing tests** — append inside `mod tests` in `src/snippets.rs`:

```rust
    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("murmur-snip-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("snippets.toml")
    }

    #[test]
    fn multi_line_text_round_trips_exactly() {
        let s = set(&[("my signature", "\nBest,\n\nJeff\n"), ("quote", "say \"\"\" then 'x'"), ("sig", "one line")]);
        let toml = s.to_toml().unwrap();
        assert!(toml.contains("\"\"\""), "multi-line text should be a \"\"\" block:\n{toml}");
        assert_eq!(Snippets::from_toml(&toml).unwrap().snippets, s.snippets);
    }

    #[test]
    fn empty_set_saves_as_an_empty_file_that_loads() {
        let toml = Snippets::default().to_toml().unwrap();
        assert!(Snippets::from_toml(&toml).unwrap().snippets.is_empty());
    }

    #[test]
    fn words_match_what_mark_matches() {
        assert_eq!(words("  My email, address. "), vec!["my", "email", "address"]);
        assert!(words(" !? ").is_empty());
    }

    #[test]
    fn load_stamped_missing_file_is_empty_with_no_stamp() {
        let p = temp_path("missing");
        let (s, stamp) = Snippets::load_stamped(&p).unwrap();
        assert!(s.snippets.is_empty());
        assert_eq!(stamp, None);
    }

    #[test]
    fn save_if_unchanged_saves_then_conflicts_after_an_outside_write() {
        let p = temp_path("conflict");
        let s = set(&[("sig", "A\nB")]);
        let SaveOutcome::Saved(stamp) = s.save_if_unchanged(&p, None).unwrap() else { panic!("first save conflicted") };
        let (back, loaded) = Snippets::load_stamped(&p).unwrap();
        assert_eq!(back.snippets, s.snippets);
        assert_eq!(loaded, stamp);
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&p, "[[snippet]]\ntrigger = \"outside\"\ntext = \"x\"\n").unwrap();
        assert_eq!(s.save_if_unchanged(&p, stamp).unwrap(), SaveOutcome::Conflict);
        assert_eq!(Snippets::load_stamped(&p).unwrap().0.snippets[0].trigger, "outside");
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cmd //c "C:\Users\JeffLocal\git\murmur\target\murmur-test.cmd" --lib snippets::`
Expected: compile error, `no method named to_toml`.

- [ ] **Step 3: Implement.** In `src/snippets.rs`:

Imports become:

```rust
use crate::config::{config_dir, file_stamp, SaveOutcome, Stamp};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
```

Derives:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Snippet {
    pub trigger: String,
    pub text: String,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Snippets {
    #[serde(rename = "snippet", default, skip_serializing_if = "Vec::is_empty")]
    pub snippets: Vec<Snippet>,
}
```

After `fn bare`, add:

```rust
/// The words a trigger matches: lower-cased, punctuation stripped, empties dropped.
pub fn words(trigger: &str) -> Vec<String> {
    trigger.split_whitespace().map(bare).filter(|w| !w.is_empty()).collect()
}
```

In `mark`, replace the trigger-word expression `s.trigger.split_whitespace().map(bare).filter(|w| !w.is_empty()).collect::<Vec<_>>()` with `words(&s.trigger)`.

Inside `impl Snippets`, after `from_toml`:

```rust
    pub fn to_toml(&self) -> Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }

    /// Load for editing. The stamp is read before the file, so a write landing mid-read shows
    /// up later as a conflict rather than being silently overwritten.
    pub fn load_stamped(p: &Path) -> Result<(Self, Stamp)> {
        let stamp = file_stamp(p);
        if stamp.is_none() {
            return Ok((Self::default(), None));
        }
        let s = Self::from_toml(&std::fs::read_to_string(p).context("read snippets.toml")?)?;
        Ok((s, stamp))
    }

    pub fn save_to(&self, p: &Path) -> Result<()> {
        crate::config::write_with_backup(p, &self.to_toml()?)
    }

    /// Save only if the file is still the version stamped at load (or at the last save).
    pub fn save_if_unchanged(&self, p: &Path, stamp: Stamp) -> Result<SaveOutcome> {
        crate::config::write_if_unchanged(p, stamp, &self.to_toml()?)
    }
```

(`SystemTime` stays imported: `SnippetFile` uses it.)

- [ ] **Step 4: Run the full suite**

Run: `cmd //c "C:\Users\JeffLocal\git\murmur\target\murmur-test.cmd"`
Expected: lib 121, bin 74, integration 1, all pass. If `multi_line_text_round_trips_exactly` fails on the `"""` assertion only, print `toml` and stop: the spec's file-format promise needs Jeff's call.

- [ ] **Step 5: Commit**

```bash
git add src/snippets.rs
git commit -m "feat(snippets): save and stamped load for the editor

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Snippet editing rules (`snippets_edit.rs`)

**Files:**
- Create: `src/snippets_edit.rs`
- Modify: `src/lib.rs` (add `pub mod snippets_edit;` after `pub mod snippets;`)

**Interfaces:**
- Consumes: `snippets::{Snippet, Snippets, words, restore}` (Task 2), `dictionary_edit::changes` (Task 1), `phonetic::is_common`.
- Produces:
  - `pub enum Field { Trigger, Text }` (`Debug, Clone, Copy, PartialEq`)
  - `pub struct Issue { pub snippet: usize, pub field: Field, pub error: bool, pub message: String }`
  - `pub struct Deleted { pub index: usize, pub snippet: Snippet }`
  - `pub fn validate(list: &[Snippet]) -> Vec<Issue>`
  - `pub fn normalize(list: Vec<Snippet>) -> Vec<Snippet>`
  - `pub fn visible(list: &[Snippet], query: &str) -> Vec<usize>`
  - `pub fn delete(list: &mut Vec<Snippet>, index: usize) -> Deleted`
  - `pub fn undo(list: &mut Vec<Snippet>, d: Deleted) -> usize`
  - `pub use crate::dictionary_edit::changes;`
  - `pub fn preview(list: &[Snippet], text: &str) -> String`

- [ ] **Step 1: Write the file with its tests and stub bodies** — create `src/snippets_edit.rs` with the types, `todo!()` bodies for each function, and this test module; add `pub mod snippets_edit;` to `src/lib.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn s(trigger: &str, text: &str) -> Snippet {
        Snippet { trigger: trigger.into(), text: text.into() }
    }

    fn errors(list: &[Snippet]) -> Vec<(usize, Field)> {
        validate(list).into_iter().filter(|i| i.error).map(|i| (i.snippet, i.field)).collect()
    }

    #[test]
    fn clean_list_has_no_issues() {
        assert!(validate(&[s("my signature", "Best,\nJeff"), s("my email", "me@example.com")]).is_empty());
    }

    #[test]
    fn trigger_without_words_is_an_error() {
        assert_eq!(errors(&[s("  ", "x"), s(" ?! ", "y")]), vec![(0, Field::Trigger), (1, Field::Trigger)]);
    }

    #[test]
    fn empty_or_blank_text_is_an_error() {
        assert_eq!(errors(&[s("sig", ""), s("sig two", " \n\t ")]), vec![(0, Field::Text), (1, Field::Text)]);
    }

    #[test]
    fn duplicate_triggers_ignore_case_and_punctuation() {
        let list = [s("My email.", "a"), s("my  email", "b"), s("my emails", "c")];
        assert_eq!(errors(&list), vec![(0, Field::Trigger), (1, Field::Trigger)]);
    }

    #[test]
    fn common_one_word_trigger_is_a_warning_only() {
        let issues = validate(&[s("All.", "x")]);
        assert_eq!(issues.len(), 1);
        assert!(!issues[0].error);
        assert_eq!(issues[0].field, Field::Trigger);
        assert!(issues[0].message.contains("'all'"), "{}", issues[0].message);
        assert!(validate(&[s("all done", "x")]).is_empty(), "a common word inside a longer trigger is fine");
    }

    #[test]
    fn normalize_trims_the_trigger_and_leaves_text_alone() {
        assert_eq!(normalize(vec![s("  my sig ", "\n  Best, \n")]), vec![s("my sig", "\n  Best, \n")]);
    }

    #[test]
    fn visible_keeps_file_order_and_filters_trigger_and_text() {
        let list = [s("zeta", "Best, Jeff"), s("alpha", "me@example.com"), s("", "")];
        assert_eq!(visible(&list, ""), vec![0, 1, 2]);
        assert_eq!(visible(&list, "BEST"), vec![0]);
        assert_eq!(visible(&list, " alp "), vec![1]);
        assert!(visible(&list, "zzz").is_empty());
    }

    #[test]
    fn delete_then_undo_restores_position_and_clamps() {
        let mut list = vec![s("a", "1"), s("b", "2"), s("c", "3")];
        let before = list.clone();
        let d = delete(&mut list, 1);
        assert_eq!(undo(&mut list, d), 1);
        assert_eq!(list, before);
        let d = delete(&mut list, 2);
        list.clear();
        assert_eq!(undo(&mut list, d), 0);
    }

    #[test]
    fn changes_counts_added_removed_and_edited() {
        let loaded = vec![s("a", "1"), s("b", "2")];
        let mut working = loaded.clone();
        working[0].text = "9".into();
        working.push(s("c", "3"));
        assert_eq!(changes(&loaded, &working), 2);
    }

    #[test]
    fn preview_expands_unsaved_snippets_and_skips_invalid_ones() {
        let list = [s(" sig ", "Best,\nJeff"), s("", "never"), s("oops", "")];
        assert_eq!(preview(&list, "thanks sig."), "thanks Best,\nJeff");
        assert_eq!(preview(&list, "say oops now"), "say oops now");
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cmd //c "C:\Users\JeffLocal\git\murmur\target\murmur-test.cmd" --lib snippets_edit::`
Expected: the tests panic with `not yet implemented`.

- [ ] **Step 3: Implement** — the file above the test module:

```rust
//! Editing rules for the snippets editor, kept free of egui so they can be unit-tested.

use crate::phonetic;
use crate::snippets::{self, Snippet, Snippets};

pub use crate::dictionary_edit::changes;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Field {
    Trigger,
    Text,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    pub snippet: usize,
    pub field: Field,
    /// Errors block Save; warnings don't.
    pub error: bool,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Deleted {
    pub index: usize,
    pub snippet: Snippet,
}

fn fold(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

pub fn validate(list: &[Snippet]) -> Vec<Issue> {
    let mut out = Vec::new();
    for (i, s) in list.iter().enumerate() {
        let w = snippets::words(&s.trigger);
        let trigger = |error, message| Issue { snippet: i, field: Field::Trigger, error, message };
        if w.is_empty() {
            out.push(trigger(true, "Trigger needs at least one word".into()));
        } else if (0..list.len()).any(|j| j != i && snippets::words(&list[j].trigger) == w) {
            out.push(trigger(true, format!("'{}' is already a trigger", w.join(" "))));
        } else if let [word] = w.as_slice() {
            if phonetic::is_common(word) {
                out.push(trigger(false, format!("'{word}' is a common word: this will paste every time you say '{word}'")));
            }
        }
        if s.text.trim().is_empty() {
            out.push(Issue { snippet: i, field: Field::Text, error: true, message: "Text can't be empty".into() });
        }
    }
    out
}

/// What Save writes: the trigger trimmed, the text exactly as typed.
pub fn normalize(list: Vec<Snippet>) -> Vec<Snippet> {
    list.into_iter().map(|s| Snippet { trigger: s.trigger.trim().to_string(), text: s.text }).collect()
}

/// Indices of the snippets to list: matching `query` (trigger or text, ignoring case), in file order.
pub fn visible(list: &[Snippet], query: &str) -> Vec<usize> {
    let q = fold(query);
    (0..list.len()).filter(|&i| q.is_empty() || fold(&list[i].trigger).contains(&q) || fold(&list[i].text).contains(&q)).collect()
}

pub fn delete(list: &mut Vec<Snippet>, index: usize) -> Deleted {
    Deleted { index, snippet: list.remove(index) }
}

pub fn undo(list: &mut Vec<Snippet>, d: Deleted) -> usize {
    let at = d.index.min(list.len());
    list.insert(at, d.snippet);
    at
}

/// `text` with the (unsaved) snippets expanded. Snippets with no trigger words or no text are
/// skipped: one never matches, the other would delete the phrase.
pub fn preview(list: &[Snippet], text: &str) -> String {
    let usable = normalize(list.to_vec())
        .into_iter()
        .filter(|s| !snippets::words(&s.trigger).is_empty() && !s.text.trim().is_empty())
        .collect();
    let (marked, expansions) = Snippets { snippets: usable }.mark(text);
    snippets::restore(&marked, &expansions)
}
```

- [ ] **Step 4: Run the full suite**

Run: `cmd //c "C:\Users\JeffLocal\git\murmur\target\murmur-test.cmd"`
Expected: lib 131, bin 74, integration 1, all pass. If `common_one_word_trigger_is_a_warning_only` fails because `all` isn't in `common_words.txt`, pick a word that `phonetic::tests` already asserts is common (e.g. `the`) and keep the rest of the test.

- [ ] **Step 5: Commit**

```bash
git add src/snippets_edit.rs src/lib.rs
git commit -m "feat(snippets): editing rules for the snippets editor

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Snippets panel

**Files:**
- Create: `src/editor_kit.rs`, `src/snippets_panel.rs`
- Modify: `src/dictionary_panel.rs:16` (`RED`), `:63-66` (`open_file`), `:137-139` (comment detection), imports; add a test-only hook
- Modify: `src/main.rs:10-11` (module list: add `mod editor_kit;` and `mod snippets_panel;`, alphabetical)

**Interfaces:**
- Consumes: `snippets::{Snippet, Snippets}` + `load_stamped`/`save_to`/`save_if_unchanged` (Task 2); `snippets_edit::*` (Task 3); `config::{file_stamp, SaveOutcome, Stamp}` (Task 1).
- Produces:
  - `editor_kit::RED: egui::Color32`, `editor_kit::open_file(p: &Path)`, `editor_kit::has_comments(p: &Path) -> bool`
  - `pub struct SnippetsPanel` with `pub fn new(path: PathBuf) -> Self`, `pub fn ui(&mut self, ui: &mut egui::Ui)`, `pub fn is_dirty(&self) -> bool`, `pub fn save(&mut self) -> bool`, and `#[cfg(test)] pub(crate) fn edit(&mut self) -> &mut Vec<Snippet>`
  - `DictionaryPanel` gains `#[cfg(test)] pub(crate) fn edit(&mut self) -> &mut Vec<Term>`
  - Panel IDs used: `snip-toolbar`, `snip-footer`, `snip-list` (distinct from the dictionary's `dict-*`).

- [ ] **Step 1: Extract `editor_kit`.** Create `src/editor_kit.rs`:

```rust
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
```

In `src/dictionary_panel.rs`: delete `const RED`, `fn open_file`, add `use crate::editor_kit::{has_comments, open_file, RED};`, and replace the `self.has_comments = std::fs::read_to_string(...)...unwrap_or(false);` expression in `reload` with `self.has_comments = has_comments(&self.path);`. If `Path` becomes unused outside the tests, drop it from the top-level `std::path` import and add `use std::path::Path;` inside the test module (its `on_disk` helper uses it). Add, inside `impl DictionaryPanel`:

```rust
    /// The working copy, for the editor window's tests.
    #[cfg(test)]
    pub(crate) fn edit(&mut self) -> &mut Vec<Term> {
        &mut self.working
    }
```

Add `mod editor_kit;` and `mod snippets_panel;` to `src/main.rs`. Create `src/snippets_panel.rs` containing only `//! placeholder` for now, so the tree compiles. Run the suite: bin 74 pass (pure move). Don't commit yet.

- [ ] **Step 2: Write the failing panel tests** — replace `src/snippets_panel.rs` with the struct and `impl` skeleton from Step 4 with `todo!()` bodies, plus this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

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
```

- [ ] **Step 3: Run to verify they fail**

Run: `cmd //c "C:\Users\JeffLocal\git\murmur\target\murmur-test.cmd" --bin murmur snippets_panel::`
Expected: the tests panic with `not yet implemented`.

- [ ] **Step 4: Implement** — the full `src/snippets_panel.rs` above the tests:

```rust
//! The snippets editor's content: snippet list, snippet editor, test box and save bar. Draws
//! into any `egui::Ui`; the editor window hosts it as a tab beside the dictionary.

use crate::correction_ui::{AMBER, GREEN, MUTED, TEXT};
use crate::editor_kit::{has_comments, open_file, RED};
use eframe::egui::{self, Align, Button, CentralPanel, Frame, Key, Layout, Margin, Modifiers, Panel, RichText, ScrollArea, TextEdit};
use murmur_lib::config::{file_stamp, SaveOutcome, Stamp};
use murmur_lib::snippets::{Snippet, Snippets};
use murmur_lib::snippets_edit::{self as edit, Deleted, Field, Issue};
use std::path::{Path, PathBuf};

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
    /// Scroll the selected row into view on the next draw.
    reveal: bool,
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
            reveal: false,
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
                self.fresh = None;
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

    /// Adds a snippet at the bottom and puts the cursor in its trigger. A search that found
    /// nothing becomes the trigger, so the phrase isn't typed twice.
    fn new_snippet(&mut self) {
        let query = self.search.trim();
        let trigger = if edit::visible(&self.working, query).is_empty() { query.to_string() } else { String::new() };
        self.working.push(Snippet { trigger, text: String::new() });
        let i = self.working.len() - 1;
        self.selected = Some(i);
        self.fresh = Some(i);
        self.focus = Some(Focus::Trigger);
        self.reveal = true;
        self.search.clear();
    }

    fn select(&mut self, i: usize) {
        if self.selected != Some(i) {
            self.selected = Some(i);
            self.fresh = None;
        }
    }

    fn delete_selected(&mut self) {
        if let Some(i) = self.selected.filter(|&i| i < self.working.len()) {
            self.undo = Some(edit::delete(&mut self.working, i));
            self.selected = None;
            self.fresh = None;
        }
    }

    fn undo_delete(&mut self) {
        if let Some(d) = self.undo.take() {
            self.fresh = None;
            self.selected = Some(edit::undo(&mut self.working, d));
            self.reveal = true;
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
        let rows = edit::visible(&self.working, &self.search);
        if rows.is_empty() && !self.search.trim().is_empty() {
            ui.label(RichText::new("No snippet matches.").color(MUTED));
            if ui.button(format!("+ Add '{}'", self.search.trim())).on_hover_text("Or press Enter in Search").clicked() {
                self.new_snippet();
            }
            return;
        }
        let mut clicked = None;
        for i in rows {
            let s = &self.working[i];
            let bad = issues.iter().any(|x| x.error && x.snippet == i);
            let text = if s.trigger.trim().is_empty() {
                RichText::new("(no trigger)").color(if bad { RED } else { MUTED })
            } else {
                RichText::new(s.trigger.trim()).color(if bad { RED } else { TEXT })
            };
            let r = ui.selectable_label(self.selected == Some(i), text);
            if self.reveal && self.selected == Some(i) {
                r.scroll_to_me(None);
            }
            if r.clicked() {
                clicked = Some(i);
            }
        }
        self.reveal = false;
        if let Some(i) = clicked {
            self.select(i);
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
        let enter = ui.input(|x| x.key_pressed(Key::Enter));
        let mut delete = false;
        let s = &mut self.working[i];

        ui.label(RichText::new("Say").size(12.0).color(MUTED));
        let trigger =
            ui.add(TextEdit::singleline(&mut s.trigger).id_salt(("trigger", i)).hint_text("e.g. my signature").desired_width(f32::INFINITY));
        if focus == Some(Focus::Trigger) {
            trigger.request_focus();
        }
        issues.iter().filter(|x| x.snippet == i && x.field == Field::Trigger).for_each(|x| show(ui, x));
        ui.add_space(6.0);

        ui.label(RichText::new("Paste").size(12.0).color(MUTED));
        // Tab isn't captured (egui's default), so it moves focus on instead of typing a tab
        let text = ui.add(TextEdit::multiline(&mut s.text).id_salt(("text", i)).desired_rows(5).desired_width(f32::INFINITY));
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
```

`Path` is used only by the tests' `on_disk`; if the compiler warns it's unused in non-test builds, move `Path` into the test module's imports (`use std::path::Path;`).

- [ ] **Step 5: Run the full suite**

Run: `cmd //c "C:\Users\JeffLocal\git\murmur\target\murmur-test.cmd"`
Expected: lib 131, bin 90 (74 + 16), integration 1, all pass, no warnings. `SnippetsPanel` is not yet reachable from `main`, so a `dead_code` warning on `SnippetsPanel::new`/`ui`/`is_dirty`/`save` is expected until Task 5; nothing else.

If `a_snippet_can_be_typed_without_the_mouse_and_proves_itself` fails only on the preview assertion, draw one more frame before asserting (as the dictionary test does for its proof line). If `layout_fits_the_window_with_a_vertical_list` finds `Delete snippet` off-screen at `SMALLEST`, stop and report the measured positions rather than shrinking the Paste field or dropping the assertion (the spec's 5-row minimum is Jeff's call). If `tab_in_paste_moves_focus_instead_of_typing_a_tab` finds a `\t` in the text, stop and report: egui's default tab handling differs from what the spec assumes.

- [ ] **Step 6: Commit**

```bash
git add src/editor_kit.rs src/snippets_panel.rs src/dictionary_panel.rs src/main.rs
git commit -m "feat(editor): snippets panel

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Two-tab editor window, tray and launch

**Files:**
- Rename: `src/dictionary_editor.rs` → `src/editor.rs` (`git mv`)
- Modify: `src/editor.rs` (whole shell), `src/main.rs:10` (module), `:65-69` (`EDITOR_ARG`/`wants_editor`), `:121-126` (startup), `:285-293` (tray events), `:426-429` (update guard), `:529-534` (test)
- Modify: `src/tray.rs:16` (`OpenSnippets`), `:69` (label), `:83` (id map)

**Interfaces:**
- Consumes: `DictionaryPanel` (+ test `edit()`), `SnippetsPanel` (+ test `edit()`) from Task 4.
- Produces:
  - `pub enum editor::Tab { Dictionary, Snippets }` with `pub fn flag(self) -> &'static str` (`"--dictionary"` / `"--snippets"`) and `pub fn from_flag(s: &str) -> Option<Tab>`
  - `pub fn editor::run(tab: Tab) -> anyhow::Result<()>`, `pub fn editor::is_open() -> bool`
  - `TrayEvent::EditSnippets` (replaces `OpenSnippets`)

- [ ] **Step 1: Rename and write the failing shell tests.** `git mv src/dictionary_editor.rs src/editor.rs`; in `src/main.rs` change `mod dictionary_editor;` to `mod editor;` (keep the list alphabetical) and `dictionary_editor::` to `editor::` at its two uses (`run()` keeps its signature until Step 3). Replace the test module of `src/editor.rs` with:

```rust
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
```

- [ ] **Step 2: Run to verify they fail**

Run: `cmd //c "C:\Users\JeffLocal\git\murmur\target\murmur-test.cmd" --bin murmur editor::`
Expected: compile errors, `no function or associated item named new`, `no variant Tab`, `no method draw`.

- [ ] **Step 3: Implement the shell.** Replace `src/editor.rs` above the tests with:

```rust
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
            Panel::top("editor-tabs").frame(Frame::new().inner_margin(Margin { left: 0, right: 0, top: 0, bottom: 6 })).resizable(false).show(ui, |ui| {
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
```

(The eframe app id stays `"murmur-dictionary"` so the saved window size and position carry over.)

- [ ] **Step 4: Wire `main.rs` and the tray.** In `src/main.rs`:

Replace `const EDITOR_ARG` and `wants_editor` with:

```rust
/// The editor tab asked for on the command line, or `None` for the app itself.
fn wants_editor(mut args: impl Iterator<Item = String>) -> Option<editor::Tab> {
    args.nth(1).as_deref().and_then(editor::Tab::from_flag)
}

/// Opens the editor on `tab` as a separate process, so dictation keeps working while it's open.
fn open_editor(tray: &Tray, tab: editor::Tab) {
    let spawned = std::env::current_exe().and_then(|exe| std::process::Command::new(exe).arg(tab.flag()).spawn());
    if let Err(e) = spawned {
        log::error!("editor: {e}");
        tray.notify("Murmur", &format!("Couldn't open the editor: {e}"));
    }
}
```

Start of `main`:

```rust
    let editor_tab = wants_editor(std::env::args());
    init_logging(editor_tab.is_none());
    // the editor is its own process, so it must not take the app's single-instance mutex
    if let Some(tab) = editor_tab {
        return editor::run(tab);
    }
```

Tray events (replacing the `EditDictionary` block and the `OpenSnippets` arm):

```rust
                TrayEvent::EditDictionary => open_editor(&tray, editor::Tab::Dictionary),
                TrayEvent::EditSnippets => open_editor(&tray, editor::Tab::Snippets),
```

Update guard:

```rust
                // the installer would force-close the editor and lose unsaved edits
                UpdateMsg::Downloaded(_) if editor::is_open() => {
                    offer.failed();
                    restore_update_item(&tray, &offer, installed_copy);
                    tray.notify("Close the Dictionary & Snippets window to update", "Then choose Update again.");
                }
```

Replace the test `dictionary_flag_selects_the_editor` with:

```rust
    #[test]
    fn editor_flags_select_the_tab() {
        assert_eq!(wants_editor(args(&["murmur.exe", "--dictionary"])), Some(editor::Tab::Dictionary));
        assert_eq!(wants_editor(args(&["murmur.exe", "--snippets"])), Some(editor::Tab::Snippets));
        assert_eq!(wants_editor(args(&["murmur.exe"])), None);
        assert_eq!(wants_editor(args(&["murmur.exe", "--other"])), None);
    }
```

`open_path` stays (Open config folder uses it); `snippets` stays imported (`ensure_file`).

In `src/tray.rs`: rename the variant `OpenSnippets` to `EditSnippets` (enum and the id map), and the menu label `"Open snippets"` to `"Snippets…"`.

- [ ] **Step 5: Run the full suite**

Run: `cmd //c "C:\Users\JeffLocal\git\murmur\target\murmur-test.cmd"`
Expected: lib 131, bin 96 (90 − 2 old shell tests + 8 shell tests; the flag test is replaced one for one), integration 1, all pass, **no warnings** (the Task 4 dead-code warnings are gone). If `closing_with_only_the_hidden_tab_dirty_asks` fails because `close_requested()` is false, check how `egui::ViewportInfo::close_requested` reads `events` in egui 0.36.2 and set the input accordingly; don't weaken the assertion.

- [ ] **Step 6: Commit**

```bash
git add -A src/editor.rs src/dictionary_editor.rs src/main.rs src/tray.rs
git commit -m "feat(editor): Dictionary and Snippets tabs, Snippets… in the tray

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: README and smoke test

**Files:**
- Modify: `README.md:57` (tray row), `README.md:97-107` (Snippets section)

- [ ] **Step 1: Update the README.** Tray table row:

```markdown
| Snippets… | Add, edit, delete and test snippets |
```

Snippets section (replace the paragraph and keep the TOML example under a power-user note):

````markdown
### Snippets

Say a trigger phrase anywhere in a dictation and a saved block of text is pasted instead, exactly as written. Open them from the tray with **Snippets…**: pick one to edit it, **+ New snippet** to add one (**Say** is the trigger, **Paste** is the text), and type or dictate into the test box to see what the snippets do to a phrase. Triggers match whole words, ignoring case and punctuation. Changes apply to the next dictation once you save; dictation keeps working while the window is open.

Snippets live in `%APPDATA%\Murmur\snippets.toml`, which you can also edit by hand:

```toml
[[snippet]]
trigger = "my signature"
text = """
Best,
Your Name"""
```
````

Check the Dictionary section (line ~82) still reads correctly with "the editor" now being a two-tab window; change "while the editor is open" to "while the window is open" only if it reads wrong.

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "docs: snippets editor in the README

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 3: Release build for the smoke test**

Run (Git Bash): create `target\murmur-build.cmd` with the same three header lines as `murmur-test.cmd` and `cargo build --release --locked` as the last line, then `cmd //c "C:\Users\JeffLocal\git\murmur\target\murmur-build.cmd"`.
Expected: `Finished release`, no warnings.

- [ ] **Step 4: Hand the smoke test to Jeff** (the shell can't touch the real AppData or drive the tray). Report "clean build, not smoke tested" and give him this checklist to run against `target\release\murmur.exe` launched from Explorer (quit the installed Murmur first):
  1. Tray → **Snippets…** opens "Murmur — Dictionary & Snippets" on the Snippets tab; Tray → **Dictionary…** while it's open brings the same window forward.
  2. **+ New snippet**, Say `test signature`, Paste two lines, Save → dictate "thanks test signature" into Notepad: the two lines are pasted, no restart.
  3. Open `snippets.toml` (link in the tab): the text is a `"""` block.
  4. Edit a snippet without saving, hand-edit and save the file in Notepad, then Save in the editor: the conflict banner appears; Reload and Overwrite both work.
  5. Unsaved edits on both tabs, close the window: one prompt naming both; Save writes both files.
  6. Dictionary tab: add, test and save a term as before.
  7. Delete the test snippet and Save.

---

## Self-review notes

- Spec coverage: window/tabs/dot/Ctrl+S/close prompt/updater notice → Task 5; layout, open, edit, validation, test box, save/conflict, comments note → Task 4; file format and round-trip → Task 2; validation rules → Task 3; shared stamped save → Task 1; README → Task 6. The pipeline needs no change (already reloads on modified time).
- Deviation from spec, recorded there: only `RED`, `open_file`, `has_comments` move to `editor_kit` (the snippets panel has no motion).
- Save button: enabled while a new snippet's errors are still held back, so pressing it reveals them instead of Save being greyed out with no visible reason.
