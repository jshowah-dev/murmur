# Murmur snippets editor

Date: 2026-09-29. Status: design approved in chat, pending review of this written spec.

## Goal

Adding and editing snippets no longer means hand-editing `snippets.toml`. The dictionary editor window gains a **Snippets** tab that lists snippets and lets you add, edit, delete and test them; saved edits apply to the next dictation without a restart (the pipeline already reloads `snippets.toml` when its modified time changes). Dictation keeps working while the window is open. Hand-editing the file stays possible.

## Decisions (Jeff, 2026-09-29)

| Question | Decision |
|---|---|
| Window | **One window, two tabs**: Dictionary \| Snippets. Same process, same single-instance mutex, same close prompt, same updater guard. |
| Tray | "Open snippets" becomes **"Snippets…"**. "Dictionary…" and "Snippets…" open the window on that tab; if it's already open, either one just brings it to the front (no tab switch). |
| v1 features | Search, Delete with one-step Undo, Test a phrase, conflict detection (Reload / Overwrite), common-word warning. |
| Validation | Errors block Save: empty trigger, empty text, duplicate trigger. A one-word common trigger is a warning only. |
| List order | **A–Z by trigger**, with the dictionary's freeze-and-slide: the list holds still while a trigger is typed (a new snippet waits at the bottom), then re-sorts with each row sliding to its new place; no slide when Windows animations are off. Saving keeps the file's own order, new snippets appended. (Revised 2026-09-30 after the smoke test; was file order.) |
| Save | Explicit Save (button or Ctrl+S), per tab. |

Out of scope: a settings window, snippet variables (date, clipboard), the version bump and release.

## Components

| Unit | File | Job |
|---|---|---|
| Entry point | `src/main.rs` | `wants_editor(args) -> Option<Tab>`: `--dictionary` → `Tab::Dictionary`, `--snippets` → `Tab::Snippets`, anything else → `None` (the app). Still runs before the app's single-instance mutex. `TrayEvent::OpenSnippets` becomes `TrayEvent::EditSnippets`; both edit events spawn `current_exe()` with their flag via one helper. A spawn failure logs and shows a tray notice. |
| Editor process shell | `src/editor.rs` (renamed from `src/dictionary_editor.rs`) | Title `Murmur — Dictionary & Snippets`, mutex `Local\Murmur.DictionaryEditor` (unchanged, so an editor from the previous version is still detected). Hosts both panels, draws the tab strip, owns the active tab. `run(tab)` opens on that tab. `is_open()` unchanged. |
| Dictionary panel | `src/dictionary_panel.rs` | Unchanged behaviour. `RED` and `open_file` move to `src/editor_kit.rs`. |
| Snippets panel | `src/snippets_panel.rs` (new) | `SnippetsPanel` (working copy, selection, search, test text, undo slot, stamp, banner) with `ui`, `is_dirty`, `save`. Knows nothing about windows. |
| Shared UI helpers | `src/editor_kit.rs` (new) | `RED`, `open_file`, `reduced_motion` and `progress` moved from `dictionary_panel.rs`, plus `has_comments(path)`. The banner and save bar stay in each panel (their wording differs). |
| Snippet editing logic | `src/snippets_edit.rs` (new, lib, egui-free) | `validate`, `normalize`, `visible`, `delete`/`undo`, `changes`, `preview`. Unit-tested without a UI. |
| Stamped load/save | `src/config.rs` + `src/dictionary.rs` + `src/snippets.rs` | `Stamp`, `SaveOutcome`, `file_stamp` move from `dictionary.rs` to `config.rs` (re-exported from `dictionary` so existing callers compile). New `config::write_with_backup(path, contents)` does the `.bak` / `.tmp` / rename; `Dictionary::save_to` calls it after its loaded-cleanly check. `Snippets` gains `Serialize`, `to_toml`, `load_stamped(path)` and `save_if_unchanged(path, stamp)`, shaped like the dictionary's. |
| Styling | `src/correction_ui.rs` | Reuse `BG`, `GREEN`, `AMBER`, `MUTED`, `TEXT`, `load_system_font`. |

The pipeline's `SnippetFile::refresh` is unchanged; a save by the editor changes the modified time and the next dictation picks it up.

## Behaviour

### Window and tabs

```
 Dictionary   Snippets •                                   ← tab strip
 ─────────────────────────────────────────────────────────
 <active panel>
```

- The tab strip sits above the panel. A tab with unsaved edits shows " •" after its name.
- Switching tabs keeps each panel's state (selection, search, unsaved edits, banners).
- **Ctrl+S** saves the active tab only.
- **Close** with unsaved edits in either panel: the close is cancelled and one prompt asks "Save changes to the dictionary and snippets?" (or names just the one with edits) with **Save / Discard / Cancel**. Save saves every dirty panel; if any save fails (validation, conflict, I/O), the window stays open on the first failing tab with its banner.
- The updater's notice becomes "Close the Dictionary & Snippets window to update".

### Snippets tab layout

```
[Search…        ] [+ New snippet]              [Open snippets file]
┌───────────────┐ ┌────────────────────────────────────────────┐
│▸my signature  │ │ Say:   [my signature                    ]  │
│ my address    │ │ Paste: ┌──────────────────────────────┐    │
│               │ │        │Best,                         │    │
│               │ │        │Jeff                          │    │
│               │ │        └──────────────────────────────┘    │
│               │ │                          [Delete snippet]  │
└───────────────┘ └────────────────────────────────────────────┘
Test (snippets only): [thanks my signature]
                      → thanks Best,
                        Jeff
<banner line: conflict / error / comments note>
                          [Undo delete]  1 unsaved change  [Save]
```

Same panel structure as the dictionary (top toolbar, bottom footer, resizable left list, central editor), so the footer can't be pushed off-screen.

### Open

- `Snippets::load_stamped()` gives the working copy and the stamp. A missing file is an empty list.
- Parse error: the panel shows the error, **Open snippets file** and **Retry**, and nothing else. Save is impossible (`is_dirty()` is false while the load failed).
- If the file has any line whose first non-space character is `#`, the note "Saving removes comments from snippets.toml" shows near Save. The seed file is all comments, so the note shows until the first save. `.bak` keeps the previous version.

### Edit

- The list shows each trigger; an empty trigger shows as a muted "(no trigger)".
- Search filters by trigger or text, case-insensitive.
- **Say**: one single-line field. **Paste**: a multi-line field, 5 rows minimum, growing with its content inside the editor's scroll area. Tab inside Paste moves focus rather than inserting a tab character.
- **+ New snippet** appends `{trigger: "", text: ""}`, selects it, clears the search, and focuses Say. If the search found nothing, Enter in the search box does the same and takes the search text as the trigger (as the dictionary does).
- **Delete snippet** removes it into the single undo slot, replacing whatever was there. **Undo delete** reinserts it at that index, clamped. The slot is cleared by Save and by Reload.
- Dirty count: the number of snippets that differ from the loaded copy (added, removed or changed), counted like `dictionary_edit::changes`.

### Validation (on every change)

A trigger's **words** are its whitespace-separated tokens with punctuation stripped and lower-cased, exactly as `Snippets::mark` matches them.

Errors, shown in red next to the field; they disable Save:
- trigger with no words (empty, or punctuation only);
- text empty (the whole text, untrimmed, is `""` or only whitespace);
- two snippets whose triggers have the same words ("My email." and "my email" collide).

A new snippet's empty-trigger and empty-text errors wait until the user leaves the field or tries to save, as the dictionary does for a new term's written form.

Warning, shown in amber; it doesn't block Save:
- a one-word trigger that is a common word (`phonetic::is_common`): "this will paste every time you say '{word}'".

### Test a phrase

When the test text or the snippets change, the result is `snippets_edit::preview(&working, text)`: `mark` then `restore` using the normalised working copy, skipping snippets with errors. Multi-line results show on several lines. Dictionary and cleanup are not applied; the label says "snippets only". Empty text shows nothing.

### Save

Normalise: trim the trigger; the text is kept exactly as typed. Then `save_if_unchanged(stamp)`:
- **Saved**: take the new stamp, reset the loaded copy to the saved one, clear the dirty count and undo slot.
- **Conflict**: banner "snippets.toml changed outside the editor." with **Reload** (discard my edits) and **Overwrite** (save anyway, take the new stamp; still refuses invalid snippets).
- **I/O error**: red banner with the error; edits are kept.
- Saving with no changes writes nothing (no `.bak` churn), as the dictionary does.

### File format

`toml::to_string_pretty` writes `[[snippet]]` tables. Text containing a newline is written as a `"""` multi-line string (`toml_writer` picks that encoding by default), so the saved file stays hand-editable. Text round-trips exactly, including leading, trailing and blank lines.

## Testing

Unit tests (`cargo test --release --locked`):
- `snippets_edit::validate`: no-word trigger; punctuation-only trigger; empty and whitespace-only text; duplicate triggers ignoring case and punctuation flag both; a common one-word trigger is a warning, not an error; a common word inside a longer trigger is not warned.
- `normalize` trims triggers and leaves text untouched; `visible` filters by trigger and text ignoring case, A–Z by trigger words; delete then undo restores the position and clamps; `changes` counts added, removed and edited; `preview` uses unsaved snippets and skips invalid ones.
- `Snippets` round-trip: multi-line text with leading/trailing newlines survives `to_toml` → `from_toml`, and the output contains `"""`.
- `save_if_unchanged` in a temp directory: `Saved` when the stamp matches; `Conflict` after an outside write; a missing file saves.
- `config::write_with_backup`: writes, keeps `.bak` of the previous version, leaves no `.tmp`. The existing dictionary save tests still pass.
- `SnippetsPanel` (temp dir, as `dictionary_panel` tests do): edit and save; second save is not a conflict; conflict then Overwrite / Reload; invalid snippets block save and leave the file alone; broken file shows the error and never saves; save without changes leaves the file and backup alone; comments detected.
- Headless egui: the snippets layout fits a 560×420 window with the footer visible; a snippet can be added and typed without the mouse and shows in the test box.
- Editor shell: the close prompt appears when only the inactive tab is dirty; Save that fails on one tab keeps the window open and switches to that tab; Esc dismisses the prompt like Cancel (existing test kept).
- Argument parsing: `--dictionary` → Dictionary, `--snippets` → Snippets, no argument → app.

Baseline test counts are recorded before the first change; all existing tests must still pass.

Manual smoke on the installed build (AppData writes go through an Explorer-launched script because the Claude desktop app's AppData is MSIX-virtualized):
1. Tray → Snippets… opens the window on the Snippets tab; Tray → Dictionary… while it's open brings the same window forward.
2. Add a multi-line snippet → Save → dictate its trigger into Notepad: expanded, no restart.
3. Open `snippets.toml`: the text is a `"""` block.
4. Unsaved edits, hand-edit the file, Save: conflict banner; Reload and Overwrite both work.
5. Edits on both tabs, close: one prompt; Save writes both files.
6. Dictionary tab still behaves as before.

No test, fixture or screenshot uses Jeff's local `snippets.toml` or `dictionary.toml` contents.

## Docs

README: the tray table row "Open snippets" becomes "Snippets…" (add, edit, delete and test snippets); "Dictionary…" row unchanged. The Snippets section describes the Snippets tab first, keeping the `snippets.toml` format as a power-user note. The dictionary section's "the editor" wording stays accurate.
