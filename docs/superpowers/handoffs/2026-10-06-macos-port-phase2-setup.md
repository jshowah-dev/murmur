# Handoff: Murmur on macOS, phase 2: editor, second display, setup (2026-10-06)

**State:** the three items a peer session relayed from Jeff are done and committed locally, not pushed: the editor's ⌘S, `caret::work_area` per screen, and the setup card's Accessibility step. None has been smoke-tested by Jeff. Windows CI hasn't run on them.

Supersedes `docs/superpowers/handoffs/2026-10-06-macos-port-phase2.md` for state and next steps. Its traps and "facts you'd otherwise re-derive" still stand and aren't repeated here. Read them, especially: never call `finishLaunching`, the fix-last card stays out of process, and only Jeff's real clicks can test the menu bar.

## State

| | |
|---|---|
| Branch | `feat/macos-phase0`, local `62610bd`, 4 commits ahead of `origin` (`29e62b1`). Not pushed, no PR: the peer's instruction was that both are Jeff's call. |
| New commits | `b4d0717` handoff · `6316ec9` editor ⌘S tests · `61d9210` work area per screen · `62610bd` setup's Access step |
| Windows CI | Last green on `29e62b1`. The new commits touch shared files (`setup_ui.rs`, `app.rs`, `caret.rs`, both editor panels), so run `gh workflow run release.yml --ref feat/macos-phase0` after pushing. |
| Running app | Jeff's Murmur is still the process started at 04:21 (from `27f73d6`'s bundle). `target/release/Murmur.app` has since been rebuilt at `62610bd`, so a restart picks that up. It's trusted, so the new Access step won't show for him. |
| Scratch | Ad-hoc copy and throwaway homes live in this session's scratchpad only; nothing is installed or registered. |

## First actions, in order

1. Ask Jeff whether to push. If yes, push (`git -c credential.helper= -c 'credential.helper=!gh auth git-credential' push`; the `gh` account must be `jshowah-dev`), then run Windows CI. Expected: success, no warnings.
2. Ask Jeff to smoke-test, after restarting Murmur on the new bundle:
   - Dictionary… / Snippets…: edit, ⌘S saves (the title or Save state clears).
   - Fix last still opens at the caret (the work area change runs there).
3. The Access step needs an untrusted Murmur to see. Don't revoke Jeff's permission to test it. Use an ad-hoc-signed copy launched with `open -n --env HOME=<scratch home> <copy>/Murmur.app`. Launched from a shell directly, macOS judges the shell's parent app (Claude, trusted) instead, and the card never shows. Link the real `models` folder into the scratch home so nothing downloads. **Kill the copy after looking**: past setup it runs a full Murmur with its own hotkey.

## Verified, and how

| Item | Evidence | Level |
|---|---|---|
| Editor ⌘S | Unit tests send the modifiers egui-winit reports (⌘ = `mac_cmd + command`; Ctrl alone doesn't save on a Mac); they fail with the old `Modifiers::CTRL`. The editor process opens on macOS (760×592 window, in a scratch home). | Unit-tested and launch-checked. **⌘S keypress in the real app: UNPROVEN.** |
| Work area per screen | `nearest_screen` unit-tested on a two-display layout (edges, above, off-screen); `flipped` unit-tested. A card process at a bottom-right caret was pulled inside the usable area (1170..1710 × 900..978, clear of the Dock). | Unit-tested and single-display smoke. **A real second display: UNPROVEN** (only the built-in one was attached). |
| Setup Access step | Unit tests: shows its text and buttons, waits while not allowed, closes once allowed (nothing to download), Esc goes on without it. An untrusted ad-hoc copy opened the setup card (480×151, centred) and logged `not allowed to paste yet`. | Unit-tested and window-appears smoke. **Its look, Open Settings, the move-on after allowing, and the download after it: UNPROVEN.** |

## What a green build will not tell you

- **The setup card runs in Murmur's own process,** before the tray exists. Fix last in-process left the menu bar icon unable to open its menu; History, the key picker and About didn't. Whether setup does is **unknown**. If Jeff sees a dead menu after first-run setup, move setup out of process the way `correction_ui::card_process` does.
- Open Settings calls `AXIsProcessTrustedWithOptions` with the prompt. That's what adds Murmur to the Accessibility list, so don't swap it for just opening the Settings URL.
- `Plan::Skip`/`ReadyOnly` plus `ask_access` means a window with only the Access step, and `installed` starts true there (the model is present). A missing custom `model_dir` still ends in the tray notice.

## Decisions taken without Jeff (assumptions, revisit freely)

- The Access step replaces the bare system prompt at startup. It shows on every launch while untrusted, as the prompt did.
- Copy on the card: "Murmur pastes what you say by pressing ⌘V for you. macOS lets it once you allow it." / "Allow Murmur under Privacy & Security › Accessibility" / buttons "Open Settings", "Not now" / "Until then, your words are left on the clipboard."
- Input Monitoring isn't asked for: only an Esc, F-key or Caps Lock push-to-talk key needs it, and the default is Right Option.

## Open items

- Parked by the peer's instruction: `raise_titled` (a design call: looking up a window by title needs Screen Recording), the root cause of the menu lock, the pill's right-click menu and hover hint.
- Push and PR timing: Jeff's call.
- Before merging to `main`, Jeff smoke-tests the Windows build on his PC (pill, fix-last, editor).
