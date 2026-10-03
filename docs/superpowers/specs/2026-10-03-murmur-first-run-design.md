# Murmur first run: the card goes home

Date: 2026-10-03. Status: design approved in chat, pending review of this written spec.

## Goal

The first-run setup window is the odd one out (UX audit item 6, 2026-10-02):

1. **It doesn't look like Murmur.** It is the only window still on stock egui chrome: a title bar, default buttons, plain text. About and History use the frameless card, the `BG`/`BORDER` palette and `keycap`.
2. **Its errors are raw.** A failed download shows `FetchError`'s `Display`, e.g. "Download interrupted (os error 10054)"; `Io` shows the bare OS text. Nothing says what to do.
3. **"You're ready" only tells.** It describes the key and where the tray icon is, then waits for "Got it". Nothing invites you to try it.

## Decisions (Jeff, 2026-10-03)

| Question | Decision |
|---|---|
| Carved moment | **C-lite, "the card goes home"**: the card fades to a dot, the mote carries it from where the card was to the pill, and the pill invites you to talk. Chosen over B (the pill invites, no flight) and A (dictate into the setup card); the full C (a new layered window flying on to the tray) was cut for cost. |
| When the invitation stops coming back | **After your first dictation with words.** Quitting before trying brings it back next launch. |
| After the first dictation | **Nothing extra.** The words landing is the answer; a "That's it" would be a celebration for a routine act. |
| Errors | Say what happened and what to do, at the Retry button. Raw detail goes to the log only. |

Out of scope: startup errors as balloons (audit item 7), the editor's size and position (8), raw durations in `correction_ui.rs`/`history_ui.rs` (9), the tray's "Fix last (Left Shift+PTT)" copy (10).

## Behaviour

| Case (`setup_ui::plan`) | What you see |
|---|---|
| `Download { then_ready: true }`: first run, model missing | The card downloads and unpacks; then "Ready" for one beat; the card fades to a dot; the mote carries the dot to the pill and unfurls the invitation. |
| `Download { then_ready: false }`: welcomed before, model missing | The card downloads and unpacks, then closes without a beat or a dot, as today. No invitation. |
| `ReadyOnly`: model present, not welcomed (e.g. reinstall) | No card. The invitation unfurls above the pill, rising from the pill's centre. |
| `Skip` | Nothing changes from today. |

**The invitation:** "Hold Right Ctrl and talk", naming your actual talk key (`cfg.ptt_key_label()`), with the key in `GREEN` (`#60D060`) and the rest in `TEXT`. It is said above the pill (`Target::Pill`) and **stays open until dismissed**. It does not time out after `hold_for`, and switching windows does not close it (`said_in` stays `None`).

| While the invitation is open | Result |
|---|---|
| Talk key down | Dismissed (the existing key-down dismiss, `main.rs:431`); the dictation runs as normal. |
| That dictation's `Done` has words | Words land as usual. Write the `welcomed` marker. No further invitation this run or later. |
| That dictation is empty | "Didn't catch that", as usual. Marker not written; the invitation returns next launch, not this run. |
| Fix-last, or "Nothing to fix yet" | Replaces the invitation; the replacement holds for its normal read time. |
| Pause from the tray | Dismissed. |
| Quit | Marker not written; the invitation returns next launch. |

## The card

- **Look:** frameless, transparent viewport, a `BG` card with a 1 px `BORDER` stroke and the About window's corner radius; system font; centred on screen, not resizable, and it keeps its taskbar button (nothing else of Murmur is on screen yet). Window size follows the content, as About does.
- **Copy:** "Murmur needs its speech model (about 460 MB) before first use." (computed from the asset sizes, as now), then the stage line and its bar:
  - "Downloading speech model: N / 460 MB", bar, **Cancel**.
  - "Unpacking speech model: N%", bar held under 100% until tar exits, no Cancel (close refused, as today).
  - Error text (below), **Retry · Quit**. Retry has keyboard focus, so Enter retries.
  - "Ready", one beat of `duration.locate`, then Closing.
- **Esc** counts as Cancel while downloading, and as Quit on an error. There is no title-bar X.
- **Closing (the fade to a dot):** over `duration.exit` with `easing.exit`, the card's contents and background fade out until only a dot is left at the card's centre. The dot is the mote's: a `#E8FFE8` core of `DOT` (5 px at 96 dpi) with a `#60D060` halo of radius 12 at alpha 0.5. Opacity only; nothing changes size or layout. Then the window closes and `run` returns the card's centre in physical pixels (`outer_rect` centre × `native_pixels_per_point`).
- **Reduced motion:** no Closing. The window closes after the "Ready" beat and `run` returns `from: None`, so the invitation appears above the pill without a flight.

## Errors

`FetchError::advice()` gives the card's text; `Display` stays unchanged for the log and its tests.

| `FetchError` | Card text |
|---|---|
| `Interrupted(_)` | "The download stopped partway. Check your connection, then Retry picks up where it left off." |
| `Http(n)` | "The model's host isn't answering right now (error n). Retry in a few minutes." |
| `ChecksumMismatch` | "The download arrived damaged. Retry fetches it again from the start." |
| `Unpack(_)` | "The model downloaded but wouldn't unpack. Retry unpacks it again." |
| `Io`, kind `StorageFull` | "Not enough disk space. Murmur needs about 1.2 GB free on the drive with your user folder. Free some up, then Retry." |
| `Io`, kind `PermissionDenied` | "Windows blocked Murmur from saving the model in your AppData folder. Retry, or restart and try again." |
| `Io`, anything else | "Couldn't save the model to disk. Retry, or check the log in %APPDATA%\Murmur." |
| `Cancelled` | Never shown (the worker returns silently, as today). |

The tray's model-upgrade failure (`main.rs`, `UpdateMsg::ModelFailed(e.to_string())`) uses `advice()` too, so one error reads the same everywhere.

## Motion

| Phase | What happens | Token |
|---|---|---|
| Ready beat | "Ready" shown, still | `duration.locate` |
| Closing | Card contents and background fade; the dot stays | `duration.exit`, `easing.exit` |
| Carry | The mote's existing flight from the card's centre to just above the pill | `FLIGHT` (break, ledgered 2026-09-30), `easing.enter` |
| Unfurl | The existing unfurl into the capsule | `duration.enter`, `easing.enter` |
| Hold | Until dismissed (new: sticky) | none |
| Furl | The existing furl and dissolve | `duration.exit`, `easing.exit` |
| Reduced motion | No fade, no flight; the invitation appears open above the pill | none |

## Components

| Unit | Change |
|---|---|
| `src/setup_ui.rs` | New card look. `Stage::Ready` shows "Ready" for the beat, then `Stage::Closing { start }` (download path with `then_ready` only). `SetupOutcome::Installed` becomes `Installed { from: Option<Pt> }`. `mark_welcomed` and "Got it" go; `welcome_marker()` stays. `plan()` is unchanged. `run` no longer takes `key` (the card no longer names it). `refuse_close` refuses nothing in Closing. |
| `src/model_fetch.rs` | `FetchError::advice(&self) -> String`. |
| `src/mote.rs` | `Flight` gains a sticky flag: `say_until_dismissed(m, from, to, now)` sets it, and `Speaking` doesn't move to `Furling` while it is set. `dismiss()`, `say()`, `launch()` and `fade()` clear it. `Mote` exposes `say_until_dismissed`. |
| `src/main.rs` | After setup: `let mut invite = !welcome_marker().exists() && !model_missing` (true for `ReadyOnly` and a finished `Download { then_ready: true }`; false for a missing custom `model_dir`, which gets the tray notice instead). Once the overlay and mote exist, if `invite`, say the invitation from `from.unwrap_or(overlay.centre_physical())` to `Target::Pill(overlay.above_physical())`. On the first `Done` with words while `invite`: write the marker (`setup_ui::mark_welcomed`, moved to a `pub fn`) and clear `invite`. Pause dismisses a speaking mote. `ModelFailed` uses `advice()`. |

The `welcomed` marker's write moves out of the window: it was written on "Got it" or the window's X on the Ready screen; now it is written on the first dictation with words.

## Risks

- **UNPROVEN: the gap between the card closing and the mote's first frame**, while the main loop starts. `Overlay::create` and `Mote::create` are synchronous and fast, and the pipeline loads on its own thread, so it should be under a frame or two. Fallback if the smoke shows a blink: create the `Mote` before `setup_ui::run`, so its window already exists.
- **Mixed-DPI monitors:** the card's centre comes from egui's `native_pixels_per_point`; the mote scales by `GetDpiForSystem`. On a single monitor they agree. Mixed DPI stays UNPROVEN, as it is for the mote today.

## Testing

- **Unit:**
  - `advice()` for every variant, including `Io` with `StorageFull`, `PermissionDenied` and another kind.
  - Sticky `Flight`: still `Speaking` well past `hold_for`; `dismiss` furls it; a later `say` is not sticky; reduced motion appears open and stays.
  - `refuse_close`: Unpacking still refused, Closing not.
  - `plan_covers_first_run_cases` unchanged and green.
- **Frames** (considered-ux, `frames.mjs`, normal and `--reduced-motion`): the card's Ready → Closing → dot.
- **Smoke (Jeff, release build):**
  1. Fresh first run: rename `%LOCALAPPDATA%\Murmur\models`, delete `%APPDATA%\Murmur\welcomed`, launch. Card in the family look; download; "Ready"; fade to a dot; the dot arcs to the pill and says "Hold Right Ctrl and talk". No blink between card and mote.
  2. The invitation stays for over 10 s, and stays when you switch windows.
  3. Hold the key: it furls. Dictate into Notepad: the words land, and `welcomed` now exists.
  4. Quit before dictating, relaunch: the invitation returns (no card, rising from the pill).
  5. Unplug the network mid-download: the "stopped partway" text with Retry focused; reconnect and press Enter: it resumes.
  6. Reduced motion on: no fade, no flight; the invitation appears open above the pill.
