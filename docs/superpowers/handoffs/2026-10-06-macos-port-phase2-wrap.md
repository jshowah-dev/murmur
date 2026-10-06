# Handoff: Murmur on macOS, phase 2 nearly done (2026-10-06)

**State:** everything is pushed and Windows CI is green at `b5b2526`. Phase 2 code is done apart from `raise_titled`, which is parked for a design call. What's left is mostly Jeff's hands-on checks, then the decision about a PR.

Supersedes `docs/superpowers/handoffs/2026-10-06-macos-port-phase2-setup.md` (its update banner is folded in here). What still stands from earlier handoffs:
- `2026-10-06-macos-port-phase2.md`: the traps and "facts you'd otherwise re-derive" sections. Read them: never call `finishLaunching`, the fix-last card stays out of process, only Jeff's real clicks can test the menu bar.
- `2026-10-05-macos-port.md`: layout, build/run/test, signing, pushing, decisions, and the phase 3–4 lists.

## State

| | |
|---|---|
| Branch | `feat/macos-phase0` at `b5b2526`, pushed, clean. No PR. |
| Windows CI | Green on `b5b2526` (run 37418629810). |
| This session's commits | `e4e862a` mote dwell · `50231b1` speaker mute · `ea96793` Open at Login · `6667dd7` egui windows open, pill survives them · `27f73d6` fix-last card in its own process, ⌘ shortcuts, `move_to`/`window_rect` · `6316ec9` editor ⌘S tests · `61d9210` work area per screen · `62610bd` setup Access step · `c1e6e74` first words survive the mute · docs `d0430cb` `29e62b1` `b4d0717` `5abea28` `47f4981` `b5b2526` |
| Running app | Jeff's Murmur runs `target/release/Murmur.app` built at `c1e6e74`'s code (the latest), and it's a login item. `mute_output = true` in his config (it was briefly false for a test, then restored). |
| Toolchain | Command Line Tools updated to Xcode 27.0 (ld-27037.1), so builds link against `MacOSX27.0.sdk` with no `SDKROOT` workaround. |
| gh | The active account must be `jshowah-dev`; a git-guard hook blocks pushes otherwise. Another session uses `showjefb`: if a push is blocked, ask Jeff to run `gh auth switch -u jshowah-dev`. Don't switch it yourself. |

## First actions, in order

1. `git status -sb` should show clean, matching `origin/feat/macos-phase0` at `b5b2526`.
2. Ask Jeff for the hands-on checks still owed. Batch them into one message:
   - Dictionary… and Snippets…: edit, then **⌘S** saves.
   - After a Mac restart, Murmur starts on its own (Open at Login).
   - With a second display, if he has one: Fix last on that screen opens on it.
3. Ask Jeff the open decisions: PR now or after phase 3, and whether to keep `raise_titled` parked.

## Verified, and how

- **Jeff, live:** mote dwell, speaker mute, History / key picker / About (menu works after them), the fix-last card (menu works after it), and first words kept with a video playing and the mute on.
- **Replayed:** the mute fix, on 8 of Jeff's real dictations. Every word came out, none of the video's. The recordings are deleted: they were his voice.
- **Unit-tested only:** editor ⌘S; `nearest_screen` across two displays; the setup Access step; `vad` segments keep the mic's level (that test fails with the old drain).
- **Smoke by me:** the editor process opens; a card at a corner caret stays inside the usable area; an untrusted ad-hoc copy shows the setup card.
- **UNPROVEN:** ⌘S pressed in the real editor; Murmur starting after a restart; a real second display; how the Access step looks and its move-on after allowing; whether first-run setup (still in-process) kills the menu-bar menu like fix-last did.

## What a green build will not tell you

- **`vad::GAIN` is 7 on macOS, 1 elsewhere.** The detector hears the mic 7× louder; segments are cut from `fed` (the real level), never from the detector's copy. Silero on the quiet MacBook mic started segments up to 0.8 s late. A synthetic quiet WAV does *not* reproduce this, so only real dictations prove it.
- **The pre-roll is dropped when the mute finds the output running** (`OutputMute::mute()` returns `kAudioDevicePropertyDeviceIsRunningSomewhere`). Otherwise it carries the video's words. Windows' `mute()` always returns false, so Windows behaves as before.
- The traps in `2026-10-06-macos-port-phase2.md` still apply: `finishLaunching`, winit closing every window when its loop exits, and the fix-last card staying out of process.

## Facts you'd otherwise re-derive

- **Debugging dictation:** dump what the pipeline receives to a WAV (16 kHz mono, 16-bit) at stop, then replay it in a throwaway `tests/zz_*.rs` with `murmur_lib::{vad::Vad, stt::Recognizer}` and compare variants. That cracked the mute bug in one round. Delete the dumps afterwards.
- **To see an untrusted Murmur** (the Access step), launch an ad-hoc-signed copy with `open -n --env HOME=<scratch home>`. Launched from the shell, macOS judges the shell's app (Claude, trusted) instead. Kill the copy afterwards: it runs a full Murmur with its own hotkey.
- The pre-existing clippy note `manual_is_multiple_of` in `app.rs` (`resting_tick % 64`) isn't from this work.

## Open items

- `raise_titled` (the editor's second launch can't bring the first forward): parked. Looking up a window by title needs Screen Recording; a design call for Jeff.
- The root cause of the in-process menu lock: parked, worked around.
- If first-run setup turns out to lock the menu, move it out of process like `correction_ui::card_process`.
- Assumptions taken without Jeff on the Access step: the card's wording, and Input Monitoring not asked for (only Esc, F-key or Caps Lock push-to-talk keys need it).
- The pill's right-click menu and hover hint: parked.
- PR timing: Jeff's call. Before merging to `main`, Jeff smoke-tests the Windows build on his PC (pill, fix-last, editor).
- Next phases: phase 3 (email formatting via bundle id, notifications, the pill's right-click menu; `grep -rn "TODO(macos" src`), then phase 4 (Developer ID, notarization, DMG, a macOS job in `release.yml`, an updater).
