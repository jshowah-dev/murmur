# Murmur answers where you acted

Date: 2026-10-02. Status: design approved in chat, pending review of this written spec.

## Goal

When you do something in Murmur, the answer shows up where your eyes already are: at the caret, carried by the mote. Today three things fail that:

1. **Nothing heard is silent.** If you hold the key and the speech decodes to nothing, the mote fades or nothing happens. "Didn't hear you" looks the same as "still working".
2. **Your own actions are answered in corner balloons.** Fix-last's "Learned", "Corrected text copied", "nothing to fix yet" and History's "nothing dictated yet" all go to the tray. One balloon replaces another: "Corrected text copied" straight after "Learned" hides it, which is the likely cause of "Learned shows only about half the time".
3. **A failed paste is a raw corner balloon** ("paste failed: …"), and it doesn't say your words are safe.

## Decisions (Jeff, 2026-10-02)

| Question | Decision |
|---|---|
| Nothing heard: how much to say | **Plain**: "Didn't catch that". No diagnosis (muted mic, too short). |
| Where answers appear | **At the caret**, falling back to just above the pill when no caret is found. |
| Fix-last confirmation | **One tag where the correction lands**: "Learned hob → HAWB", or "Copied, press Ctrl+V to paste", or both in one tag. No balloons. |
| Clipboard write fails during paste | **Type the text in instead**, as already happens when the clipboard can't be opened. No message. |
| Look and feel | **The mote speaks**: the mote stretches into a capsule holding the words, holds, then pulls back to a dot and dissolves. |
| Voice | Plain, not first person. |
| Hold time | New kit token `duration.read_per_char = 60ms`; hold = `max(duration.locate, read_per_char × characters)`. |

Out of scope: dictation errors (model failed to load, a broken dictionary or snippets file found while dictating) stay balloons and go with audit item 7, startup errors. Background events (updates, model download, mic lost) stay balloons. No first-person copy. No diagnosis of why nothing was heard.

## Behaviour

| Event | Trigger | Answer |
|---|---|---|
| Nothing heard (no text came back) | releasing the key | The mote at the caret unfurls "Didn't catch that". With no caret, a mote rises from the pill and unfurls just above it. |
| Fix-last saved, words learned | Ctrl+Enter | A mote flies from the card's centre to the caret where the correction landed and unfurls "Learned hob → HAWB". Several terms are joined with ", ". |
| Fix-last saved, nothing learned, replaced | Ctrl+Enter | The mote dissolves into the corrected text with no words, as a normal dictation lands. |
| Fix-last couldn't replace (older than 60 s, not pasted, window changed, or undo-and-paste failed) | Ctrl+Enter | "Copied, press Ctrl+V to paste", with the text on the clipboard. If something was also learned: "Learned hob → HAWB · Copied, press Ctrl+V". |
| Fix last, nothing to fix | Shift + talk key | "Nothing to fix yet" at the caret. |
| Fix last, nothing to fix | tray menu | The same, above the pill. |
| History, nothing dictated | tray menu | The History window opens anyway and says "Nothing dictated yet. Hold Right Ctrl and speak.", naming your actual talk key. |
| Clipboard write fails during paste | releasing the key | The text is typed in. No message. |

The tray item still reads "Fix last (Left Shift+PTT)"; its copy is audit item 10, not this change.

## Motion and look

| Phase | What happens | Token |
|---|---|---|
| Arrive | The mote's existing flight (`FLIGHT` 300 ms, upward arc). Skipped if the mote is already settled at the caret; if it is still flying, it finishes first. Fix-last flies from the card's centre; tray-triggered messages from the pill. | `FLIGHT`, `easing.enter` |
| Unfurl | The dot stretches sideways into a capsule; the words fade in once it is about 70% wide. | `duration.enter`, `easing.enter` |
| Hold | `max(duration.locate, read_per_char × characters)`: about 1.2 s for "Didn't catch that", about 2.5 s for the longest message. Ends early on a new dictation or a foreground-window change. | `duration.locate`, `duration.read_per_char` (new) |
| Furl | The words fade, the capsule pulls back to a dot, and the dot dissolves (the existing dissolve). | `duration.exit`, `easing.exit` |
| Reduced motion | No flight and no stretch: the tag appears at full size, holds for the same time, and disappears. | none |

Look:

- **Material:** the live pill's body, `#202020` at alpha `0xE6`. Text is the system font (Segoe UI Variable, falling back to Segoe UI) at 12 pt scaled for DPI, colour `#EEEEEE`.
- **"Learned" colours:** the fix-last card's: the spoken form in `SPOKEN` (`#D98C7A`), the written form in `AMBER` (`#F5C54A`), the arrow and separators muted (`#999999`).
- **Placement:** the capsule sits above the caret line, its left edge at the caret, so it never covers what you are about to type. It is kept inside the caret's monitor work area. Click-through, never activates.

## Components

| Unit | Change |
|---|---|
| `src/canvas.rs` | New `text(x, y, spans, px)`: GDI draws the spans into a 32-bit DIB with grayscale antialiasing (ClearType doesn't composite on a layered window); the coverage becomes premultiplied pixels in each span's colour. Also a way to measure spans, for the capsule's width. |
| `src/mote.rs` | `Flight` gains `Unfurl`, `Hold` and `Furl` phases carrying the message. New `Mote::say(message, from)`. The mote window resizes from its fixed 40 px square to the capsule's size while speaking. A `dismiss()` starts the furl early. |
| `src/motion.rs` + kit | `READ_PER_CHAR` added to `~/.claude/skills/considered-ux/kits/jeff.tokens.json` and emitted with `emit-tokens.mjs --target rust`; then `kit.mjs --check`. |
| `src/correction.rs` | `fix_last` returns a `FixOutcome { learned, copied, nothing_to_fix }` and no longer calls `tray.notify` for these cases. A pure `fix_message(&FixOutcome) -> Option<Message>` builds the spans. The "dictionary not loaded" balloon stays. |
| `src/correction_ui.rs` | `show` also returns the card's last centre in physical pixels, the flight's start. |
| `src/main.rs` | Wires each Behaviour row to `mote.say`. Nothing heard is a `Done` with empty text. After a fix-last replace, the caret is read again for the landing point. A new key-down or a foreground change dismisses a speaking mote. The History tray item opens the window even with no items. |
| `src/inject.rs` | In `paste`, when the clipboard write fails, restore the saved text and type `text` in, returning a `Typed` record. |
| `src/history_ui.rs` | Empty state with the talk key's label; `show` takes the label. |

## Testing

Unit tests, written first:

- `fix_message` for each `FixOutcome`: learned only, copied only, both, neither.
- `Flight`: unfurl, hold and furl timing; the hold equals `max(locate, 60 ms × characters)`; `dismiss` starts the furl from where it is; a `say` mid-flight finishes the flight first; reduced motion goes straight to full size and still holds.
- `canvas::text`: non-empty, premultiplied pixels; spans keep their colours.
- History's empty state draws "Nothing dictated yet" with the given key label.

Frames: strips of the unfurl, normal and `--reduced-motion`, from a debug build with `MOTION_TIME_SCALE=10` that Jeff starts, using `frames-native.ps1`; then a zombie-critic subagent pass on the strips.

Smoke, by Jeff on a release build: each Behaviour row once.

Not unit tested: the clipboard-write fallback in `paste` (forcing a clipboard failure needs a seam that isn't worth it); covered by the smoke run where possible.

## Known limits

- Windows silently blocks Murmur's keystrokes into apps running as administrator. Murmur gets no error, so nothing can answer it.
- A failure while typing the text in can't be detected, so it has no message.
- When no caret is found (some apps don't expose one), every answer appears above the pill instead.
