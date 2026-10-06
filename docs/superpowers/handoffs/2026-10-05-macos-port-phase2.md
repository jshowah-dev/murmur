# Handoff: Murmur on macOS, phase 2 under way (2026-10-05, evening)

**State:** dwell, speaker mute, Open at Login done and pushed; the egui windows open on macOS (History, key picker, About confirmed by Jeff); ⌘ shortcuts written but **uncommitted and untested**.

Supersedes `docs/superpowers/handoffs/2026-10-05-macos-port.md`. Its state section and "Do next" are out of date; its layout, build/run/test steps, signing, pushing, decisions table, and phases 2–4 lists still stand.

## State

| | |
|---|---|
| Branch | `feat/macos-phase0`, pushed through `6667dd7`. No PR. |
| New commits | `e4e862a` mote dwell (250 ms macOS, 0 Windows) · `50231b1` speaker mute (CoreAudio) · `ea96793` Open at Login (SMAppService) · `6667dd7` egui windows open + pill survives them |
| Windows CI | Passed on `50231b1` (run 37405343484). Not run since; `ea96793` and `6667dd7` touch only Mac files (`autostart/mac.rs`, `platform/mac.rs`, a Cargo.toml objc2-foundation feature `NSError` under the macOS target). |
| Working tree | Uncommitted: `correction_ui.rs`, `dictionary_panel.rs`, `snippets_panel.rs`, `editor_kit.rs`. Fix-last submit and editor Save use `Modifiers::COMMAND` (⌘ on Mac, Ctrl on Windows); hints use new `editor_kit::CMD` (`"Ctrl+"` / `"⌘"`). Builds, all tests pass, bundled and running. |
| Running app | `target/release/Murmur.app` built from the working tree above. Registered as a login item (Jeff ticked Open at Login). |

## First actions, in order

1. Ask Jeff to test the uncommitted build (it's what's running):
   - Fix last: dictate, menu › Fix last dictation, edit, **⌘Enter** → card closes, fixed text replaces the paste.
   - Menu › Dictionary… and Snippets… → editor opens (it's a separate process: `murmur --dictionary`); edit, **⌘S** saves.
   - After each window closes, the pill is still there.
2. If good, commit the four files (`feat(macos): ⌘ for fix-last's submit and the editor's save`), then `gh workflow run release.yml --ref feat/macos-phase0` — these files compile on Windows too. Expected: success, no warnings.
3. Continue the phase 2 list below.

## Verified, and how

- Dwell, speaker mute: Jeff, by eye/ear, on the Mac. Mute round-trip also unit-tested against the real speakers.
- Open at Login: Jeff saw the menu checkmark. **UNPROVEN:** Murmur actually starting after a restart.
- History, key picker, About open and close; no Dock icon: Jeff, on the Mac build `6667dd7`.
- Pill returns after History closes: Jeff.
- **UNPROVEN:** fix-last card, editor (dictionary/snippets), first-run setup window, all on macOS.

## What a green build will not tell you

- **Never call `finishLaunching` in `platform::init_app`.** winit (0.30.13) sets itself as NSApp delegate when eframe makes its event loop (first egui window) and waits for `applicationDidFinishLaunching:`. If Murmur already finished launching, winit's `app.run()` never gets it and the app loop hangs forever (symptom: log shows `ptt down` from the hotkey thread, then nothing). winit's own `run` now finishes the launch.
- **winit closes every NSWindow when an egui loop exits** (`notify_windows_of_exit`), the pill's `Panel` and mote's `Stage` included. `platform::mac::front` orders a panel front whenever `!isVisible()`; the resting pill repaints about once a second (`refresh_resting`, every 64 ticks), so it comes back within ~1 s. Don't reintroduce a "shown" flag for ordering front.
- Dock icon: none, because the bundle has `LSUIElement` and winit leaves the activation policy alone for bundled apps when none is given. An unbundled `cargo run` would get a Dock icon.

## Facts you'd otherwise re-derive

- Scripted driving is limited: System Events can click Murmur's menu-bar item, but menu items aren't readable by script, `screencapture` fails (no Screen Recording for Claude), and since `finishLaunching` was removed Murmur's windows no longer show up in System Events' window list. Jeff blocked blind menu/key scripting twice; ask him to click and read `~/Library/Application Support/Murmur/murmur.log` alongside.
- `sample <pid>` hung for 120 s here; don't use it.
- `mote.rs` tests are in the app crate: `cargo test --release --bin murmur mote`.
- Tray notifications are a no-op on macOS (phase 3), so `tray.notify` errors (e.g. Open at Login needing approval) only reach the log.

## Phase 2: remaining

- Fix-last and editor: test (first actions).
- `platform/mac.rs` stubs still empty: `keep_on_top`, `move_to`, `window_rect` (fix-last positions itself with these), `raise_titled` (editor's second launch). `caret::work_area` uses the main screen only.
- First-run setup should walk through Accessibility (and optionally Input Monitoring).
- UI font: SF Pro (`/System/Library/Fonts/SFNS.ttf`) — Jeff didn't flag the font on History/key picker/About; not explicitly checked.

## Open items

- Pill's right-click menu and hover hint on macOS (panel ignores the mouse): not done, parked.
- Whether to open a PR for `feat/macos-phase0` now or after phase 2: not asked.
- Before merging to `main`: Jeff smoke-tests the Windows build on his PC (pill, fix-last, editor) — from the previous handoff, still stands.
