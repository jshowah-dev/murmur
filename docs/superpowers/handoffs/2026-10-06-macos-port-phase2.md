# Handoff: Murmur on macOS, phase 2 continued (2026-10-06)

**State:** the egui windows work on macOS, and fix-last's card runs in its own process so the menu bar icon keeps working. Everything is pushed, and Windows CI is green. Next: the editor's ⌘S, then the rest of phase 2.

Supersedes `docs/superpowers/handoffs/2026-10-05-macos-port-phase2.md`; its state and first actions are out of date, and its traps are copied below. `docs/superpowers/handoffs/2026-10-05-macos-port.md` still stands for layout, build/run/test, signing, pushing, the decisions table, and the phase 3–4 lists.

## State

| | |
|---|---|
| Branch | `feat/macos-phase0`, pushed through `29e62b1`, clean. No PR. |
| Commits this session | `e4e862a` mote dwell (250 ms macOS, 0 Windows) · `50231b1` speaker mute (CoreAudio) · `ea96793` Open at Login (SMAppService) · `6667dd7` egui windows open, pill survives them · `27f73d6` fix-last card in its own process, ⌘ shortcuts, `move_to`/`window_rect` · docs `d0430cb`, `29e62b1` |
| Windows CI | Passed on `29e62b1`, no warnings (run 37413724913). |
| Running app | `target/release/Murmur.app` at `29e62b1`, registered as a login item. |
| gh account | Jeff switched the active `gh` account back to `jshowah-dev`; another session uses `showjefb`. A git-guard hook blocks pushes to this repo under the wrong account. |

## First actions, in order

1. `git status -sb` → clean, matches `origin/feat/macos-phase0` at `29e62b1`.
2. Ask Jeff to try the editor: menu › Dictionary…, edit, **⌘S** saves; same for Snippets…. It's the only change from this session not yet tried.
3. Phase 2, remaining list below.

## Verified, and how

- Dwell, speaker mute: Jeff, by eye and ear. Mute round-trip is also unit-tested against the real speakers.
- Open at Login: Jeff saw the menu checkmark. **UNPROVEN:** Murmur starting after a restart.
- History, key picker, About: open and close, no Dock icon, menu works afterwards. Jeff, on `6667dd7`.
- Fix last: Jeff opened it twice in a row from the menu on `27f73d6` (log 04:23:13 and 04:23:25), so the menu works after the card. I also ran the card process alone: it appeared at the anchor, took the keyboard (Esc closed it), and replied on stdout. **UNPROVEN:** that ⌘Enter replaced the text in Jeff's test; he only said "it seems to work".
- **UNPROVEN:** editor ⌘S; first-run setup window on macOS.

## What a green build will not tell you

- **Never call `finishLaunching` in `platform::init_app`.** winit 0.30.13 sets itself as the NSApp delegate when eframe makes its event loop (the first egui window), then waits for `applicationDidFinishLaunching:`. If Murmur has already finished launching, the app loop hangs forever. Symptom: the log shows `ptt down` from the hotkey thread, then nothing.
- **winit closes every NSWindow when an egui loop exits** (`notify_windows_of_exit`), including the pill's `Panel` and the mote's `Stage`. `platform::mac::front` orders a panel front whenever `!isVisible()`, and the resting pill repaints about once a second. Don't bring back a "shown" flag.
- **Fix-last's card must stay out of process on macOS** (`correction_ui::card_process`, `murmur --fix-card`). In-process, after the card closed, clicks still reached the status item's window (`NSStatusBarWindow`, number 4294967296), but its menu never opened. Root cause not found. Ruled out: the text field's focus (it still locked with focus never requested), and TextInputUI's hidden `TUINSWindow` (closing it didn't help). History, the key picker and About stay in-process and haven't caused the lock.
- Protocol: stdin carries `heard_ms|-`, then `caret|area l t r b` or `-`, then the text. stdout carries `x y|-`, then `=edited` or `!` for cancel. A child that dies says nothing, which reads as cancel. The card process loads the dictionary from disk itself.
- No Dock icon, because of `LSUIElement` (winit leaves the activation policy alone for bundled apps). An unbundled `cargo run` would get one.

## Facts you'd otherwise re-derive

- **Only Jeff's real clicks can test the menu bar.** macOS drops synthetic clicks there, whether they come from osascript, Swift `CGEventPost`, or Murmur's own `CGEventPost`. An AX press on the status item doesn't open a tray-icon menu either, because tray-icon attaches the menu only inside `mouseDown:`.
- `screencapture` fails (Claude has no Screen Recording permission). To see Murmur's windows, use `CGWindowListCopyWindowInfo` filtered by pid (a few lines of Swift, `swiftc` works). An open tray menu shows as layer 101, onscreen, alpha > 0.
- Once an egui window has run, System Events sees Murmur's menu bars: bar 1 is winit's default app menu, the last bar is the status item. Before that it sees none.
- Jeff blocked blind osascript key presses twice. Ask before sending keystrokes, and prefer that he clicks while you read `~/Library/Application Support/Murmur/murmur.log`.
- `sample <pid>` hung for 120 s; don't use it. Foreground `sleep` over a minute is blocked; wait with a background until-loop.
- `mote.rs` tests live in the app crate: `cargo test --release --bin murmur mote`.
- Tray notifications are a no-op on macOS (phase 3), so `tray.notify` errors (e.g. Open at Login needing approval) only reach the log.
- History isn't saved to disk: after a restart, Fix last says "Nothing to fix yet" until you dictate.

## Phase 2: remaining

- The editor's ⌘S (first actions).
- `platform::mac::raise_titled` is still a no-op (the editor's second launch can't bring the first forward). Finding another process's window by title needs Screen Recording, so this needs a different approach.
- `caret::work_area` uses the main screen only (TODO for a second display).
- First-run setup should walk through Accessibility, and optionally Input Monitoring; it's untested on macOS.
- UI font: SF Pro, not explicitly checked; Jeff hasn't flagged anything.
- `keep_on_top` is a documented no-op on macOS (the floating level holds); that one is done.

## Open items

- Root cause of the in-process menu lock: parked, worked around by the card process.
- The pill's right-click menu and hover hint on macOS: parked.
- PR for `feat/macos-phase0` now or after phase 2: not asked.
- Before merging to `main`, Jeff smoke-tests the Windows build on his PC (pill, fix-last, editor). Still stands from the first handoff.
