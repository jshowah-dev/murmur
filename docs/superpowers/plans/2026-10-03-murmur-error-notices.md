# Murmur Error Notices Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every error Murmur shows says what happened (with toml's line and reason), what Murmur did, and what to do; errors from a dictation appear above the pill, startup and file errors in one balloon whose click opens the files in Notepad.

**Architecture:** A new pure module `notice` in the library builds every message (`FileProblem`, `Balloon`, pill constants). The tray shows a `Balloon` and remembers what a click opens. The pipeline sends file reload problems as their own message, so they no longer disturb the dictation's landing; `PipelineMsg::Error` now carries only pill text.

**Tech Stack:** Rust 2021, anyhow, toml 1.1.6 (`toml::de::Error::{message, span}`), tray-icon 0.25 + `Shell_NotifyIconW` balloons, the existing mote/pill.

**Spec:** `docs/superpowers/specs/2026-10-03-murmur-error-notices-design.md`

## Global Constraints

- Balloon body ≤ 255 UTF-16 units; balloon title ≤ 63 UTF-16 units (Windows cuts them there).
- Copy is verbatim from the spec's Behaviour table. Pill texts: `Can't open the microphone`, `Couldn't load the speech model. Try again.`, `Something went wrong. Try again.`
- The full error chain (`{e:#}`) is still logged wherever an error is turned into a notice.
- Background events (update, model download, mic lost on its own) and tray-action errors (`open_editor`, saving the talk key, autostart) keep their current balloons, except that a mic that fails to reopen uses the new mic balloon copy.
- A balloon click opens files with `notepad.exe <path>`, one process per path.
- No first-person copy ("I couldn't…"). No new dependencies.
- Spec deviation, mechanical: the spec's single `KeepingPrevious` effect is split into `KeepingTerms` and `KeepingSnippets` so each says its own noun.

## Review Focus

1. **Startup balloons replace each other.** With a broken file *and* a model upgrade on offer, the upgrade balloon (shown after) hid the error. Expected: the error is the one left on screen. Pinned by `startup_shows_what_matters_most_last` (Task 1).
2. **A span at the end of the file, or after non-ASCII text.** `line_of` must count bytes up to the offset without slicing a `str` mid-character and without panicking at `offset == len`. Pinned by `line_of_counts_lines_up_to_the_offset` (Task 1).
3. **A file that can't be read at all** (locked, access denied). Expected: the OS reason, no "Line" prefix, no panic. Pinned by `a_non_toml_error_gives_its_root_cause_and_no_line` (Task 1).
4. **Long paths and reasons, and characters that take two UTF-16 units.** Expected: every body ≤ 255 units and every title ≤ 63, the cut ending in `…`. Pinned by `every_balloon_fits_windows_limits` (Task 1).
5. **A balloon clicked after a different kind replaced it.** Expected: the click does what the balloon on screen offers (an update balloon after a file balloon updates; it doesn't open files). Pinned by `a_click_does_what_the_last_balloon_offers` (Task 2).

---

## Before you start

The worktree is `C:\Users\JeffLocal\git\murmur\.claude\worktrees\startup-errors`, branch `feat/startup-errors`. Run everything from it.

From a Claude session, `cmd.exe`/`vcvars64.bat` are refused. Use the bash wrapper `mcargo.sh` in the session scratchpad (it puts MSVC 14.44.35207 and Windows Kits 10.0.26100.0 on `PATH`/`LIB`/`INCLUDE` and sets `CARGO_TARGET_DIR=../../../target`). Below, `$MCARGO` means `bash <scratchpad>/mcargo.sh`.

Baseline: `$MCARGO test --release --locked` → all pass (lib 171, bin 186, stt_integration 1 at v0.4.17). If lib tests crash with `ORT Version is: 1.17.1` / `STATUS_ACCESS_VIOLATION`, copy `target\release\onnxruntime*.dll` into `target\release\deps\` (environment, not code).

Commits end with the trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`, as the repo's recent commits do.

---

### Task 1: The `notice` module

**Files:**
- Create: `src/notice.rs`
- Modify: `src/lib.rs` (add `pub mod notice;` in alphabetical order, after `pub mod model_fetch;`)

**Interfaces:**
- Consumes: nothing new.
- Produces (all `pub` in `murmur_lib::notice`):
  - `const BODY_MAX: usize = 255`, `const TITLE_MAX: usize = 63`
  - `const MIC: &str`, `const MODEL_LOAD: &str`, `const RESTARTED: &str`
  - `enum Effect { Defaults, CorrectionsOff, SnippetsOff, KeepingTerms, KeepingSnippets }` (`Debug, Clone, Copy, PartialEq, Eq`)
  - `struct FileProblem { pub path: PathBuf, pub line: Option<usize>, pub reason: String, pub effect: Effect }` (`Debug, Clone, PartialEq`)
  - `struct Balloon { pub title: String, pub body: String, pub open: Vec<PathBuf> }` (`Debug, Clone, PartialEq`)
  - `fn line_of(text: &str, offset: usize) -> usize`
  - `fn file_problem(path: &Path, e: &anyhow::Error, effect: Effect) -> FileProblem`
  - `fn files_balloon(problems: &[FileProblem]) -> Balloon`
  - `fn reload_balloon(p: &FileProblem) -> Balloon`
  - `fn model_missing_balloon(model_dir: &Path, config: &Path) -> Balloon`
  - `fn mic_balloon(key: &str) -> Balloon`
  - `fn upgrade_balloon(mb: u64) -> Balloon`
  - `fn startup(problems: &[FileProblem], mic: Option<Balloon>, model_missing: Option<Balloon>, upgrade: Option<Balloon>) -> Vec<Balloon>`
  - `fn not_learned(p: &FileProblem) -> String`

- [ ] **Step 1: Write the failing tests**

Create `src/notice.rs` with only the test module (and the `use` lines it needs), and add `pub mod notice;` to `src/lib.rs`:

```rust
//! What Murmur says when something goes wrong: what happened, what Murmur did about it, and
//! what you can do. Text only; the tray and the pill show it.

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp(name: &str, text: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("murmur-notice-{}-{name}", std::process::id()));
        std::fs::write(&p, text).unwrap();
        p
    }

    fn toml_error(text: &str) -> anyhow::Error {
        anyhow::Error::from(toml::from_str::<toml::Table>(text).unwrap_err()).context("parse config.toml")
    }

    fn problem(name: &str, line: Option<usize>, effect: Effect) -> FileProblem {
        FileProblem { path: PathBuf::from(format!("C:\\Users\\a\\AppData\\Roaming\\Murmur\\{name}")), line, reason: "expected `=`.".into(), effect }
    }

    fn units(s: &str) -> usize {
        s.encode_utf16().count()
    }

    #[test]
    fn line_of_counts_lines_up_to_the_offset() {
        assert_eq!(line_of("a = 1\nb\n", 0), 1);
        assert_eq!(line_of("a = 1\nb\n", 6), 2);
        assert_eq!(line_of("a = 1\r\nb = 2\r\nc\r\n", 14), 3);
        // past the end, and just after a two-byte character: no panic
        assert_eq!(line_of("é\nx", 3), 2);
        assert_eq!(line_of("é\nx", 99), 2);
        assert_eq!(line_of("é", 1), 1);
    }

    #[test]
    fn a_toml_error_gives_its_message_and_line_not_the_context() {
        let text = "# a\n# b\n# c\nbroken\n";
        let p = temp("line4.toml", text);
        let e = toml_error(text);
        let want = e.chain().find_map(|c| c.downcast_ref::<toml::de::Error>()).unwrap().message().to_string();
        let got = file_problem(&p, &e, Effect::Defaults);
        assert_eq!(got.line, Some(4));
        assert!(got.reason.starts_with(want.trim_end_matches('.')), "{:?} vs {want:?}", got.reason);
        assert!(!got.reason.contains("parse config.toml"));
        assert!(got.reason.ends_with('.'));
        assert_eq!(got.path, p);
        assert_eq!(got.effect, Effect::Defaults);
    }

    #[test]
    fn a_non_toml_error_gives_its_root_cause_and_no_line() {
        let io = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "Access is denied. (os error 5)");
        let e = anyhow::Error::from(io).context("read config.toml");
        let got = file_problem(std::path::Path::new("C:\\nowhere\\config.toml"), &e, Effect::Defaults);
        assert_eq!(got.line, None);
        assert_eq!(got.reason, "Access is denied. (os error 5)");
    }

    #[test]
    fn one_broken_file_says_what_murmur_did() {
        let b = files_balloon(&[problem("config.toml", Some(4), Effect::Defaults)]);
        assert_eq!(b.title, "Murmur couldn't read config.toml");
        assert_eq!(b.body, "Line 4: expected `=`. Using default settings for now. Click to open it.");
        assert_eq!(b.open, vec![PathBuf::from("C:\\Users\\a\\AppData\\Roaming\\Murmur\\config.toml")]);
        let b = files_balloon(&[problem("dictionary.toml", Some(12), Effect::CorrectionsOff)]);
        assert_eq!(b.body, "Line 12: expected `=`. Corrections are off until it's fixed. Click to open it.");
        let mut p = problem("snippets.toml", None, Effect::SnippetsOff);
        p.reason = "Access is denied. (os error 5)".into();
        assert_eq!(files_balloon(&[p]).body, "Access is denied. (os error 5) Snippets are off. Click to open it.");
    }

    #[test]
    fn several_broken_files_share_one_balloon() {
        let b = files_balloon(&[problem("config.toml", Some(4), Effect::Defaults), problem("dictionary.toml", Some(12), Effect::CorrectionsOff)]);
        assert_eq!(b.title, "Murmur couldn't read 2 files");
        assert_eq!(
            b.body,
            "config.toml line 4: expected `=`. Using defaults.\ndictionary.toml line 12: expected `=`. Corrections off.\nClick to open them."
        );
        assert_eq!(b.open.len(), 2);
    }

    #[test]
    fn a_reload_problem_says_the_previous_ones_are_kept() {
        let b = reload_balloon(&problem("dictionary.toml", Some(4), Effect::KeepingTerms));
        assert_eq!(b.title, "dictionary.toml has a mistake");
        assert_eq!(b.body, "Line 4: expected `=`. Still using your previous terms. Click to open it.");
        let b = reload_balloon(&problem("snippets.toml", None, Effect::KeepingSnippets));
        assert_eq!(b.title, "Murmur couldn't read snippets.toml");
        assert_eq!(b.body, "expected `=`. Still using your previous snippets. Click to open it.");
    }

    #[test]
    fn model_missing_and_mic_say_what_to_do() {
        let cfg = PathBuf::from("C:\\m\\config.toml");
        let b = model_missing_balloon(std::path::Path::new("D:\\models\\parakeet"), &cfg);
        assert_eq!(b.title, "Speech model not found");
        assert_eq!(b.body, "Nothing at D:\\models\\parakeet (model_dir in config.toml). Click to open config.toml.");
        assert_eq!(b.open, vec![cfg]);
        let b = mic_balloon("Right Ctrl");
        assert_eq!(b.title, "Can't open the microphone");
        assert_eq!(b.body, "Check one is connected and allowed in Windows privacy settings. Murmur tries again when you press Right Ctrl.");
        assert!(b.open.is_empty());
        let b = upgrade_balloon(640);
        assert_eq!(b.title, "A new speech model is available (640 MB)");
        assert_eq!(b.body, "Download from the tray menu.");
        assert!(b.open.is_empty());
    }

    #[test]
    fn every_balloon_fits_windows_limits() {
        let long = "🎤".repeat(300);
        let mut ps: Vec<FileProblem> = ["config.toml", "dictionary.toml", "snippets.toml"]
            .iter()
            .map(|n| FileProblem { reason: long.clone(), ..problem(n, Some(4), Effect::Defaults) })
            .collect();
        let deep = PathBuf::from(format!("C:\\{}\\parakeet", "deep\\".repeat(80)));
        let all = [
            files_balloon(&ps[..1]),
            files_balloon(&ps),
            reload_balloon(&ps[0]),
            model_missing_balloon(&deep, &PathBuf::from("C:\\m\\config.toml")),
            mic_balloon(&long),
        ];
        for b in &all {
            assert!(units(&b.body) <= BODY_MAX, "{} units: {}", units(&b.body), b.body);
            assert!(units(&b.title) <= TITLE_MAX, "{}", b.title);
        }
        // a cut ends in an ellipsis; the path keeps its end, where the folder name is
        assert!(all[0].body.ends_with('…') || all[0].body.ends_with("Click to open it."));
        assert!(all[3].body.contains("parakeet (model_dir in config.toml). Click to open config.toml."));
        // many files: each reason is trimmed first, so the click line survives
        ps.truncate(2);
        assert!(files_balloon(&ps).body.ends_with("Click to open them."));
    }

    #[test]
    fn startup_shows_what_matters_most_last() {
        let files = [problem("config.toml", Some(4), Effect::Defaults)];
        let mic = mic_balloon("Right Ctrl");
        let model = model_missing_balloon(std::path::Path::new("D:\\x"), std::path::Path::new("C:\\m\\config.toml"));
        let up = upgrade_balloon(640);
        let shown = startup(&files, Some(mic.clone()), Some(model.clone()), Some(up.clone()));
        // each balloon replaces the last, so the order runs from least to most important
        assert_eq!(shown, vec![up.clone(), files_balloon(&files), mic, model]);
        assert_eq!(startup(&[], None, None, Some(up.clone())), vec![up]);
        assert!(startup(&[], None, None, None).is_empty());
    }

    #[test]
    fn not_learned_names_the_file_and_line() {
        assert_eq!(not_learned(&problem("dictionary.toml", Some(12), Effect::CorrectionsOff)), "Not learned: dictionary.toml line 12 has a mistake");
        assert_eq!(not_learned(&problem("dictionary.toml", None, Effect::CorrectionsOff)), "Not learned: dictionary.toml can't be read");
    }

    #[test]
    fn pill_texts() {
        assert_eq!(MIC, "Can't open the microphone");
        assert_eq!(MODEL_LOAD, "Couldn't load the speech model. Try again.");
        assert_eq!(RESTARTED, "Something went wrong. Try again.");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `$MCARGO test --release --locked --lib notice`
Expected: compile errors — `cannot find function line_of`, `cannot find type FileProblem`, etc.

- [ ] **Step 3: Write the implementation**

Insert above the test module in `src/notice.rs`:

```rust
use std::path::{Path, PathBuf};

/// Windows cuts a balloon's text at 255 UTF-16 units and its title at 63.
pub const BODY_MAX: usize = 255;
pub const TITLE_MAX: usize = 63;

/// Said above the pill when the mic won't open as you press the talk key.
pub const MIC: &str = "Can't open the microphone";
/// Said above the pill when the speech model fails to load for a dictation.
pub const MODEL_LOAD: &str = "Couldn't load the speech model. Try again.";
/// Said above the pill when the pipeline panicked and restarted.
pub const RESTARTED: &str = "Something went wrong. Try again.";

/// Longest a file's reason runs: alone in a balloon, and as one of several.
const REASON_ONE: usize = 150;
const REASON_MANY: usize = 60;

/// What Murmur does while a file can't be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Defaults,
    CorrectionsOff,
    SnippetsOff,
    KeepingTerms,
    KeepingSnippets,
}

impl Effect {
    fn long(self) -> &'static str {
        match self {
            Effect::Defaults => "Using default settings for now.",
            Effect::CorrectionsOff => "Corrections are off until it's fixed.",
            Effect::SnippetsOff => "Snippets are off.",
            Effect::KeepingTerms => "Still using your previous terms.",
            Effect::KeepingSnippets => "Still using your previous snippets.",
        }
    }

    fn short(self) -> &'static str {
        match self {
            Effect::Defaults => "Using defaults.",
            Effect::CorrectionsOff => "Corrections off.",
            Effect::SnippetsOff => "Snippets off.",
            Effect::KeepingTerms => "Previous terms kept.",
            Effect::KeepingSnippets => "Previous snippets kept.",
        }
    }
}

/// A settings file Murmur couldn't use: which, where in it, why, and what Murmur did instead.
#[derive(Debug, Clone, PartialEq)]
pub struct FileProblem {
    pub path: PathBuf,
    pub line: Option<usize>,
    pub reason: String,
    pub effect: Effect,
}

impl FileProblem {
    fn name(&self) -> String {
        self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| self.path.display().to_string())
    }

    /// "Line 4: expected `=`." or just the reason.
    fn cause(&self, max: usize) -> String {
        let reason = cut(&self.reason, max);
        match self.line {
            Some(n) => format!("Line {n}: {reason}"),
            None => reason,
        }
    }
}

/// A corner balloon, and the files a click on it opens (none: a click does nothing).
#[derive(Debug, Clone, PartialEq)]
pub struct Balloon {
    pub title: String,
    pub body: String,
    pub open: Vec<PathBuf>,
}

/// The 1-based line holding byte `offset` of `text`.
pub fn line_of(text: &str, offset: usize) -> usize {
    text.as_bytes()[..offset.min(text.len())].iter().filter(|&&b| b == b'\n').count() + 1
}

/// What went wrong with the file at `path`: toml's own message and line when it's a parse
/// error, otherwise the root cause (an OS error, say). The line is found by reading the file again.
pub fn file_problem(path: &Path, e: &anyhow::Error, effect: Effect) -> FileProblem {
    let parse = e.chain().find_map(|c| c.downcast_ref::<toml::de::Error>());
    let (line, reason) = match parse {
        Some(t) => {
            let line = t.span().and_then(|s| std::fs::read_to_string(path).ok().map(|text| line_of(&text, s.start)));
            (line, t.message().to_string())
        }
        None => (None, e.root_cause().to_string()),
    };
    FileProblem { path: path.to_path_buf(), line, reason: sentence(&reason), effect }
}

/// The startup balloon for one or more files that couldn't be read.
pub fn files_balloon(problems: &[FileProblem]) -> Balloon {
    let open = problems.iter().map(|p| p.path.clone()).collect();
    if let [p] = problems {
        let body = fit(&[p.cause(REASON_ONE), p.effect.long().into()], " ", "Click to open it.");
        return Balloon { title: title(&format!("Murmur couldn't read {}", p.name())), body, open };
    }
    let lines: Vec<String> = problems
        .iter()
        .map(|p| {
            let at = p.line.map(|n| format!(" line {n}")).unwrap_or_default();
            format!("{}{at}: {} {}", p.name(), cut(&p.reason, REASON_MANY), p.effect.short())
        })
        .collect();
    let body = fit(&lines, "\n", "Click to open them.");
    Balloon { title: format!("Murmur couldn't read {} files", problems.len()), body, open }
}

/// A file edited while Murmur runs turned out broken; the previous version stays in use.
pub fn reload_balloon(p: &FileProblem) -> Balloon {
    let head = match p.line {
        Some(_) => format!("{} has a mistake", p.name()),
        None => format!("Murmur couldn't read {}", p.name()),
    };
    let body = fit(&[p.cause(REASON_ONE), p.effect.long().into()], " ", "Click to open it.");
    Balloon { title: title(&head), body, open: vec![p.path.clone()] }
}

/// A custom `model_dir` that doesn't exist. A click opens config.toml, where it's set.
pub fn model_missing_balloon(model_dir: &Path, config: &Path) -> Balloon {
    let rest = " (model_dir in config.toml). Click to open config.toml.";
    let room = BODY_MAX - units("Nothing at ") - units(rest);
    let body = format!("Nothing at {}{rest}", cut_front(&model_dir.display().to_string(), room));
    Balloon { title: "Speech model not found".into(), body, open: vec![config.to_path_buf()] }
}

/// The mic wouldn't open at startup, after a pause, or after the device went away.
pub fn mic_balloon(key: &str) -> Balloon {
    let body = format!("Check one is connected and allowed in Windows privacy settings. Murmur tries again when you press {key}.");
    Balloon { title: "Can't open the microphone".into(), body: cut(&body, BODY_MAX), open: vec![] }
}

/// A newer speech model can be downloaded from the tray.
pub fn upgrade_balloon(mb: u64) -> Balloon {
    Balloon { title: format!("A new speech model is available ({mb} MB)"), body: "Download from the tray menu.".into(), open: vec![] }
}

/// Startup's balloons in the order to show them. Each replaces the last, so the one that
/// matters most goes last: an offer, then files read wrong, then no mic, then no model.
pub fn startup(problems: &[FileProblem], mic: Option<Balloon>, model_missing: Option<Balloon>, upgrade: Option<Balloon>) -> Vec<Balloon> {
    let mut out: Vec<Balloon> = upgrade.into_iter().collect();
    if !problems.is_empty() {
        out.push(files_balloon(problems));
    }
    out.extend(mic);
    out.extend(model_missing);
    out
}

/// Fix-last's answer when dictionary.toml can't be read, so nothing could be learned.
pub fn not_learned(p: &FileProblem) -> String {
    match p.line {
        Some(n) => format!("Not learned: {} line {n} has a mistake", p.name()),
        None => format!("Not learned: {} can't be read", p.name()),
    }
}

/// One line, ending in a full stop.
fn sentence(s: &str) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.ends_with(['.', '!', '?']) { s } else { format!("{s}.") }
}

fn units(s: &str) -> usize {
    s.encode_utf16().count()
}

/// `s` cut to `max` UTF-16 units, ending in an ellipsis when cut.
fn cut(s: &str, max: usize) -> String {
    if units(s) <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut n = 0;
    for c in s.chars() {
        if n + c.len_utf16() > max - 1 {
            break;
        }
        n += c.len_utf16();
        out.push(c);
    }
    out.push('…');
    out
}

/// `s` cut to `max` UTF-16 units from the front, keeping its end (a path's folder name).
fn cut_front(s: &str, max: usize) -> String {
    if units(s) <= max {
        return s.to_string();
    }
    let mut kept: Vec<char> = Vec::new();
    let mut n = 0;
    for c in s.chars().rev() {
        if n + c.len_utf16() > max - 1 {
            break;
        }
        n += c.len_utf16();
        kept.push(c);
    }
    std::iter::once('…').chain(kept.into_iter().rev()).collect()
}

fn title(s: &str) -> String {
    cut(s, TITLE_MAX)
}

/// `parts` joined by `sep`, then `click` if it still fits; cut to the limit if even the parts don't.
fn fit(parts: &[String], sep: &str, click: &str) -> String {
    let body = parts.join(sep);
    let with = format!("{body}{sep}{click}");
    if units(&with) <= BODY_MAX { with } else { cut(&body, BODY_MAX) }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `$MCARGO test --release --locked --lib notice`
Expected: 11 passed. If `every_balloon_fits_windows_limits` fails on the many-file click line, lower `REASON_MANY` until it passes and note the value in the commit.

- [ ] **Step 5: Commit**

```bash
git add src/notice.rs src/lib.rs
git commit -m "feat(notice): error notices say what happened, what Murmur did, what to do" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: File reload problems go to a balloon you can click; dictation errors go to the pill

**Files:**
- Modify: `src/tray.rs` (balloon click state, `show`, `on_click`, Notepad)
- Modify: `src/dictionary.rs:292-325` (`DictionaryFile::refresh`)
- Modify: `src/snippets.rs:151-175` (`SnippetFile::refresh`)
- Modify: `src/pipeline.rs:1-27, 101-105, 146-151, 160-166, 203`
- Modify: `src/main.rs:3` (import `notice`), `src/main.rs:601-613` (`PipelineMsg::Error` handler, new `FileProblem` arm)
- Test: in-file `#[cfg(test)]` modules of `src/tray.rs`, `src/dictionary.rs`, `src/snippets.rs`

**Interfaces:**
- Consumes (Task 1): `notice::{Balloon, FileProblem, Effect, file_problem, reload_balloon, MODEL_LOAD, RESTARTED}`.
- Produces:
  - `Tray::show(&self, b: &Balloon)` — shows `b`; a click opens `b.open` in Notepad (nothing if empty).
  - `Tray::notify` / `Tray::notify_update` keep their signatures.
  - `DictionaryFile::refresh(&mut self, shared: &Mutex<Dictionary>) -> Option<FileProblem>` (effect `KeepingTerms`)
  - `SnippetFile::refresh(&mut self) -> Option<FileProblem>` (effect `KeepingSnippets`)
  - `PipelineMsg::FileProblem(FileProblem)`; `PipelineMsg::Error(String)` now only ever carries `MODEL_LOAD` or `RESTARTED`.

- [ ] **Step 1: Write the failing tests**

In `src/tray.rs`'s test module, add:

```rust
    #[test]
    fn a_click_does_what_the_last_balloon_offers() {
        let files = vec![PathBuf::from("C:\\m\\config.toml"), PathBuf::from("C:\\m\\dictionary.toml")];
        assert_eq!(on_click(&BalloonClick::Nothing), (None, vec![]));
        assert_eq!(on_click(&BalloonClick::Update), (Some(TrayEvent::BalloonUpdate), vec![]));
        assert_eq!(on_click(&BalloonClick::Open(files.clone())), (None, files));
        assert_eq!(click_for(&Balloon { title: "t".into(), body: "b".into(), open: vec![] }), BalloonClick::Nothing);
        assert_eq!(
            click_for(&Balloon { title: "t".into(), body: "b".into(), open: vec![PathBuf::from("C:\\m\\config.toml")] }),
            BalloonClick::Open(vec![PathBuf::from("C:\\m\\config.toml")])
        );
    }
```

In `src/dictionary.rs`'s test module, add (it uses the module's existing `temp_path` helper):

```rust
    #[test]
    fn a_broken_reload_names_the_line_and_keeps_the_terms() {
        let p = temp_path("reload-line");
        dict().save_to(&p).unwrap();
        let shared = std::sync::Mutex::new(dict());
        let mut f = DictionaryFile::new(p.clone());
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&p, "# a\n# b\n# c\nbroken\n").unwrap();
        let got = f.refresh(&shared).unwrap();
        assert_eq!(got.line, Some(4));
        assert_eq!(got.effect, crate::notice::Effect::KeepingTerms);
        assert_eq!(got.path, p);
        // deleted while running: nothing to report
        std::fs::remove_file(&p).unwrap();
        assert_eq!(f.refresh(&shared), None);
    }
```

In `src/snippets.rs`'s test module, add:

```rust
    #[test]
    fn a_broken_reload_names_the_line_and_keeps_the_snippets() {
        let p = std::env::temp_dir().join(format!("murmur-snippets-{}-reload-line.toml", std::process::id()));
        std::fs::write(&p, "# a\n# b\n# c\nbroken\n").unwrap();
        let mut f = SnippetFile::new(p.clone());
        let got = f.refresh().unwrap();
        assert_eq!(got.line, Some(4));
        assert_eq!(got.effect, crate::notice::Effect::KeepingSnippets);
        assert_eq!(f.refresh(), None);
        let _ = std::fs::remove_file(&p);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `$MCARGO test --release --locked --lib a_broken_reload`
Expected: compile errors — `no field line on type String` (refresh still returns `Option<String>`).

- [ ] **Step 3: Make `refresh` return a `FileProblem`**

`src/dictionary.rs`, in `DictionaryFile::refresh`: change the doc line to `/// Reload if the file changed since the last check. Returns what was wrong, once per bad edit.`, the signature to `-> Option<crate::notice::FileProblem>`, and the error arm to:

```rust
            Err(e) => {
                log::error!("dictionary.toml: {e:#}");
                Some(crate::notice::file_problem(&self.path, &e, crate::notice::Effect::KeepingTerms))
            }
```

`src/snippets.rs`, in `SnippetFile::refresh`: same doc change, signature `-> Option<crate::notice::FileProblem>`, error arm:

```rust
            Err(e) => {
                log::error!("snippets.toml: {e:#}");
                Some(crate::notice::file_problem(&self.path, &e, crate::notice::Effect::KeepingSnippets))
            }
```

Run: `$MCARGO test --release --locked --lib`
Expected: lib passes (bin won't compile yet; that's Step 4).

- [ ] **Step 4: Pipeline messages**

`src/main.rs:3`: add `notice` to the import list, keeping it alphabetical:

```rust
use murmur_lib::{audio, cleanup, config, context, dictionary, history, model_fetch, notice, snippets, stt, update, vad};
```

`src/pipeline.rs`: add `use crate::notice::{self, FileProblem};` after `use crate::inject;`, and change the enum:

```rust
pub enum PipelineMsg {
    Processing,
    Done(Entry),
    /// Said above the pill: the dictation couldn't go ahead.
    Error(String),
    /// A settings file changed and is broken; the previous version stays in use.
    FileProblem(FileProblem),
}
```

In `start()`:

```rust
        if let Err(e) = self.ensure_loaded() {
            log::error!("load model: {e:#}");
            let _ = self.tx.send(PipelineMsg::Error(notice::MODEL_LOAD.into()));
            return;
        }
```

In `stop()`, the two refresh calls:

```rust
        if let Some(p) = self.snippets.refresh() {
            let _ = self.tx.send(PipelineMsg::FileProblem(p));
        }
        if let Some(p) = self.dict_file.refresh(&self.dict) {
            let _ = self.tx.send(PipelineMsg::FileProblem(p));
        }
```

and the paste:

```rust
        // paste falls back to typing the text in, so an error here is only logged
        let inject = inject::paste(&cleaned).map_err(|e| log::error!("paste: {e:#}")).ok();
```

In `spawn()`'s panic arm: `let _ = st.tx.send(PipelineMsg::Error(notice::RESTARTED.into()));`

- [ ] **Step 5: The tray shows a `Balloon` and remembers its click**

`src/tray.rs`:
- Imports: change `use std::cell::Cell;` to `use std::cell::{Cell, RefCell};`, add `use std::path::{Path, PathBuf};` and `use crate::notice::Balloon;`.
- Delete the `UPDATE_BALLOON` static and its doc comment.
- Add a field `click: RefCell<BalloonClick>` to `Tray`, initialised `click: RefCell::new(BalloonClick::Nothing)` in the constructor's `Ok(Tray { … })`.
- Add, after `relabel`:

```rust
/// What a click on the balloon on screen does. It belongs to the last balloon shown.
#[derive(Debug, Clone, PartialEq)]
enum BalloonClick {
    Nothing,
    Update,
    Open(Vec<PathBuf>),
}

fn click_for(b: &Balloon) -> BalloonClick {
    if b.open.is_empty() { BalloonClick::Nothing } else { BalloonClick::Open(b.open.clone()) }
}

/// The event a click raises, and the files it opens.
fn on_click(c: &BalloonClick) -> (Option<TrayEvent>, Vec<PathBuf>) {
    match c {
        BalloonClick::Nothing => (None, vec![]),
        BalloonClick::Update => (Some(TrayEvent::BalloonUpdate), vec![]),
        BalloonClick::Open(paths) => (None, paths.clone()),
    }
}

/// Notepad: always there, and it shows the line number a notice names.
fn open_in_notepad(p: &Path) {
    if let Err(e) = std::process::Command::new("notepad.exe").arg(p).spawn() {
        log::error!("open {}: {e}", p.display());
    }
}
```

- Replace the three balloon methods:

```rust
    pub fn notify(&self, title: &str, body: &str) {
        self.show_balloon(title, body, BalloonClick::Nothing);
    }

    /// The "update available" balloon: a click raises `BalloonUpdate`.
    pub fn notify_update(&self, title: &str, body: &str) {
        self.show_balloon(title, body, BalloonClick::Update);
    }

    /// A notice; a click opens its files.
    pub fn show(&self, b: &Balloon) {
        self.show_balloon(&b.title, &b.body, click_for(b));
    }

    fn show_balloon(&self, title: &str, body: &str, click: BalloonClick) {
        // a click belongs to the balloon on screen, which is the last one shown
        *self.click.borrow_mut() = click;
        let ok = balloon(&self._icon, title, body);
        log::info!("notify: {title}: {body} (balloon accepted: {ok})");
        if !ok {
            // Fallback: Shell_NotifyIconW failed, use the tooltip as a lightweight notice.
            let _ = self._icon.set_tooltip(Some(format!("Murmur — {title}: {body}")));
        }
    }
```

- In `poll()`, replace the first `if`:

```rust
        if BALLOON_CLICKED.swap(false, Ordering::Relaxed) {
            let (ev, open) = on_click(&self.click.borrow());
            for p in &open {
                open_in_notepad(p);
            }
            if ev.is_some() {
                return ev;
            }
        }
```

- In the test module add `use crate::notice::Balloon;` and `use std::path::PathBuf;` if `super::*` doesn't bring them in.

- [ ] **Step 6: main handles the two messages**

`src/main.rs`, the `PipelineMsg::Error(s)` arm becomes (the mote is no longer faded first: `say` starts a fresh flight from wherever it is):

```rust
                PipelineMsg::Error(s) => {
                    overlay.set(resting(paused));
                    awaiting = None;
                    expect_words = false;
                    unheard_pending = false;
                    overlay.set_quiet(false);
                    forwarding = false;
                    listening = false;
                    locked = false;
                    while audio_rx.try_recv().is_ok() {}
                    said_in = say(&mut mote, &overlay, Message::plain(&s), overlay.centre_physical(), None);
                }
                // the dictation goes on with the previous version, so its landing is left alone
                PipelineMsg::FileProblem(p) => tray.show(&notice::reload_balloon(&p)),
```

- [ ] **Step 7: Run the full suite**

Run: `$MCARGO test --release --locked`
Expected: all pass, no warnings. New: `a_click_does_what_the_last_balloon_offers`, `a_broken_reload_names_the_line_and_keeps_the_terms`, `a_broken_reload_names_the_line_and_keeps_the_snippets`. The existing `dictionary_file_reloads_on_change_and_keeps_last_good` still passes unchanged.

- [ ] **Step 8: Commit**

```bash
git add src/tray.rs src/dictionary.rs src/snippets.rs src/pipeline.rs src/main.rs
git commit -m "feat(notice): a broken file found while dictating is a balloon that opens it; dictation errors go to the pill" -m "The reload error no longer goes through PipelineMsg::Error, which cleared the landing of the dictation it arrived with." -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Startup notices and the mic

**Files:**
- Modify: `src/main.rs:173-196` (collect `FileProblem`s), `:270-273` (startup mic), `:292-306` (show startup balloons), `:324-331` (deferred mic), `:342-345` (mic lost), `:377-381` (resume), `:474-480` (key-down mic), `:782-796` (`open_mic`)

**Interfaces:**
- Consumes (Task 1): `notice::{file_problem, Effect, startup, mic_balloon, model_missing_balloon, upgrade_balloon, MIC}`; (Task 2): `Tray::show`.
- Produces: `fn open_mic(audio_tx, lost_tx) -> Option<audio::Capture>` (logs, reports nothing); `fn open_mic_or_balloon(audio_tx, lost_tx, tray: &Tray, key: &str) -> Option<audio::Capture>`.

This task is wiring only; its logic (`startup` order, every balloon's copy) is unit-tested in Task 1. Verification is the build, the suite and the smoke rows in Task 5.

- [ ] **Step 1: Collect file problems at startup**

Replace `let mut startup_errors: Vec<String> = Vec::new();` and the three pushes:

```rust
    let mut problems: Vec<notice::FileProblem> = Vec::new();
    let mut cfg = match Config::load_or_create() {
        Ok(c) => c,
        Err(e) => {
            log::error!("{e:#}");
            problems.push(notice::file_problem(&config::config_dir().join("config.toml"), &e, notice::Effect::Defaults));
            Config::default()
        }
    };
```

dictionary arm: `problems.push(notice::file_problem(&dictionary::path(), &e, notice::Effect::CorrectionsOff));`
snippets: `problems.push(notice::file_problem(&snippets::path(), &e, notice::Effect::SnippetsOff));`

- [ ] **Step 2: `open_mic` stops reporting; a balloon helper reports for it**

Replace `open_mic` at the bottom of `main.rs`:

```rust
fn open_mic(audio_tx: &crossbeam_channel::Sender<Vec<f32>>, lost_tx: &crossbeam_channel::Sender<()>) -> Option<audio::Capture> {
    match audio::Capture::start(audio_tx.clone(), lost_tx.clone()) {
        Ok(c) => Some(c),
        Err(e) => {
            log::error!("open mic: {e:#}");
            None
        }
    }
}

/// Opens the mic where nobody just pressed the key (a resume, a lost device, the deferred
/// first-run open), so a failure goes to the corner.
fn open_mic_or_balloon(
    audio_tx: &crossbeam_channel::Sender<Vec<f32>>,
    lost_tx: &crossbeam_channel::Sender<()>,
    tray: &Tray,
    key: &str,
) -> Option<audio::Capture> {
    let c = open_mic(audio_tx, lost_tx);
    if c.is_none() {
        tray.show(&notice::mic_balloon(key));
    }
    c
}
```

- [ ] **Step 3: Call sites**

Startup (`:272`), so its balloon joins the ordered startup set instead of being replaced:

```rust
    let mut capture = if mic_start == MicStart::Now { open_mic(&audio_tx, &lost_tx) } else { None };
    let mic_failed = mic_start == MicStart::Now && capture.is_none();
```

Deferred first-run open (`:328`), mic lost (`:344`) and resume (`:379`):

```rust
                capture = open_mic_or_balloon(&audio_tx, &lost_tx, &tray, &cfg.ptt_key_label());
```

Key-down (`:474-480`), replacing the two `if capture.is_none()` blocks:

```rust
                    if capture.is_none() {
                        capture = open_mic(&audio_tx, &lost_tx);
                        if capture.is_none() {
                            said_in = say(&mut mote, &overlay, Message::plain(notice::MIC), overlay.centre_physical(), None);
                            continue;
                        }
                    }
```

- [ ] **Step 4: Show the startup balloons in order**

Replace the `if model_missing { … }`, `for msg in &startup_errors { … }` and `if model_state == UpgradeAvailable { … }` blocks with:

```rust
    // only reachable with a custom model_dir: the default one is set up above
    let model_gone = model_missing.then(|| {
        log::error!("model missing at {} (LOCALAPPDATA={:?})", model_dir.display(), std::env::var("LOCALAPPDATA"));
        notice::model_missing_balloon(&model_dir, &config::config_dir().join("config.toml"))
    });
    let upgrade = (model_state == config::ModelState::UpgradeAvailable).then(|| {
        tray.set_model(Some(&model_offer_label()), true);
        notice::upgrade_balloon(model_fetch::parakeet().size / 1_000_000)
    });
    let mic = mic_failed.then(|| notice::mic_balloon(&cfg.ptt_key_label()));
    for b in notice::startup(&problems, mic, model_gone, upgrade) {
        tray.show(&b);
    }
```

- [ ] **Step 5: Build and run the suite**

Run: `$MCARGO test --release --locked`
Expected: all pass, no warnings (in particular no unused `startup_errors`, no unused `tray` parameter).

- [ ] **Step 6: Commit**

```bash
git add src/main.rs
git commit -m "feat(notice): startup errors in one balloon, shown last; a mic that fails on key-down says so at the pill" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Fix-last says why nothing was learned

**Files:**
- Modify: `src/correction.rs:1-10` (imports), `:35-46` (`FixOutcome`), `:62-90` (`fix_message`), `:92-128` (`fix_last`)
- Modify: `src/main.rs:384` and the hotkey `FixLast` arm (`:546`): drop the `&tray` argument
- Test: `src/correction.rs` test module

**Interfaces:**
- Consumes (Task 1): `notice::{FileProblem, Effect, file_problem, not_learned}`.
- Produces: `FixOutcome { …, pub not_learned: Option<FileProblem> }`; `fix_last(history: &mut History, dict: &Arc<Mutex<Dictionary>>) -> FixOutcome` (no tray).

- [ ] **Step 1: Write the failing test**

In `src/correction.rs`'s test module:

```rust
    fn unreadable(line: Option<usize>) -> Option<crate::notice::FileProblem> {
        Some(crate::notice::FileProblem {
            path: "C:\\m\\dictionary.toml".into(),
            line,
            reason: "expected `=`.".into(),
            effect: crate::notice::Effect::CorrectionsOff,
        })
    }

    #[test]
    fn not_learned_comes_first_then_what_happened_to_the_text() {
        let m = fix_message(&FixOutcome { not_learned: unreadable(Some(12)), replaced: true, ..Default::default() }).unwrap();
        assert_eq!(text(&m), "Not learned: dictionary.toml line 12 has a mistake · Replaced");
        let m = fix_message(&FixOutcome { not_learned: unreadable(Some(12)), copied: true, ..Default::default() }).unwrap();
        assert_eq!(text(&m), "Not learned: dictionary.toml line 12 has a mistake · Copied, press Ctrl+V");
        let m = fix_message(&FixOutcome { not_learned: unreadable(None), ..Default::default() }).unwrap();
        assert_eq!(text(&m), "Not learned: dictionary.toml can't be read");
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `$MCARGO test --release --locked --bin murmur not_learned_comes_first`
Expected: compile error — `struct FixOutcome has no field named not_learned`.

- [ ] **Step 3: Implement**

`FixOutcome`, after `replaced`:

```rust
    /// dictionary.toml couldn't be read, so nothing was learned
    pub not_learned: Option<crate::notice::FileProblem>,
```

`fix_message`, right after `let (text, muted) = (rgb(TEXT), rgb(MUTED));` and `let mut spans = Vec::new();`:

```rust
    if let Some(p) = &o.not_learned {
        spans.push((crate::notice::not_learned(p), text));
    }
```

and the final `match` becomes:

```rust
    match (spans.is_empty(), o.copied) {
        (true, false) if o.replaced => spans.push(("Replaced".into(), text)),
        (true, false) => return None,
        (true, true) => spans.push(("Copied, press Ctrl+V to paste".into(), text)),
        (false, true) => spans.push((" · Copied, press Ctrl+V".into(), text)),
        // "Learned …" already says the edit landed; "Not learned" doesn't
        (false, false) if o.replaced && o.learned.is_empty() => spans.push((" · Replaced".into(), text)),
        (false, false) => {}
    }
```

`fix_last`: change the signature to `pub fn fix_last(history: &mut History, dict: &Arc<Mutex<Dictionary>>) -> FixOutcome`, remove `use crate::tray::Tray;`, and replace the `Err(e)` arm:

```rust
            Err(e) => {
                log::error!("reload dictionary: {e:#}");
                out.not_learned = Some(crate::notice::file_problem(&crate::dictionary::path(), &e, crate::notice::Effect::CorrectionsOff));
            }
```

`src/main.rs`: both `correction::fix_last(&mut history, &dict, &tray)` calls become `correction::fix_last(&mut history, &dict)`.

- [ ] **Step 4: Run the suite**

Run: `$MCARGO test --release --locked`
Expected: all pass, no warnings; the existing `fix_message` tests are unchanged and still pass.

- [ ] **Step 5: Commit**

```bash
git add src/correction.rs src/main.rs
git commit -m "feat(fix-last): say why nothing was learned when dictionary.toml can't be read" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Verify the branch and hand Jeff the smoke build

**Files:** none changed unless a check fails.

- [ ] **Step 1: No stragglers**

Run: `git grep -n "tray.notify(\"Startup\"\|startup_errors\|UPDATE_BALLOON\|paste failed\|pipeline error, restarted\|not loaded — fix dictionary"`
Expected: no output.

Run: `git grep -n "tray.notify(" src`
Expected: only the kept balloons — `open_editor`, PTT key save, autostart, update available/failed/started, model download ready/failed, "Close the Dictionary & Snippets window to update". Any other hit is a missed site: fix it under the task that owns it.

- [ ] **Step 2: Full suite and a release build**

Run: `$MCARGO test --release --locked` → all pass, no warnings; counts = baseline + the new tests (lib +13, bin +2).
Run: `$MCARGO build --release --locked` → `target\release\murmur.exe` built.

- [ ] **Step 3: Hand Jeff the smoke checklist**

Jeff runs the release build himself (a Claude session must not launch it; see the build-traps memory). Back up `%APPDATA%\Murmur\config.toml` and `dictionary.toml` first. Rows, from the spec:

1. Break `config.toml` (delete an `=`), start Murmur: one balloon "Murmur couldn't read config.toml" with the line; click it, Notepad opens the file.
2. Break `config.toml` and `dictionary.toml`, start: one balloon naming both; click opens both.
3. With Murmur running, break `dictionary.toml`, then dictate: the words land at the caret **with the mote**, and the "dictionary.toml has a mistake" balloon appears.
4. Fix-last with `dictionary.toml` broken: the tag says "Not learned: dictionary.toml line N has a mistake · Replaced" (or "· Copied, press Ctrl+V").
5. Set `model_dir` to a missing folder: "Speech model not found"; click opens `config.toml`.
6. With `mic_always_on = false`, disable the mic in Windows privacy settings, press the talk key: "Can't open the microphone" above the pill.

Restore the backed-up files afterwards.
