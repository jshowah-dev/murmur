# Handoff: Murmur on macOS (2026-10-05)

## Where things stand

Murmur dictates on Jeff's MacBook Air (Apple Silicon, macOS 26.6): hold **Right Option**, speak, let go, and the text is pasted where the cursor is. It runs as a menu bar app (`Murmur.app`, no Dock icon) with the pill above the Dock. Windows is unchanged in behaviour and verified by CI after every step.

- **Branch:** `feat/macos-phase0` (pushed; no PR yet). Commits on top of `main`:
  - `83480c5` phase 0: builds and dictates on macOS (console spike, since removed)
  - `602a08a` phase 1: push-to-talk, paste and menu bar on macOS
  - `7ad1e95` fix: a Windows-only parse error found by CI
  - `d98fefa` the pill on macOS
  - `5669d80` the pill's meter gain on macOS
  - `5d183c8` the flying mote on macOS, with the Accessibility caret
  - the web-view caret fix (Chromium answers with the whole line)
- **Specs:** `docs/superpowers/specs/2026-10-05-murmur-macos-phase1-design.md` (phase 1 decisions, spike findings, file-by-file changes).
- **Tests on the Mac:** 183 library, 188 app, speech integration; all pass. Windows CI: phase 1 passed; the pill commit's run was in progress at handoff ([runs](https://github.com/jshowah-dev/murmur/actions/workflows/release.yml)).

## Decisions (Jeff)

| Question | Decision |
|---|---|
| Audience | Jeff's own Mac for now; open-source the Mac build later like the Windows one. |
| Code shape | One codebase, one app loop (`src/app.rs`); OS calls behind `src/platform/{win,mac}.rs`. |
| Default key on macOS | Right Option (`ptt_key = "RAlt"`). |
| Order | Pill moved ahead of the phase 2 windows. |

## How the port is laid out

- **`src/platform/`**: `mod.rs` (the `Window` handle, `window_of`), `win.rs` (the original Win32 code moved verbatim), `mac.rs` (keys, focus, single-instance lock files, `init_app`/`pump` for AppKit, `accessibility_trusted`, `frontmost_pid`, screen geometry, and `Panel`, the click-through NSPanel the pill draws into).
- **Whole-module splits** (`mod.rs` picks one): `src/inject/{win,mac}.rs`, `src/audio_out/{win,mac}.rs`, `src/autostart/{win,mac}.rs`.
- **Shared logic with gated windows**: `overlay.rs` (shared `Pill` core + `render_scaled`; `overlay/mac.rs` is the Mac pill), `mote.rs` (`mote/mac.rs` is a do-nothing stand-in), `caret.rs`, `canvas.rs` (`Canvas::scaled` for Retina; GDI text is Windows-only).
- **egui windows** (history, key picker, About, fix-last card, first-run setup, editor) call `platform` instead of Win32 and compile on macOS, but are **untested** there.
- **Coordinates:** shared code uses top-left screen coordinates. On Windows that's physical pixels; on macOS it's points from the main display's top-left (`platform::mac` flips to AppKit's bottom-left).
- **Keys:** config.toml keeps Windows virtual-key names on both systems. On macOS, modifier keys are read from the device-dependent bits of `CGEventSourceFlagsState` (no permission needed); Esc and F-keys need Input Monitoring, which Murmur doesn't ask for yet. Labels say Option/Command on a Mac; `ROption`, `RCommand`, `RCmd` (and Left) are accepted spellings.

## Build, run, test on the Mac

```bash
source "$HOME/.cargo/env"
cargo test --release
./scripts/macos/bundle.sh
open target/release/Murmur.app
```

- Logs: `~/Library/Application Support/Murmur/murmur.log`; settings, dictionary and snippets in the same folder; the model in `…/Murmur/models`.
- **Permissions:** Microphone (prompted on first capture) and Accessibility (prompted at start-up; without it the text is left on the clipboard and the log says so).
- **Signing:** macOS ties Accessibility to the signature. Jeff's login keychain has a self-signed **"Murmur Dev"** code-signing certificate (made with `/System/Library/CoreServices/Certificate Assistant.app`); `bundle.sh` picks it up (or set `MURMUR_SIGN_IDENTITY`), and rebuilds keep Accessibility. If the app ever falls back to ad hoc signing, remove Murmur under Privacy & Security › Accessibility and add it again; toggling isn't enough.
- **Restarting:** `open` on a running Murmur only brings it forward. Quit it first (menu bar › Quit, or `pkill -f "Murmur.app/Contents/MacOS/murmur"`).
- **Windows can't be compiled from the Mac** (`ring` needs the Windows SDK headers). Verify with the release workflow's manual run, which builds, tests and uploads artifacts without making a release: `gh workflow run release.yml --ref feat/macos-phase0`.
- **Pushing:** git on the Mac isn't wired to the `gh` login; pushes so far used `git -c credential.helper= -c 'credential.helper=!gh auth git-credential' push`. `gh auth setup-git` would make it permanent (Jeff's call).

## Next steps

**The pill:** done and confirmed by Jeff. The MacBook mic reads about 7× quieter than the Windows mics, so the meter gain is 40 on macOS and 6 on Windows (`METER_GAIN` in `app.rs`; each release logs `loudest voice rms`). An adaptive meter would suit any mic. Not done on macOS: right-click menu and hover hint (the panel ignores the mouse).

**Phase 2: the egui windows on macOS.** Open each from the menu bar and fix what breaks:
- `platform/mac.rs` stubs: `keep_on_top`, `move_to`, `window_rect`, `raise_titled`; `caret::work_area` returns a fixed rectangle.
- Risk: winit sets the activation policy to Regular when its event loop is first created, which may add a Dock icon. eframe's `NativeOptions::event_loop_builder` can set `ActivationPolicy::Accessory`.
- Fix-last submits with Ctrl+Enter (`Modifiers::CTRL`); a Mac wants ⌘Enter (`Modifiers::COMMAND`, which is still Ctrl on Windows).
- First-run setup should walk through the Accessibility (and optionally Input Monitoring) permissions.
- The UI font is SF Pro (`/System/Library/Fonts/SFNS.ttf`), unverified in egui.

**The mote:** done and confirmed in Quick Notes, Mail and the Claude app. The `Mote` is shared; on macOS it moves a sprite layer across a still full-screen `platform::Stage` (moving a window per frame smeared). Mac tuning from Jeff: 450 ms flight, dot at 0.8 of its point size. It lands where the words will start (the caret at release), as on Windows; Jeff chose to keep that. `caret.rs` asks Accessibility for the insertion point: a character range in native fields, text markers in web views; Chromium answers a collapsed range with the whole line, so the caret comes from the neighbouring character. Electron/Chromium apps are woken once per pid with `AXManualAccessibility`, so their first dictation may miss. Jeff hasn't yet said whether the motion now looks as clean as Windows.

**Phase 3: the rest of the parity work.**
- Email formatting via bundle id + window title (`context.rs`), speaker mute via CoreAudio (`audio_out/mac.rs`), Open at Login via SMAppService (`autostart/mac.rs`), notifications (`tray.rs`), the pill's right-click menu.
- `grep -rn "TODO(macos" src` lists every placeholder.

**Phase 4: distribution.** Apple Developer ID, hardened runtime with the audio-input entitlement, notarization, a DMG, a macOS job in `release.yml`, a Mac updater (today "update available" opens the releases page), and a Mac section in the README.

**Before merging to `main`:** Jeff smoke-tests the Windows build on his PC (the pill, fix-last, the editor), since CI can't see the pill. The workflow run's artifacts include the zip and installer.
