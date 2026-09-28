# Murmur dictionary editor

Date: 2026-09-28. Status: design approved in chat, pending review of this written spec.

## Goal

Adding and editing dictionary terms no longer means hand-editing `dictionary.toml`. A "Dictionary…" window lists the terms and lets you add, edit, delete and test them; saved edits apply to the next dictation without a restart. Dictation keeps working while the window is open. Hand-editing the file stays possible.

## Decisions (Jeff, 2026-09-28)

| Question | Decision |
|---|---|
| Scope | **Dictionary only.** The earlier "no settings window" ruling stands. The editor is built as a reusable panel so a future settings window can host it as a tab. |
| Blocking | **Separate process.** The tray launches `murmur.exe --dictionary`; the main app keeps dictating. The pipeline reloads `dictionary.toml` when its modified time changes. |
| Tray | "Open dictionary" becomes **"Dictionary…"** and opens the editor. The editor has an **"Open dictionary file"** link for the raw file. |
| v1 features | Add/edit, **Delete with one-step Undo** (no confirm dialog), **Search** filter, **Test a phrase** box. No "learned by Fix-last" tag. |
| `phonetic` | A per-term checkbox **"Also match sound-alikes"**, on by default, with a tooltip. |
| Layout / save | **List on the left, editor on the right, explicit Save** (button or Ctrl+S). |
| Window | A normal window: title bar, resizable, taskbar entry, not always-on-top, doesn't close on focus loss. Murmur's dark colours. |
| Single instance | A second "Dictionary…" brings the open editor to the front. |
| Conflicts | If the file changed on disk since the editor loaded it, Save offers **Reload** (discard my edits) or **Overwrite**. |
| Validation | Errors block Save; a common-word spoken form is a warning only. |

Out of scope: a snippets editor, a settings window, "learned" tags, the version bump and release.

## Components

| Unit | File | Job |
|---|---|---|
| Entry point | `src/main.rs` | If the first argument is `--dictionary`, call `dictionary_editor::run()` and return. This runs **before** the `Local\Murmur.SingleInstance` mutex, so the editor never collides with the running app. Any other arguments are ignored, as today. |
| Editor process shell | `src/dictionary_editor.rs` (new) | Creates the mutex `Local\Murmur.DictionaryEditor`. If it already exists, finds the window titled `Murmur — Dictionary` (`FindWindowW`), restores and foregrounds it, and exits. Otherwise opens an eframe window with that title and hosts `DictionaryPanel`. Handles close-with-unsaved-changes. |
| Panel | `src/dictionary_panel.rs` (new) | `DictionaryPanel` (working copy, selection, search text, test text, undo slot, file stamp, banners) and `fn ui(&mut self, ui: &mut egui::Ui)`. Knows nothing about windows or viewports. Exposes `is_dirty()` and `save()` for the shell. |
| Editing logic | `src/dictionary_panel.rs`, egui-free functions (or `dictionary.rs` where they belong to the model) | `validate(&[Term]) -> Vec<Issue>`, `normalize(Vec<Term>) -> Vec<Term>`, `filter(&[Term], &str) -> Vec<usize>`, delete/undo on `Vec<Term>`. Unit-tested without a UI. |
| Stamped load/save | `src/dictionary.rs` | `load_stamped(path) -> Result<(Dictionary, Option<SystemTime>)>` and `save_if_unchanged(&self, path, stamp) -> Result<SaveOutcome>` where `SaveOutcome` is `Saved(new_stamp)` or `Conflict`. Reuses the existing `.bak` / `.tmp` / rename save and still refuses when not loaded cleanly. Path is a parameter so tests can use a temp directory; the existing no-argument functions keep calling `path()`. |
| Live reload | `src/dictionary.rs` + `src/pipeline.rs` | `DictionaryFile`, a stamp tracker shaped like `SnippetFile::refresh`: `refresh(&mut self, shared: &Mutex<Dictionary>) -> Option<String>`. On a new modified time it re-reads the file; a clean parse replaces the shared dictionary; a parse error keeps the last good one and returns the error once for that version. The pipeline calls it right after `snippets.refresh()` and sends an error as `PipelineMsg::Error`, as snippets do. |
| Tray | `src/tray.rs`, `src/main.rs` | Menu label "Dictionary…"; `TrayEvent::OpenDictionary` becomes `TrayEvent::EditDictionary`, which spawns `std::env::current_exe()` with `--dictionary` (detached, no console). A spawn failure logs and shows a tray notice. |
| Styling | `src/correction_ui.rs` | Reuse `BG`, `BORDER`, `GREEN`, `MUTED`, `TEXT`, `load_system_font`; make them `pub(crate)` if they aren't. |

Side effects of the live reload, both intended: hand edits take effect without a restart, and a dictionary that failed to parse at startup (`empty_unloaded`) starts working once the file is fixed.

Fix-last is unchanged: it still reloads from disk before learning and saves. Its save changes the modified time, so the pipeline re-reads the file once; that's harmless.

## Behaviour

### Layout

```
[Search…          ] [+ New term]            [Open dictionary file]
┌──────────────┐ ┌──────────────────────────────────────────┐
│ BOL          │ │ Written as: [HAWB                 ]      │
│ EAR          │ │ Heard as:   [hob] ×  [hawb] ×  [haub] × [+]│
│▸HAWB         │ │ [x] Also match sound-alikes (?)          │
│ Jira         │ │                             [Delete term]│
└──────────────┘ └──────────────────────────────────────────┘
Test (dictionary only): [send the hob to jerry]
                        → send the HAWB to Jira
<banner line: conflict / error / comments note>
                           [Undo delete]  2 unsaved changes  [Save]
```

### Open

- `load_stamped()` gives the working copy and the stamp.
- The list is shown **sorted A–Z, ignoring case**; saving keeps the file's own order, with new terms appended, so the file changes as little as possible.
- Parse error: the panel shows the error, **Open dictionary file** and **Retry**, and nothing else.
- If the file has any line whose first non-space character is `#`, the note "Saving removes comments from dictionary.toml" shows near Save. `.bak` keeps the previous version.

### Edit

- The search box filters the list by written or spoken form, case-insensitive.
- **Written as**: one text field. **Heard as**: one small text field per spoken form, each with ×, plus **+**. **Also match sound-alikes**: checkbox, tooltip: "Also catch words that sound like this term, e.g. 'haub' for HAWB. For all-caps acronyms only the single-word 'Heard as' forms are used."
- **+ New term** appends an empty term (`phonetic = true`) and selects it; the search is cleared so it's visible.
- **Delete term** removes it and stores `(index, term)` in the single undo slot, replacing whatever was there. **Undo delete** reinserts it at that index. The slot is cleared by Save and by Reload.
- Dirty count: the number of terms that differ from the loaded copy (added, removed or changed). The shell's close prompt and the Save button use `is_dirty()`.

### Validation (on every change)

Errors, shown in red next to the field; they disable Save:
- written form empty (after trimming);
- two terms with the same written form, ignoring case;
- the same spoken form (trimmed, ignoring case) on two different terms.

Warning, shown in amber; it doesn't block Save:
- a spoken form that is a common word (`phonetic::is_common`): "this will rewrite '{word}' everywhere".

### Test a phrase

When the test text or the terms change, the result is `Dictionary::from_terms(normalize(working)).apply(text)`, shown underneath. Snippets and the rest of cleanup are not applied; the label says "dictionary only". Empty text shows nothing.

### Save (button or Ctrl+S)

On save the working copy is normalised: trim written and spoken forms, drop empty spoken fields, de-duplicate spoken forms within a term, lower-case spoken forms. Then `save_if_unchanged(stamp)`:
- **Saved**: take the new stamp, reset the loaded copy to the saved one, clear the dirty count and undo slot.
- **Conflict**: banner "dictionary.toml changed outside the editor (Fix-last or a hand edit)" with **Reload** (reload from disk, discarding edits) and **Overwrite** (save anyway, take the new stamp).
- **I/O error**: red banner with the error; edits are kept.

### Close

With unsaved changes the close is cancelled and a prompt offers **Save / Discard / Cancel**. Save that fails (validation, conflict or error) keeps the window open.

### Pipeline reload

- Checked before each `cleanup::clean`, only when the modified time differs from the last one seen.
- Clean parse: the shared `Arc<Mutex<Dictionary>>` is replaced.
- Parse error: the last good dictionary stays; the error is logged and sent to the tray once for that modified time, not on every dictation.
- The file missing: keep the current dictionary (it's recreated on the next save or restart).

## Testing

Unit tests (`cargo test --release --locked`):
- `validate`: empty written form; duplicate written form ignoring case; a spoken form shared by two terms; a common-word spoken form is a warning, not an error.
- `normalize`: trim, drop empty spoken, de-duplicate, lower-case spoken.
- Delete then Undo reinserts at the original index; `filter` matches written and spoken forms, ignoring case.
- `save_if_unchanged` in a temp directory: `Saved` when the stamp matches; `Conflict` after an outside write; refuses when not loaded cleanly.
- `DictionaryFile::refresh`: reloads on change; doesn't re-read when unchanged; a broken file keeps the last good copy and reports the error once.
- Argument parsing: `--dictionary` selects the editor; no argument selects the app.

Baseline before this work: lib 69, bin 42, integration 1. All must still pass.

Manual smoke on the installed build (AppData writes go through an Explorer-launched script because the Claude desktop app's AppData is MSIX-virtualized):
1. Tray → Dictionary… opens the editor; a second click brings the same window to the front.
2. Dictate into Notepad while the editor is open: it works.
3. Add a term → Save → dictate it: applied, no restart.
4. Unsaved edits, then Fix-last, then Save: the conflict banner appears; Reload and Overwrite both work.
5. Delete → Undo; close with unsaved changes → prompt.
6. Break the file by hand → dictate: one tray notice, previous terms still work; fix the file → the terms work again.

No test, fixture or screenshot uses Jeff's local `dictionary.toml` contents.

## Docs

README: the tray table row "Open dictionary" becomes "Dictionary…" (opens the dictionary editor); the Dictionary section describes the editor, with hand-editing `dictionary.toml` kept as a power-user note. The demo GIF slot is unchanged.
