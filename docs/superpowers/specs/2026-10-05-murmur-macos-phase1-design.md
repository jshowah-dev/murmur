# Murmur on macOS, phase 1: it types

Date: 2026-10-05. Status: approved in chat ("start phase 1"); scope from the Mac port plan (phase 0 landed in 83480c5).

## Goal

On the Mac, hold **Right Option**, speak, let go: the words are pasted where the cursor is. Murmur lives in the menu bar, with Pause/Resume, Open config folder and Quit. One app loop (`app.rs`) serves both systems; nothing about the Windows app changes.

## Decisions (Jeff, 2026-10-05)

| Question | Decision |
|---|---|
| Audience | Jeff's own Mac for now; open-sourcing the Mac build later. Signing, notarization, DMG and CI wait for phase 4. |
| Code shape | **One codebase, one app loop.** OS calls move behind a small `platform` module with a Windows and a macOS half. No fork of the loop. |
| Default key on macOS | **Right Option** (`ptt_key = "RAlt"`). MacBook keyboards have no Right Ctrl. |

## Findings from the spike (2026-10-05, MacBook Air, macOS 26.6, no permissions granted)

- `CGEventSourceKeyState` for one key returns `false` without Input Monitoring.
- `CGEventSourceFlagsState(HIDSystemState)` works without any permission and carries the device-dependent modifier bits, so Right and Left Option (and Ctrl, Shift, Cmd, either side) are told apart: Right Option `0x40`, Left Option `0x20`.
- A hand-pumped `NSApplication` (`nextEventMatchingMask` with `distantPast`, then `sendEvent`) drives `tray-icon`'s status item from our own loop.
- `NSWorkspace.frontmostApplication.processIdentifier` names the app in front.
- Synthetic Cmd+V (`CGEventPost`) needs Accessibility, which the terminal didn't have.

So: **modifier keys need no permission; Esc and F-keys need Input Monitoring; pasting needs Accessibility.**

## Behaviour on macOS after phase 1

| Feature | Phase 1 |
|---|---|
| Push-to-talk, hands-free double-tap, Shift for fix-last | Works with any modifier key or chord. Esc (cancel hands-free) and F-keys as the talk key only work once Murmur has Input Monitoring; without it they are never seen as pressed. |
| Paste | NSPasteboard + Cmd+V, restoring the previous clipboard text, as on Windows. Needs Accessibility; Murmur asks for it at start-up (`AXIsProcessTrustedWithOptions` with the prompt) when it isn't trusted. |
| Menu bar | Status item with a template icon (light/dark menu bar), dimmed when paused and filled while listening. Same menu as Windows; "Start with Windows" reads "Open at Login". |
| Speech, cleanup, dictionary, snippets, email formatting | As phase 0 (email formatting off until phase 3: `context::detect` answers Plain). |
| Pill, flying mote, caret lookup, speaker mute, open at login, notifications | Phase 3. Placeholders keep their APIs: the pill and mote draw nothing, the caret is never found, mute and autostart do nothing, notifications go to the log. |
| History, fix-last card, key picker, About, first-run setup, dictionary/snippets editor | Phase 2. They compile on macOS (their Win32 calls go through `platform`) and may already work; phase 2 tests and fixes them. |
| Single instance | A lock on `~/Library/Application Support/Murmur/murmur.lock`. |
| Logs | `~/Library/Application Support/Murmur/murmur.log`, as on Windows. |

## Code

| File | Change |
|---|---|
| `src/platform/{mod,win,mac}.rs` (new) | `Rect` (Win32 `RECT` on Windows, the same four `i32` fields on macOS), `Window` (an `isize`: an HWND, or a pid on macOS), `key_down(vk)`, `foreground()`, `window_of(cc)`, `keep_on_top(w)`, `window_rect(w)`, `bring_to_front(w)`, `is_minimized(w)`, `open_path(p)`, `single_instance()`, `reduced_motion()`. The Windows half is the existing code moved verbatim. |
| `src/hotkey.rs` | `GetAsyncKeyState` becomes `platform::key_down`. The state machine is untouched. macOS maps Windows virtual-key codes to modifier bits (`CGEventSourceFlagsState`) or, for Esc and F-keys, `CGEventSourceKeyState`. |
| `src/inject/mac.rs` | Real paste, `undo_then_paste` (Cmd+Z then paste), `set_clipboard_text`, `foreground_hwnd` (frontmost pid). |
| `src/tray.rs` | Icon: Windows resources on Windows; on macOS a template icon drawn in code (no image files to ship). Balloons and the pill's right-click menu are Windows-only; macOS logs notices until phase 3. |
| `src/overlay.rs`, `src/mote.rs`, `src/canvas.rs`, `src/caret.rs` | Shared logic stays shared; the window code is `#[cfg(windows)]`. macOS gets placeholder `Overlay` (it also pumps NSApp events, as the Windows overlay pumps messages) and `Mote`, and caret/canvas functions that find and draw nothing. |
| `src/audio_out.rs`, `src/autostart.rs` | macOS placeholders. |
| `src/*_ui.rs`, `src/correction.rs`, `src/editor.rs`, `src/editor_kit.rs`, `src/setup_ui.rs` | `HWND`/`SetWindowPos`/`GetWindowRect`/mutex calls go through `platform`. Windows behaviour unchanged. |
| `src/config.rs` | macOS default `ptt_key = "RAlt"`; key labels use Mac names on macOS (Right Option, Left Command…). |
| `src/main.rs`, `src/spike.rs` | Every module builds on both systems; the phase 0 console spike is removed. |
| `scripts/macos/bundle.sh` (new) | Builds `target/release/Murmur.app` (Info.plist with `LSUIElement`, `NSMicrophoneUsageDescription`, bundle id `dev.jshowah.murmur`; binary and sherpa-onnx dylibs in `Contents/MacOS`) and signs it, with a stable identity when one is in the keychain so macOS keeps the Accessibility grant across rebuilds. |

## Risks

- **Ad-hoc signing.** macOS ties Accessibility to the code signature. Without a stable signing identity, every rebuild must be re-allowed in System Settings. `bundle.sh` uses an Apple Development or self-signed identity if one exists and says so when it falls back to ad-hoc.
- **egui windows on macOS** are compiled but untested until phase 2. A modal `run_native` that misbehaves would block the loop; if one does, its menu item is disabled on macOS until phase 2.
- **Windows** can't be built from the Mac (`ring` needs the Windows SDK headers). The release workflow's manual run builds and tests the branch on Windows; Jeff smoke-tests the pill on his PC before merging.
