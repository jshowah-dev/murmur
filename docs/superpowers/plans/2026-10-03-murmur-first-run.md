# Murmur First Run ("the card goes home") Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the first-run setup window Murmur's card look and errors that say what to do, and replace "You're ready" with the card fading to a dot that the mote carries to the pill, where it invites the first dictation until you press the key.

**Architecture:** `model_fetch` (lib) gains `FetchError::advice()`. `mote::Flight` gains a sticky message that holds until dismissed. `setup_ui` is redrawn as a frameless card with a Ready beat and a fade-to-dot, and returns the card's centre. `main.rs` says the invitation once the mote exists and writes the `welcomed` marker on the first dictation with words.

**Tech Stack:** Rust 2021, eframe/egui 0.36 (glow), `windows` 0.62, crossbeam-channel. Windows 11 only.

**Spec:** `docs/superpowers/specs/2026-10-03-murmur-first-run-design.md`

## Global Constraints

- Motion uses kit tokens only, through `motion::scaled(motion::duration::…)` and `motion::easing::…`. No raw durations or easings. The mote's `FLIGHT` is the one ledgered break.
- Opacity and transform only: the fade changes nothing's size or layout. The window height is frozen during Ready and Closing.
- Reduced motion (`editor_kit::reduced_motion()`): no fade and no flight. The invitation appears open above the pill.
- The copy is exactly the spec's: the invitation "Hold <key> and talk", with the key in `GREEN` and the rest in `TEXT`; "Ready"; "Speech model installed."; and the error texts in the spec's Errors table, verbatim.
- `FetchError`'s `Display` is unchanged (logs and the `error_messages_match_spec` test depend on it).
- A window must keep drawing the same picture on frames after it asks to close (the close-flash lesson, `c4798a6`). Send `ViewportCommand::Close` once.
- Personal repo: commit trailers are allowed. Never push or merge without Jeff.

## Review Focus

1. **The card closes on its own while you're elsewhere.** If you click away during the Ready beat, the fade and close must still run, with no focus steal. Pinned in Task 3 (`ready_runs_without_input`).
2. **Esc or Alt+F4 during Unpacking.** Close must be refused (tar can't be stopped), and Esc must do nothing. Pinned in Task 3 (`esc_does_nothing_while_unpacking`).
3. **A sticky invitation and then "Didn't catch that".** The unheard message must time out normally, with no sticky state left over. Pinned in Task 2 (`a_message_after_a_sticky_one_times_out`).
4. **Fix-last's own close fades the invitation** (`mote.fade()` paths). `leave()` must clear sticky so a later say doesn't inherit it. Pinned in Task 2 (`fade_clears_sticky`).
5. **A missing custom `model_dir` with no marker.** There must be no invitation, since there's no model. Pinned in Task 3 (`invites_only_with_a_model_and_no_marker`).

---

### Task 0: Point the shared test script at this worktree

**Files:**
- Modify: `%TEMP%\murmur-test.cmd` (outside the repo; the shared runner subagents use)

- [ ] **Step 1: Rewrite the script.** Write this content to `C:\Users\JeffLocal\AppData\Local\Temp\murmur-test.cmd`:

```bat
@echo off
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul
cd /d C:\Users\JeffLocal\git\murmur\.claude\worktrees\first-run-invite
set CARGO_TARGET_DIR=C:\Users\JeffLocal\git\murmur\target
cargo test --release --locked %*
```

- [ ] **Step 2: Baseline.** Run: `cmd.exe //c "%TEMP%\murmur-test.cmd"` (from Git Bash: `cmd.exe //c "$TEMP\\murmur-test.cmd"`).
  - Expected: lib 170, bin 164 and stt_integration 1, all passing.
  - If lib tests crash with `ORT Version is: 1.17.1` / `STATUS_ACCESS_VIOLATION`, copy `target\release\onnxruntime*.dll` into `target\release\deps\`.
  - If you see `link: extra operand`, the MSVC registration is lost. Stop and report.

No commit (nothing in the repo changed).

---

### Task 1: `FetchError::advice()`

**Files:**
- Modify: `src/model_fetch.rs` (the `impl FetchError` block, after `impl fmt::Display for FetchError` at ~line 53; tests at ~line 361)

**Interfaces:**
- Produces: `impl FetchError { pub fn advice(&self) -> String }`, which Task 3 (`setup_ui`) and Task 4 (`main.rs` `download_model`) use.

- [ ] **Step 1: Write the failing test.** Add it to `mod tests` in `src/model_fetch.rs`, after `error_messages_match_spec`:

```rust
    #[test]
    fn advice_says_what_to_do() {
        use std::io::{Error, ErrorKind};
        assert_eq!(
            FetchError::Interrupted("os error 10054".into()).advice(),
            "The download stopped partway. Check your connection, then Retry picks up where it left off."
        );
        assert_eq!(FetchError::Http(503).advice(), "The model's host isn't answering right now (error 503). Retry in a few minutes.");
        assert_eq!(FetchError::ChecksumMismatch.advice(), "The download arrived damaged. Retry fetches it again from the start.");
        assert_eq!(FetchError::Unpack("tar exit 2".into()).advice(), "The model downloaded but wouldn't unpack. Retry unpacks it again.");
        assert_eq!(
            FetchError::Io(Error::from(ErrorKind::StorageFull)).advice(),
            "Not enough disk space. Murmur needs about 1.2 GB free on the drive with your user folder. Free some up, then Retry."
        );
        assert_eq!(
            FetchError::Io(Error::from(ErrorKind::PermissionDenied)).advice(),
            "Windows blocked Murmur from saving the model in your AppData folder. Retry, or restart and try again."
        );
        assert_eq!(
            FetchError::Io(Error::other("boom")).advice(),
            "Couldn't save the model to disk. Retry, or check the log in %APPDATA%\\Murmur."
        );
        // Windows' disk-full codes reach us as StorageFull
        assert!(FetchError::Io(Error::from_raw_os_error(112)).advice().starts_with("Not enough disk space"));
    }
```

- [ ] **Step 2: Run it to verify it fails.** Run: `cmd.exe //c "%TEMP%\murmur-test.cmd" --lib advice_says_what_to_do`. Expected: a compile error, because there's no method `advice`.

- [ ] **Step 3: Implement.** Add this after the `impl fmt::Display for FetchError { … }` block:

```rust
impl FetchError {
    /// What the setup card and the tray say: what happened and what to do. The raw detail is
    /// `Display`, for the log.
    pub fn advice(&self) -> String {
        match self {
            FetchError::Interrupted(_) => {
                "The download stopped partway. Check your connection, then Retry picks up where it left off.".into()
            }
            FetchError::Http(n) => format!("The model's host isn't answering right now (error {n}). Retry in a few minutes."),
            FetchError::ChecksumMismatch => "The download arrived damaged. Retry fetches it again from the start.".into(),
            FetchError::Unpack(_) => "The model downloaded but wouldn't unpack. Retry unpacks it again.".into(),
            FetchError::Cancelled => "Cancelled".into(),
            FetchError::Io(e) => match e.kind() {
                std::io::ErrorKind::StorageFull => {
                    "Not enough disk space. Murmur needs about 1.2 GB free on the drive with your user folder. Free some up, then Retry.".into()
                }
                std::io::ErrorKind::PermissionDenied => {
                    "Windows blocked Murmur from saving the model in your AppData folder. Retry, or restart and try again.".into()
                }
                _ => "Couldn't save the model to disk. Retry, or check the log in %APPDATA%\\Murmur.".into(),
            },
        }
    }
}
```

- [ ] **Step 4: Run the tests.** Run: `cmd.exe //c "%TEMP%\murmur-test.cmd" --lib model_fetch`. Expected: all `model_fetch` tests pass, including `error_messages_match_spec` (unchanged) and `advice_says_what_to_do`. If the raw-112 assert fails, std on this toolchain doesn't map 112. Replace that line with an explicit `ErrorKind::StorageFull` match on `e.raw_os_error() == Some(112) || Some(39)` in `advice()`, and note it in the commit.

- [ ] **Step 5: Commit.**

```bash
git add src/model_fetch.rs
git commit -m "feat(model): say what to do when the model download fails"
```

---

### Task 2: A sticky mote message

**Files:**
- Modify: `src/mote.rs`. `Flight` struct and impl at ~lines 107–210; the `Phase::Speaking` arm in `sprite` at ~line 229; `Mote` impl at ~line 424; tests at the end.

**Interfaces:**
- Produces:
  - `Flight::say_until_dismissed(&mut self, m: Message, from: Pt, to: Target, now: Instant)`.
  - `Mote::say_until_dismissed(&mut self, m: Message, from: Pt, to: Target)`. Task 4 uses it.
  - A sticky message stays in `Phase::Speaking` until `dismiss()`. `say()`, `launch()`, `fade()`, `dissolve()` and `dismiss()` all clear the sticky state.

- [ ] **Step 1: Write the failing tests.** Add them to `mod tests` in `src/mote.rs` (helpers `ms`, `msg`, `PILL`, `CARET` already exist there):

```rust
    #[test]
    fn a_sticky_message_holds_until_dismissed() {
        let t0 = Instant::now();
        let m = msg("Hold Right Ctrl and talk");
        let hold = hold_for(&m);
        let mut f = Flight::new(false);
        f.say_until_dismissed(m, PILL, Target::Pill(CARET), t0);
        let late = t0 + FLIGHT + motion::duration::ENTER + hold * 10;
        let s = f.sprite(late).unwrap();
        assert!(s.open == 1.0 && s.words == 1.0, "still open long after its read time: {s:?}");
        assert!(f.is_speaking());
        f.dismiss(late);
        assert!(f.sprite(late + motion::duration::EXIT * 2 + ms(5)).is_none(), "dismiss furls and dissolves it");
    }

    #[test]
    fn a_message_after_a_sticky_one_times_out() {
        let t0 = Instant::now();
        let mut f = Flight::new(false);
        f.say_until_dismissed(msg("Hold Right Ctrl and talk"), PILL, Target::Pill(CARET), t0);
        let t1 = t0 + FLIGHT + ms(500);
        f.sprite(t1);
        let m = msg("Didn't catch that");
        let hold = hold_for(&m);
        f.say(m, PILL, Target::Pill(CARET), t1);
        assert!(f.sprite(t1 + motion::duration::ENTER + hold + motion::duration::EXIT * 2 + ms(10)).is_none());
    }

    #[test]
    fn fade_clears_sticky() {
        let t0 = Instant::now();
        let mut f = Flight::new(false);
        f.say_until_dismissed(msg("Hold Right Ctrl and talk"), PILL, Target::Pill(CARET), t0);
        f.sprite(t0 + FLIGHT + ms(300));
        f.fade(t0 + FLIGHT + ms(300));
        let t1 = t0 + FLIGHT + ms(300) + motion::duration::EXIT + ms(5);
        assert!(f.sprite(t1).is_none());
        let m = msg("Replaced");
        let hold = hold_for(&m);
        f.say(m, PILL, Target::Pill(CARET), t1);
        assert!(f.sprite(t1 + FLIGHT + motion::duration::ENTER + hold + motion::duration::EXIT * 2 + ms(10)).is_none(), "not sticky");
    }

    #[test]
    fn reduced_motion_sticky_appears_open_and_stays() {
        let t0 = Instant::now();
        let m = msg("Hold Right Ctrl and talk");
        let hold = hold_for(&m);
        let mut f = Flight::new(true);
        f.say_until_dismissed(m, PILL, Target::Pill(CARET), t0);
        let s = f.sprite(t0).unwrap();
        assert!(s.at == CARET && s.open == 1.0, "no flight, no stretch: {s:?}");
        assert!(f.sprite(t0 + hold * 10).is_some());
        f.dismiss(t0 + hold * 10);
        assert!(f.sprite(t0 + hold * 10 + ms(1)).is_none());
    }
```

- [ ] **Step 2: Run them to verify they fail.** Run: `cmd.exe //c "%TEMP%\murmur-test.cmd" --bin murmur sticky`. Expected: a compile error, because there's no method `say_until_dismissed`.

- [ ] **Step 3: Implement in `Flight`.**
  1. Add a field to `pub(crate) struct Flight`, after `centred: bool,`:

```rust
    /// the message holds until dismissed instead of for its read time
    sticky: bool,
```

  2. In `Flight::new`: `Flight { phase: Phase::Hidden, reduced, message: None, centred: false, sticky: false }`.
  3. Add `self.sticky = false;` as the first line of `launch`, `say` and `dismiss`, and of `leave` (which covers `fade` and `dissolve`'s leave path). In `dissolve`, the mid-flight branch returns early, so add `self.sticky = false;` as its first line too.
  4. Add the method after `say`:

```rust
    /// Says `m` like `say`, but it stays open until dismissed (or replaced).
    pub(crate) fn say_until_dismissed(&mut self, m: Message, from: Pt, to: Target, now: Instant) {
        self.say(m, from, to, now);
        self.sticky = true;
    }
```

  5. In `sprite`, in the `Phase::Speaking { at, start }` arm, change `if el >= unfurl + hold {` to `if !self.sticky && el >= unfurl + hold {`.

- [ ] **Step 4: Add the `Mote` wrapper** after `pub(crate) fn say(…)` in `impl Mote`:

```rust
    pub(crate) fn say_until_dismissed(&mut self, m: Message, from: Pt, to: Target) {
        self.layout = None;
        self.flight.say_until_dismissed(m, from, to, Instant::now());
    }
```

- [ ] **Step 5: Run the tests.** Run: `cmd.exe //c "%TEMP%\murmur-test.cmd" --bin murmur mote`. Expected: all mote tests pass, including the 4 new ones and the existing `reduced_motion_appears_open_and_still_holds` (non-sticky still times out).

- [ ] **Step 6: Commit.**

```bash
git add src/mote.rs
git commit -m "feat(mote): a message that holds until dismissed"
```

---

### Task 3: The setup card

**Files:**
- Modify: `src/setup_ui.rs`. A full rewrite of everything except `plan()`, `install()` and the `plan_covers_first_run_cases` test.

**Interfaces:**
- Consumes:
  - `FetchError::advice()` (Task 1).
  - `crate::mote::Pt`, `crate::caret::physical`, `crate::correction_ui::{hwnd_of, load_system_font, BG, BORDER, MUTED, TEXT}`, `crate::editor_kit::{ease, reduced_motion}`, `crate::motion`.
- Produces (Task 4 uses):
  - `pub enum SetupOutcome { Installed { from: Option<Pt> }, Quit }`.
  - `pub fn run(models: PathBuf, plan: Plan) -> SetupOutcome` (the `key` parameter is gone).
  - `pub fn invites(welcomed: bool, model_missing: bool) -> bool`.
  - `pub fn mark_welcomed()`, now `pub`.
  - `pub fn welcome_marker() -> PathBuf` and `pub fn plan(...)`, both unchanged.

- [ ] **Step 1: Write the failing tests.** Replace `mod tests` in `src/setup_ui.rs` with:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn app(stage: Stage) -> SetupApp {
        let (_, rx) = crossbeam_channel::unbounded();
        SetupApp {
            models: PathBuf::new(),
            total: 483 * MB,
            stage,
            rx,
            cancel: Arc::new(AtomicBool::new(false)),
            installed: Arc::new(AtomicBool::new(false)),
            then_ready: true,
            hwnd: HWND::default(),
            reduced: false,
            closing: false,
            focus_retry: false,
            height: 0.0,
            from: Rc::new(Cell::new(None)),
        }
    }

    /// Runs one frame, returning the text drawn and whether the window asked to close.
    fn frame(ctx: &egui::Context, app: &mut SetupApp, events: Vec<egui::Event>) -> (Vec<String>, bool) {
        fn walk(shape: &egui::epaint::Shape, out: &mut Vec<String>) {
            match shape {
                egui::epaint::Shape::Text(t) => out.push(t.galley.text().to_string()),
                egui::epaint::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let input = egui::RawInput { events, ..Default::default() };
        let out = ctx.run_ui(input, |ui| app.draw(ui));
        let mut text = Vec::new();
        out.shapes.iter().for_each(|c| walk(&c.shape, &mut text));
        let close = out.viewport_output.values().any(|v| v.commands.contains(&ViewportCommand::Close));
        (text, close)
    }

    fn esc() -> egui::Event {
        egui::Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }
    }

    #[test]
    fn close_refused_only_while_unpacking() {
        assert!(refuse_close(&Stage::Unpacking(0), false));
        assert!(!refuse_close(&Stage::Downloading(5), false));
        assert!(!refuse_close(&Stage::Failed("x".into()), false));
        assert!(!refuse_close(&Stage::Closing(Instant::now()), true));
    }

    #[test]
    fn own_close_after_install_is_not_refused() {
        // Done arrives while the stage still reads Unpacking; the window's own Close must go through
        assert!(!refuse_close(&Stage::Unpacking(0), true));
    }

    #[test]
    fn plan_covers_first_run_cases() {
        // (model_missing, default_dir, welcomed)
        assert_eq!(plan(true, true, false), Plan::Download { then_ready: true });
        assert_eq!(plan(true, true, true), Plan::Download { then_ready: false });
        assert_eq!(plan(false, true, false), Plan::ReadyOnly);
        assert_eq!(plan(false, false, false), Plan::ReadyOnly);
        assert_eq!(plan(false, true, true), Plan::Skip);
        // a custom model_dir that's missing gets the tray notice, and no Ready claim
        assert_eq!(plan(true, false, false), Plan::Skip);
        assert_eq!(plan(true, false, true), Plan::Skip);
    }

    #[test]
    fn invites_only_with_a_model_and_no_marker() {
        assert!(invites(false, false));
        assert!(!invites(true, false), "welcomed before");
        assert!(!invites(false, true), "a missing custom model_dir: nothing to dictate with");
    }

    #[test]
    fn ready_holds_for_a_beat_then_fades_or_closes() {
        let beat = motion::scaled(motion::duration::LOCATE);
        assert_eq!(after_ready(beat / 2, false), Next::Hold);
        assert_eq!(after_ready(beat, false), Next::Fade);
        assert_eq!(after_ready(beat, true), Next::Close, "reduced motion: no fade");
    }

    #[test]
    fn the_fade_runs_down_to_nothing() {
        let exit = motion::scaled(motion::duration::EXIT);
        assert_eq!(fade_alpha(Duration::ZERO), Some(1.0));
        let mid = fade_alpha(exit / 2).unwrap();
        assert!(mid > 0.0 && mid < 1.0, "{mid}");
        assert_eq!(fade_alpha(exit), None, "only the dot is left");
    }

    #[test]
    fn a_failed_download_says_what_to_do_and_esc_quits() {
        let ctx = egui::Context::default();
        let mut a = app(Stage::Failed(FetchError::Interrupted("os error 10054".into()).advice()));
        let (text, close) = frame(&ctx, &mut a, vec![]);
        assert!(text.iter().any(|t| t.starts_with("The download stopped partway")), "{text:?}");
        assert!(text.iter().any(|t| t == "Retry") && text.iter().any(|t| t == "Quit"), "{text:?}");
        assert!(!text.iter().any(|t| t.contains("10054")), "no raw detail on the card");
        assert!(!close);
        let (_, close) = frame(&ctx, &mut a, vec![esc()]);
        assert!(close);
        assert!(!a.cancel.load(Ordering::SeqCst), "nothing to cancel after a failure");
    }

    #[test]
    fn esc_cancels_the_download() {
        let ctx = egui::Context::default();
        let mut a = app(Stage::Downloading(10 * MB));
        let (text, _) = frame(&ctx, &mut a, vec![]);
        assert!(text.iter().any(|t| t == "Downloading speech model: 10 / 483 MB"), "{text:?}");
        assert!(text.iter().any(|t| t == "Murmur needs its speech model (about 483 MB) before first use."), "{text:?}");
        let (_, close) = frame(&ctx, &mut a, vec![esc()]);
        assert!(close && a.cancel.load(Ordering::SeqCst));
    }

    #[test]
    fn esc_does_nothing_while_unpacking() {
        let ctx = egui::Context::default();
        let mut a = app(Stage::Unpacking(0));
        let (_, close) = frame(&ctx, &mut a, vec![esc()]);
        assert!(!close && !a.cancel.load(Ordering::SeqCst));
    }

    #[test]
    fn ready_runs_without_input() {
        // past the beat, with no input at all: it moves on to the fade by itself
        let ctx = egui::Context::default();
        let mut a = app(Stage::Ready(Instant::now() - motion::scaled(motion::duration::LOCATE) * 2));
        a.installed.store(true, Ordering::SeqCst);
        let (text, close) = frame(&ctx, &mut a, vec![]);
        assert!(matches!(a.stage, Stage::Closing(_)), "fading");
        assert!(!close, "the fade comes first");
        assert!(text.iter().any(|t| t == "Ready"), "same card on the frame the fade starts: {text:?}");
    }

    #[test]
    fn the_faded_card_closes_once_and_hands_back_its_centre() {
        let ctx = egui::Context::default();
        let mut a = app(Stage::Closing(Instant::now() - motion::scaled(motion::duration::EXIT) * 2));
        a.installed.store(true, Ordering::SeqCst);
        let (_, close) = frame(&ctx, &mut a, vec![]);
        assert!(close);
        assert!(a.from.get().is_some(), "the mote starts where the card was");
        let (_, close) = frame(&ctx, &mut a, vec![]);
        assert!(!close, "asks to close once");
    }

    #[test]
    fn reduced_motion_closes_after_the_beat_with_no_dot() {
        let ctx = egui::Context::default();
        let mut a = app(Stage::Ready(Instant::now() - motion::scaled(motion::duration::LOCATE) * 2));
        a.reduced = true;
        a.installed.store(true, Ordering::SeqCst);
        let (_, close) = frame(&ctx, &mut a, vec![]);
        assert!(close);
        assert!(a.from.get().is_none(), "no flight: the invitation appears above the pill");
    }
}
```

- [ ] **Step 2: Run them to verify they fail.** Run: `cmd.exe //c "%TEMP%\murmur-test.cmd" --bin murmur setup_ui`. Expected: compile errors (`after_ready`, `fade_alpha`, `invites`, `Stage::Closing`, `draw` and others don't exist yet).

- [ ] **Step 3: Implement.** Replace everything in `src/setup_ui.rs` above `mod tests` with the following, **except `fn install(…)`**. Keep that function byte for byte and put it back directly after `impl SetupApp { … }`, where it already sits. It isn't repeated in the listing. `plan()` is carried over unchanged (shown in full below).

```rust
//! First-run window: the model download as a Murmur card, then "Ready", then the card fades to
//! a dot that the mote carries to the pill (see `main.rs`, where the invitation is said).

use crate::config;
use crate::correction_ui::{hwnd_of, load_system_font, BG, BORDER, MUTED, TEXT};
use crate::editor_kit::{ease, reduced_motion};
use crate::motion;
use crate::mote::Pt;
use crossbeam_channel::{Receiver, Sender};
use eframe::egui::{self, Color32, CornerRadius, Frame, Key, Margin, Modifiers, Pos2, RichText, Stroke, ViewportCommand};
use murmur_lib::model_fetch::{self, FetchError, Fetcher};
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

const MB: u64 = 1024 * 1024;
const TICK: Duration = Duration::from_millis(250);
const WIDTH: f32 = 480.0;

pub enum SetupOutcome {
    /// `from`: where the card faded to a dot, physical pixels; None when it didn't (no card,
    /// reduced motion, or no invitation to carry).
    Installed { from: Option<Pt> },
    Quit,
}

/// What the setup window has to do on this launch.
#[derive(Debug, PartialEq)]
pub enum Plan {
    Download { then_ready: bool },
    ReadyOnly,
    Skip,
}

/// The auto-download only runs for the default model_dir; a missing custom one gets the tray
/// notice instead, and no "ready" screen that would be false.
pub fn plan(model_missing: bool, default_dir: bool, welcomed: bool) -> Plan {
    match (model_missing, default_dir, welcomed) {
        (true, true, _) => Plan::Download { then_ready: !welcomed },
        (true, false, _) | (false, _, true) => Plan::Skip,
        (false, _, false) => Plan::ReadyOnly,
    }
}

/// Whether the pill invites a first dictation: never welcomed, and there's a model to dictate with.
pub fn invites(welcomed: bool, model_missing: bool) -> bool {
    !welcomed && !model_missing
}

/// Written on the first dictation with words, so the invitation stops coming back.
pub fn welcome_marker() -> PathBuf {
    config::config_dir().join("welcomed")
}

pub fn mark_welcomed() {
    let path = welcome_marker();
    if let Err(e) = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(&path, "")) {
        log::error!("write {}: {e}", path.display());
    }
}

enum Msg {
    Progress(u64),
    Unpacking(u64),
    Done,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq)]
enum Stage {
    Downloading(u64),
    Unpacking(u64),
    /// what to do, from `FetchError::advice`
    Failed(String),
    /// installed: "Ready" for a beat, since then
    Ready(Instant),
    /// fading to a dot at the card's centre, since then
    Closing(Instant),
}

#[derive(Debug, PartialEq)]
enum Next {
    Hold,
    Fade,
    Close,
}

/// What the Ready beat does `el` after it began.
fn after_ready(el: Duration, reduced: bool) -> Next {
    if el < motion::scaled(motion::duration::LOCATE) {
        Next::Hold
    } else if reduced {
        Next::Close
    } else {
        Next::Fade
    }
}

/// The card's opacity `el` into the fade; None once only the dot is left.
fn fade_alpha(el: Duration) -> Option<f32> {
    let t = el.as_secs_f32() / motion::scaled(motion::duration::EXIT).as_secs_f32();
    (t < 1.0).then(|| 1.0 - ease(motion::easing::EXIT, t))
}

/// The mote's dot, drawn where the card was: a pale core in a soft green halo (`mote::render`).
fn dot(painter: &egui::Painter, c: Pos2, alpha: f32) {
    use egui::epaint::{Mesh, Vertex, WHITE_UV};
    // the halo falls off as 0.5 * (1 - d/r)^2, as the mote's canvas draws it: sampled on rings,
    // linear between them
    const R: f32 = 12.0;
    const SEG: usize = 32;
    const RINGS: [f32; 5] = [0.0, 0.25, 0.5, 0.75, 1.0];
    let green = |f: f32| Color32::from_rgba_unmultiplied(0x60, 0xD0, 0x60, (0.5 * alpha * (1.0 - f).powi(2) * 255.0) as u8);
    let mut mesh = Mesh::default();
    for &f in &RINGS {
        for s in 0..SEG {
            let a = s as f32 / SEG as f32 * std::f32::consts::TAU;
            mesh.vertices.push(Vertex { pos: c + egui::vec2(a.cos(), a.sin()) * R * f, uv: WHITE_UV, color: green(f) });
        }
    }
    for ring in 0..RINGS.len() - 1 {
        for s in 0..SEG {
            let (i0, i1) = ((ring * SEG + s) as u32, (ring * SEG + (s + 1) % SEG) as u32);
            let (o0, o1) = (i0 + SEG as u32, i1 + SEG as u32);
            mesh.add_triangle(i0, o0, o1);
            mesh.add_triangle(i0, o1, i1);
        }
    }
    painter.add(mesh);
    painter.circle_filled(c, 2.5, Color32::from_rgba_unmultiplied(0xE8, 0xFF, 0xE8, (alpha * 255.0) as u8));
}

struct SetupApp {
    models: PathBuf,
    total: u64,
    stage: Stage,
    rx: Receiver<Msg>,
    cancel: Arc<AtomicBool>,
    installed: Arc<AtomicBool>,
    then_ready: bool,
    hwnd: HWND,
    reduced: bool,
    /// Close was sent; later frames keep drawing the same picture until the window goes
    closing: bool,
    /// a failure just arrived: Retry takes keyboard focus, so Enter retries
    focus_retry: bool,
    height: f32,
    from: Rc<Cell<Option<Pt>>>,
}

impl SetupApp {
    /// Runs one download + unpack attempt on a worker thread; Retry calls this again and resumes.
    fn start(&mut self, ctx: &egui::Context) {
        let (tx, rx) = crossbeam_channel::unbounded();
        self.rx = rx;
        self.stage = Stage::Downloading(0);
        let models = self.models.clone();
        let cancel = self.cancel.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let msg = match install(&models, &cancel, &tx, &ctx) {
                Ok(()) => Msg::Done,
                Err(FetchError::Cancelled) => return,
                Err(e) => {
                    log::error!("model setup: {e}");
                    Msg::Failed(e.advice())
                }
            };
            let _ = tx.send(msg);
            ctx.request_repaint();
        });
    }

    fn close(&mut self, ctx: &egui::Context) {
        if !self.closing {
            self.closing = true;
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }

    /// The window's centre, physical pixels: the card fills it.
    fn centre_physical(&self) -> Pt {
        crate::caret::physical(|| unsafe {
            let mut r = RECT::default();
            let _ = GetWindowRect(self.hwnd, &mut r);
            ((r.left + r.right) as f32 / 2.0, (r.top + r.bottom) as f32 / 2.0)
        })
    }

    /// The whole window, apart from eframe itself, so tests can drive it.
    fn draw(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let now = Instant::now();
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Progress(n) => self.stage = Stage::Downloading(n),
                Msg::Unpacking(n) => self.stage = Stage::Unpacking(n),
                Msg::Failed(e) => {
                    self.stage = Stage::Failed(e);
                    self.focus_retry = true;
                }
                Msg::Done => {
                    self.installed.store(true, Ordering::SeqCst);
                    if self.then_ready {
                        self.stage = Stage::Ready(now);
                    } else {
                        self.close(&ctx);
                    }
                }
            }
        }
        if let Stage::Ready(since) = self.stage {
            let el = now.saturating_duration_since(since);
            match after_ready(el, self.reduced) {
                Next::Hold => ctx.request_repaint_after(motion::scaled(motion::duration::LOCATE).saturating_sub(el)),
                Next::Fade => self.stage = Stage::Closing(now),
                Next::Close => self.close(&ctx),
            }
        }
        // Alt+F4 counts as Cancel; the .part stays for next launch
        if ctx.input(|i| i.viewport().close_requested()) {
            if refuse_close(&self.stage, self.installed.load(Ordering::SeqCst)) {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            } else if !self.installed.load(Ordering::SeqCst) {
                self.cancel.store(true, Ordering::SeqCst);
            }
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            match self.stage {
                Stage::Downloading(_) => {
                    self.cancel.store(true, Ordering::SeqCst);
                    self.close(&ctx);
                }
                Stage::Failed(_) => self.close(&ctx),
                _ => {}
            }
        }

        let alpha = match self.stage {
            Stage::Closing(start) => fade_alpha(now.saturating_duration_since(start)),
            _ => Some(1.0),
        };
        let mut retry = false;
        let card = ui
            .scope(|ui| {
                ui.set_opacity(alpha.unwrap_or(0.0));
                Frame::new()
                    .fill(BG)
                    .stroke(Stroke::new(1.0, BORDER))
                    .corner_radius(CornerRadius::same(14))
                    .inner_margin(Margin::symmetric(16, 14))
                    .show(ui, |ui| {
                        ui.set_width(WIDTH - 34.0);
                        self.body(ui, &ctx, &mut retry);
                    })
                    .response
                    .rect
            })
            .inner;

        if let Stage::Closing(_) = self.stage {
            dot(ui.painter(), card.center(), 1.0 - alpha.unwrap_or(0.0));
            match alpha {
                Some(_) => ctx.request_repaint(),
                None => {
                    if !self.closing {
                        self.from.set(Some(self.centre_physical()));
                    }
                    self.close(&ctx);
                }
            }
        } else {
            // Grow or shrink the window to the card so no invisible margin swallows clicks.
            let h = card.max.y.ceil() + 1.0;
            if (h - self.height).abs() > 0.5 && !matches!(self.stage, Stage::Ready(_)) {
                self.height = h;
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(egui::vec2(WIDTH, h)));
            }
        }
        if retry {
            self.start(&ctx);
        }
    }

    /// The card's contents. Every stage keeps the same rows, so the card never changes height.
    fn body(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, retry: &mut bool) {
        let installed = matches!(self.stage, Stage::Ready(_) | Stage::Closing(_));
        let intro = if installed {
            "Speech model installed.".to_string()
        } else {
            format!("Murmur needs its speech model (about {} MB) before first use.", self.total / MB)
        };
        ui.label(RichText::new(intro).size(13.0).color(MUTED));
        ui.add_space(12.0);
        match self.stage.clone() {
            Stage::Downloading(n) => {
                ui.label(RichText::new(format!("Downloading speech model: {} / {} MB", n / MB, self.total / MB)).size(15.0).color(TEXT));
                ui.add_space(6.0);
                ui.add(egui::ProgressBar::new(n as f32 / self.total as f32));
                ui.add_space(12.0);
                if ui.button("Cancel").clicked() {
                    self.cancel.store(true, Ordering::SeqCst);
                    self.close(ctx);
                }
            }
            Stage::Unpacking(n) => {
                // held under 100% until tar exits: the last files land after the size estimate
                let frac = (n as f32 / model_fetch::PARAKEET_UNPACKED as f32).min(0.99);
                ui.label(RichText::new(format!("Unpacking speech model: {:.0}%", frac * 100.0)).size(15.0).color(TEXT));
                ui.add_space(6.0);
                ui.add(egui::ProgressBar::new(frac));
                ui.add_space(12.0);
                // tar can't be stopped: no Cancel, but its room stays
                ui.add_visible(false, egui::Button::new("Cancel"));
            }
            Stage::Failed(e) => {
                ui.label(RichText::new(e).size(14.0).color(TEXT));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let r = ui.button("Retry");
                    if std::mem::take(&mut self.focus_retry) {
                        r.request_focus();
                    }
                    *retry = r.clicked();
                    if ui.button("Quit").clicked() {
                        self.close(ctx);
                    }
                });
            }
            Stage::Ready(_) | Stage::Closing(_) => {
                ui.label(RichText::new("Ready").size(15.0).color(TEXT));
                ui.add_space(6.0);
                ui.add(egui::ProgressBar::new(1.0));
                ui.add_space(12.0);
                ui.add_visible(false, egui::Button::new("Cancel"));
            }
        }
    }
}

/// tar.exe can't be stopped, so a close while it runs would orphan it mid-unpack. Once
/// installed, the close is the window's own and must go through.
fn refuse_close(stage: &Stage, installed: bool) -> bool {
    matches!(stage, Stage::Unpacking(_)) && !installed
}

impl eframe::App for SetupApp {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        self.draw(ui);
    }
}

/// First-run window, per `plan`: the model download, then (first run only) "Ready" and the fade
/// to a dot. Blocks until the model is installed and the window is closed, or the user quits.
/// `ReadyOnly` and `Skip` show no window.
pub fn run(models: PathBuf, plan: Plan) -> SetupOutcome {
    let then_ready = match plan {
        Plan::Download { then_ready } => then_ready,
        Plan::ReadyOnly | Plan::Skip => return SetupOutcome::Installed { from: None },
    };
    let installed = Arc::new(AtomicBool::new(false));
    let done = installed.clone();
    let from = Rc::new(Cell::new(None));
    let app_from = from.clone();
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Murmur setup")
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_inner_size([WIDTH, 170.0]),
        centered: true,
        ..Default::default()
    };
    let r = eframe::run_native(
        "murmur-setup",
        opts,
        Box::new(move |cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            load_system_font(&cc.egui_ctx);
            let (_, rx) = crossbeam_channel::unbounded();
            let mut app = SetupApp {
                models,
                total: model_fetch::vad().size + model_fetch::parakeet().size,
                stage: Stage::Downloading(0),
                rx,
                cancel: Arc::new(AtomicBool::new(false)),
                installed: done,
                then_ready,
                hwnd: hwnd_of(cc).unwrap_or_default(),
                reduced: reduced_motion(),
                closing: false,
                focus_retry: false,
                height: 0.0,
                from: app_from,
            };
            app.start(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    );
    if let Err(e) = r {
        log::error!("setup window: {e}");
    }
    if installed.load(Ordering::SeqCst) {
        SetupOutcome::Installed { from: from.get() }
    } else {
        SetupOutcome::Quit
    }
}
```

  Notes for the implementer:
  - `install()` is the existing function, byte for byte. Do not change it.
  - `dot()` samples the mote's halo on five rings with linear interpolation between them. If `Vertex`'s fields differ in epaint 0.36, check `~/.cargo/registry/src/*/epaint-0.36*/src/mesh.rs` and match them. Don't switch to stacked circles.
  - `ui.add_visible(false, …)` keeps the row's room so the card never changes height between stages.
  - The test `app()` helper sets `total: 483 * MB`, so the copy assertions read "483 MB". The real total is `vad().size + parakeet().size`, which shows as 460.
  - If `Ui::scope`'s `.inner` doesn't give the `Rect` because the closure returns `Frame::show(..).response.rect`, check egui 0.36's `InnerResponse` at the source (`~/.cargo/registry/src/*/egui-0.36*/src/containers/frame.rs`) before changing the shape.

- [ ] **Step 4: Fix the call site so the bin compiles.** In `src/main.rs`, the `setup_ui::run(models, plan, cfg.ptt_key_label())` match (~line 209):

```rust
        match setup_ui::run(models, plan) {
            // re-checked: the unpacked folder must be the one model_dir names
            setup_ui::SetupOutcome::Installed { .. } => model_missing = !model_fetch::is_installed(&model_dir),
```

  (Task 4 uses `from`; this step only compiles.)

- [ ] **Step 5: Run the tests.** Run: `cmd.exe //c "%TEMP%\murmur-test.cmd" --bin murmur setup_ui`. Expected: all 12 `setup_ui` tests pass. Then run the full bin suite: `cmd.exe //c "%TEMP%\murmur-test.cmd" --bin murmur`. Expected: all pass (164 + 4 from Task 2 + 9 new here = 177).

- [ ] **Step 6: Commit.**

```bash
git add src/setup_ui.rs src/main.rs
git commit -m "feat(setup): the first-run card in Murmur's look, fading to a dot when ready"
```

---

### Task 4: Wire the invitation, the marker, pause, and the tray's model failure

**Files:**
- Modify: `src/main.rs`
  - `download_model`, ~line 146.
  - The setup block, ~lines 204–216.
  - After `overlay.set(OverlayState::Idle);`, ~line 277.
  - `TrayEvent::TogglePause`, ~line 343.
  - `PipelineMsg::Done`, ~line 556.
  - `UpdateMsg::ModelFailed`, ~line 619.
  - Near `const UNHEARD`, ~line 645.
  - Tests at the end.

**Interfaces:**
- Consumes:
  - `setup_ui::{run, invites, mark_welcomed, welcome_marker, SetupOutcome::Installed { from }}` (Task 3).
  - `Mote::say_until_dismissed` (Task 2).
  - `FetchError::advice()` (Task 1).
- Produces: `fn invite(key: &str) -> Message` in `main.rs`.

- [ ] **Step 1: Write the failing test.** Add it to `mod tests` at the end of `src/main.rs`:

```rust
    #[test]
    fn the_invitation_names_the_key_in_green() {
        let m = invite("Right Ctrl");
        let text: String = m.0.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!(text, "Hold Right Ctrl and talk");
        let key = m.0.iter().find(|(s, _)| s == "Right Ctrl").expect("the key is its own run");
        assert_eq!(key.1, correction_ui::rgb(correction_ui::GREEN));
        assert!(m.0.iter().filter(|(s, _)| s != "Right Ctrl").all(|(_, c)| *c == correction_ui::rgb(correction_ui::TEXT)));
    }
```

  (If `mod tests` in `main.rs` lacks `use super::*;`, it already has it. Check, and use `crate::correction_ui` if `correction_ui` isn't in scope.)

- [ ] **Step 2: Run it to verify it fails.** Run: `cmd.exe //c "%TEMP%\murmur-test.cmd" --bin murmur the_invitation`. Expected: a compile error, because there's no function `invite`.

- [ ] **Step 3: Add `invite`** after `const UNHEARD: &str = "Didn't catch that";`:

```rust
/// What the pill says on first run until you first press the key.
fn invite(key: &str) -> Message {
    use correction_ui::{rgb, GREEN, TEXT};
    Message(vec![("Hold ".into(), rgb(TEXT)), (key.to_string(), rgb(GREEN)), (" and talk".into(), rgb(TEXT))])
}
```

- [ ] **Step 4: Setup block.** Replace the block from `let plan = setup_ui::plan(…)` through the closing `}` of `if plan != setup_ui::Plan::Skip { … }` with:

```rust
    let welcomed = setup_ui::welcome_marker().exists();
    let plan = setup_ui::plan(model_missing, default_dir, welcomed);
    // where the setup card faded to a dot, for the mote that carries it to the pill
    let mut card_at = None;
    if plan != setup_ui::Plan::Skip {
        let models = model_dir.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| model_dir.clone());
        match setup_ui::run(models, plan) {
            // re-checked: the unpacked folder must be the one model_dir names
            setup_ui::SetupOutcome::Installed { from } => {
                model_missing = !model_fetch::is_installed(&model_dir);
                card_at = from;
            }
            setup_ui::SetupOutcome::Quit => {
                log::info!("model setup not finished; exiting");
                return Ok(());
            }
        }
    }
    // until the first dictation with words: quitting before trying brings it back next launch
    let mut invite_open = setup_ui::invites(welcomed, model_missing);
```

- [ ] **Step 5: Say it.** Directly after `overlay.set(OverlayState::Idle);` (before the `if model_missing {` tray notice):

```rust
    if invite_open {
        let from = card_at.unwrap_or_else(|| overlay.centre_physical());
        mote.say_until_dismissed(invite(&cfg.ptt_key_label()), from, Target::Pill(overlay.above_physical()));
    }
```

  `said_in` stays `None`, so switching windows doesn't close it.

- [ ] **Step 6: Marker on the first words.** In `PipelineMsg::Done(e)`, after the existing `if heard { history.push(e); }`:

```rust
                    if heard && std::mem::take(&mut invite_open) {
                        setup_ui::mark_welcomed();
                    }
```

- [ ] **Step 7: Pause dismisses.** In `TrayEvent::TogglePause`, inside `if paused {`, before `capture.take();`:

```rust
                        // a paused key does nothing, so nothing should invite it
                        if mote.is_speaking() {
                            mote.dismiss();
                        }
```

- [ ] **Step 8: The tray's model failure.**
  1. In `download_model`, replace `Err(e) => UpdateMsg::ModelFailed(e.to_string()),` with:

```rust
        Err(e) => {
            log::error!("model upgrade: {e}");
            UpdateMsg::ModelFailed(e.advice())
        }
```

  2. In the `UpdateMsg::ModelFailed(why)` handler, delete the line `log::error!("model upgrade: {why}");`, since the raw error is now logged at the source.

- [ ] **Step 9: Build and run the full suite.** Run: `cmd.exe //c "%TEMP%\murmur-test.cmd"`. Expected:
  - lib 171 (170 + `advice_says_what_to_do`), bin 178 (177 + `the_invitation_names_the_key_in_green`) and stt_integration 1, all passing.
  - No new warnings. `grep -n "ptt_key_label" src/setup_ui.rs` returns nothing.

- [ ] **Step 10: Commit.**

```bash
git add src/main.rs
git commit -m "feat(first-run): the pill invites your first dictation until you press the key"
```

---

### Task 5: Frames, then a release build for Jeff's smoke

**Files:** none in the repo (frames go to the scratchpad).

- [ ] **Step 1: Ask Jeff whether to record frames.** He skipped them for the last feature. Recording needs a debug build with `MOTION_TIME_SCALE=10` doing a real first-run download in this session's private `%LOCALAPPDATA%` (about 460 MB, resumable). If he says skip, record that in the handoff as UNPROVEN and go to Step 3.
- [ ] **Step 2 (if yes): Record the frames.**
  1. Build debug: `cmd.exe //c` a copy of the test script with `cargo build` in place of `cargo test --release --locked`.
  2. Launch `target\debug\murmur.exe` with `MOTION_TIME_SCALE=10`. Once unpacking reaches about 95%, run `powershell.exe -NoProfile -ExecutionPolicy Bypass -File ~/.claude/skills/considered-ux/scripts/frames-native.ps1 -Title "Murmur setup" -Out <scratchpad>/frames-normal`.
  3. Strip the Ready → Closing → dot span with `node ~/.claude/skills/considered-ux/scripts/strip.mjs --dir <dir> --out <dir>/strip.png`.
  4. Check: the card stays the same size, the opacity runs down, the dot emerges at the centre, and there's no blank frame before the window goes.
  5. Repeat with reduced motion on (Settings → Accessibility → Visual effects → Animation effects off): the card closes after the beat with no fade.
  6. Fix anything the frames show before going on.
- [ ] **Step 3: Release build for the smoke.** `cmd.exe //c` the test script with `cargo build --release --locked`. Expected: `target\release\murmur.exe` builds. Hand Jeff the spec's smoke list (Testing → Smoke, rows 1–6) and the exe path. Don't launch the installed copy from this session (build trap: the Claude app update kills it).
- [ ] **Step 4: Log the moment in the kit ledger.** Append a row to `~/.claude/skills/considered-ux/kits/jeff.md`:

```
| 2026-10-03 | murmur | First run | The card goes home: "Ready", the setup card fades to the mote's dot, which arcs to the pill and says "Hold Right Ctrl and talk" until you press the key | I want to hear and understand you | beat duration.locate; fade duration.exit + easing.exit; carry FLIGHT (break) + easing.enter; unfurl duration.enter + easing.enter; hold until dismissed |
```

  Then run `node ~/.claude/skills/considered-ux/scripts/kit.mjs --check`. Expected: no errors.
