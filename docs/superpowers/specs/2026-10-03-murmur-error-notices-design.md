# Murmur error notices: say what happened, what Murmur did, what to do

Date: 2026-10-03. Status: design approved in chat, pending review of this written spec.

## Goal

Errors are raw corner balloons (UX audit item 7, 2026-10-02):

1. **The cause is lost.** Startup errors are formatted with `{e}`, which prints only anyhow's outer context: a broken `config.toml` shows "Startup" / "config: parse config.toml", without toml's line or reason.
2. **Nothing says what Murmur did or what to do.** A broken config silently runs on defaults; a broken dictionary silently turns corrections off.
3. **Errors are lost.** One balloon per startup error, and each replaces the last, so only the final one is seen.
4. **Dictation errors go to the corner.** The model failing to load, the mic failing on key-down and a pipeline restart are answers to something you just did, but appear as "Murmur" / raw text in a balloon. The item-2 spec (`2026-10-02-murmur-answers-where-you-acted-design.md`) deferred these to this item.
5. **A file reload error breaks the landing.** `pipeline.rs` sends `PipelineMsg::Error` for a broken `dictionary.toml`/`snippets.toml` just before the `Done` of the same dictation. The `Error` handler (`main.rs:601`) clears `expect_words` and `awaiting` and fades the mote, so the words paste but the mote never lands at the caret. Found by reading the code; not reproduced.

## Decisions (Jeff, 2026-10-03)

| Question | Decision |
|---|---|
| Where errors go | **Split.** Errors from something you just did go to the pill; startup and file errors stay as one rewritten balloon, because at login you aren't watching the pill. |
| A file you hand-edited is broken, found when a dictation finishes | **Balloon**, same copy as the startup file errors. The pill stays free for where your words landed. |
| How long a pill error stays | **Timed**, like "Didn't catch that" (`hold_for`). The next key press retries anyway. |
| What a balloon click does | **Opens each named file in `notepad.exe`.** Notepad is always present and shows the line number; `.toml` often has no default app. |
| Fix-last's "Dictionary not loaded" balloon | **Included.** It is an answer to fix-last, so it moves into the fix-last tag (item-2 rule). |

Out of scope: background events (update available/failed, model download, mic lost) stay balloons as today. Errors from tray actions (opening the editor, saving the talk key, autostart) stay balloons. The editor's size and position (8), raw durations (9), the tray's "Fix last" copy (10).

## Behaviour

Every notice says what happened, what Murmur did about it, and what you can do.

| Trigger | Where | Title / text |
|---|---|---|
| Startup: `config.toml` unreadable | Balloon (one for all files) | **Murmur couldn't read config.toml** / "Line 4: expected `=`. Using default settings for now. Click to open it." |
| Startup: `dictionary.toml` unreadable | Same balloon | **Murmur couldn't read dictionary.toml** / "Line 12: …. Corrections are off until it's fixed. Click to open it." |
| Startup: `snippets.toml` can't be created | Same balloon | **Murmur couldn't read snippets.toml** / the OS reason, then "Snippets are off. Click to open it." |
| Two or more of the above | One balloon | **Murmur couldn't read 2 files** / one line per file (`config.toml line 4: expected "=". Using defaults.`), then "Click to open them." |
| A file edited while Murmur runs is broken (found at `refresh` after a dictation) | Balloon | **dictionary.toml has a mistake** (or snippets.toml) / "Line 4: …. Still using your previous terms (snippets). Click to open it." The words land at the caret as normal. |
| Startup: custom `model_dir` missing | Balloon | **Speech model not found** / "Nothing at `<path>` (model_dir in config.toml). Click to open config.toml." |
| Startup: mic won't open (`mic_always_on`, including the first-run deferred open) | Balloon | **Can't open the microphone** / "Check one is connected and allowed in Windows privacy settings. Murmur tries again when you press Right Ctrl." The key is `cfg.ptt_key_label()`. |
| Key-down: mic won't open (mic opened per dictation) | Pill, timed | "Can't open the microphone" |
| Dictation: speech model fails to load | Pill, timed | "Couldn't load the speech model. Try again." |
| Pipeline panicked and restarted | Pill, timed | "Something went wrong. Try again." |
| Fix-last: `dictionary.toml` unreadable | Fix-last tag | "Not learned: dictionary.toml line 12 has a mistake", followed by the usual " · Replaced" / " · Copied, press Ctrl+V". |

Rules:

- **The cause** for a toml error is `toml::de::Error::message()`; the line is the 1-based line of `span().start` in the file's text. Any other error uses its root cause (`e.root_cause()`), e.g. "Access is denied. (os error 5)". With no span, the "Line N:" prefix is left out.
- **The full chain** (`{e:#}`) still goes to the log, as today.
- **Balloon text** is capped at 255 UTF-16 units (Windows cuts it there). The many-file body trims each file's reason before dropping the "Click to open them." line.
- **A balloon click** opens the files named by the balloon on screen, which is the last one shown. A later balloon of another kind (an update) replaces the click action, as the update balloon's does today.
- **Pill errors** are said above the pill with `say(…, None)`, as "Didn't catch that" is; a pill error during a dictation also clears that dictation's pending landing.

## Code

| File | Change |
|---|---|
| `src/notice.rs` (new) | Pure, no Win32. `FileProblem { path: PathBuf, line: Option<usize>, reason: String, effect: Effect }`; `Effect { Defaults, CorrectionsOff, SnippetsOff, KeepingPrevious }`. `file_problem(path, &anyhow::Error, effect) -> FileProblem`: finds a `toml::de::Error` in the chain (`downcast_ref` per cause), takes `message()`, and turns `span()` into a line by reading the file (`line_of(text, offset)`, pure). `files_balloon(&[FileProblem]) -> (String, String)`, `reload_balloon(&FileProblem)`, `model_missing_balloon(&Path)`, `mic_balloon(&str)`, all within the 255 cap. Pill constants `MIC`, `MODEL_LOAD`, `RESTARTED`. `not_learned(&FileProblem) -> String`. |
| `src/tray.rs` | `UPDATE_BALLOON: AtomicBool` becomes a `Cell<BalloonClick>` on `Tray`, with `BalloonClick { Nothing, Update, Open(Vec<PathBuf>) }`. New `notify_open(title, body, paths)`. `poll()`: a click on `Update` returns `TrayEvent::BalloonUpdate` as today; on `Open` it spawns `notepad.exe <path>` per path and returns `None`. The choice is a pure `on_click(&BalloonClick) -> Click` for testing. |
| `src/pipeline.rs` | New `PipelineMsg::FileProblem(FileProblem)` for `refresh` errors. `PipelineMsg::Error` carries only pill messages: `MODEL_LOAD` (from `ensure_loaded`, detail logged) and `RESTARTED`. The `paste failed` branch is removed: `inject::paste` falls back to typing and never returns `Err`. |
| `src/dictionary.rs`, `src/snippets.rs` | `DictionaryFile::refresh` / `SnippetFile::refresh` return `Option<FileProblem>` (effect `KeepingPrevious`) instead of a string. |
| `src/main.rs` | Startup collects `Vec<FileProblem>` (config → `Defaults`, dictionary → `CorrectionsOff`, snippets → `SnippetsOff`) and shows one `notify_open` balloon. Model missing uses `model_missing_balloon` with `config.toml` as the path to open. `open_mic` takes where to report: a balloon at startup, a pill message on key-down. `PipelineMsg::Error(s)` says `s` on the pill instead of `tray.notify`; `PipelineMsg::FileProblem(p)` shows `reload_balloon` and touches no overlay or mote state. |
| `src/correction.rs` | `FixOutcome` gains `not_learned: Option<FileProblem>`; the `tray.notify("Dictionary", …)` call goes. `fix_message` puts `not_learned(p)` first, then the existing replaced/copied part. |

## Testing

Test-first, unit level:

- `notice`: `line_of` for first line, a later line and CRLF; a toml error yields its message and line, not "parse config.toml"; a non-toml error yields its root cause and no line; each `Effect`'s wording; one-file vs many-file titles and bodies; every builder stays within 255 UTF-16 units with long paths and reasons.
- `DictionaryFile::refresh` / `SnippetFile::refresh` on a file broken at line 4 return a `FileProblem` with `line == Some(4)`.
- `tray::on_click`: `Update` → the update event; `Open(paths)` → open those paths; `Nothing` → nothing.
- `fix_message` with `not_learned` alone, with `replaced`, and with `copied`.

Smoke (Jeff, release build):

1. Break `config.toml` (delete an `=`), start Murmur: one balloon with the line; click it, Notepad opens the file.
2. Break `config.toml` and `dictionary.toml`, start: one balloon naming both; click opens both.
3. With Murmur running, break `dictionary.toml`, then dictate: the words land at the caret with the mote, and the reload balloon appears.
4. Fix-last with `dictionary.toml` broken: the tag says "Not learned: … line N …".
5. Set `model_dir` to a missing folder: "Speech model not found"; click opens `config.toml`.
6. With `mic_always_on = false`, disable the mic, press the talk key: "Can't open the microphone" above the pill.
