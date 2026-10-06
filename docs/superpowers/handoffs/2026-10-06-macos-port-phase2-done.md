# Handoff: Murmur on macOS, phase 2 done, PR open (2026-10-06)

**State:** phase 2 is done and verified by Jeff on his Mac. PR [jshowah-dev/murmur#34](https://github.com/jshowah-dev/murmur/pull/34) (`feat/macos-phase0` → `main`) is open, mergeable, and Windows CI is green. The only thing left before merging is Jeff's hands-on Windows smoke test. Phase 3 hasn't started.

Supersedes `docs/superpowers/handoffs/2026-10-06-macos-port-phase2-wrap.md`. Its state, first actions and open items are out of date. Its "What a green build will not tell you" (`vad::GAIN`, the dropped pre-roll) and "Facts you'd otherwise re-derive" sections still stand. Older handoffs that still stand:
- `2026-10-06-macos-port-phase2.md`: the traps (never `finishLaunching`; winit closes every NSWindow when an egui loop exits) and the facts (only Jeff's real clicks test the menu bar; `screencapture` fails; ask before sending keystrokes; don't use `sample`).
- `2026-10-05-macos-port.md`: layout, build/run/test (`./scripts/macos/bundle.sh`), signing ("Murmur Dev" cert), restarting, decisions, and the phase 3–4 lists.

## State

| | |
|---|---|
| Branch | `feat/macos-phase0` at `a9801b9`, pushed. This handoff's commit sits on top, local only unless Jeff said to push it. |
| PR | [#34](https://github.com/jshowah-dev/murmur/pull/34), open, `MERGEABLE`/`CLEAN`, not merged. Auto-merge off. |
| Windows CI | Green on `a9801b9`, 0 warnings (run 37453794944). |
| This session's commits | `aa43ec0` dark theme holds in light mode · `a9801b9` every egui window in its own process, Dock icon for the editor |
| Running app | Jeff's `target/release/Murmur.app` at `a9801b9`, restarted 2026-10-06, still a login item. |
| Throwaway copy | Deleted from the scratchpad. Jeff should remove its second "Murmur" entry under Privacy & Security › Accessibility; not confirmed that he did. |
| gh | Active account `jshowah-dev`. Don't switch it. |

## First actions, in order

1. `git status -sb` → clean; `gh pr view 34 -R jshowah-dev/murmur --json state` → `OPEN` (unless Jeff merged it).
2. Ask Jeff for the Windows smoke result on his PC: pill, fix-last, editor, and a glance at the dark theme (Windows now gets a dark native title bar too). Merge only when he says so.
3. Then phase 3: `grep -rn "TODO(macos" src`, plus email formatting by bundle id (`context.rs`), notifications (`tray.rs`, a no-op today), the pill's right-click menu.

## Verified, and how

- **Jeff, live, on `a9801b9`:** History, the key picker and About each close with Esc and the menu opens afterwards. A key picked in the picker shows in the menu and dictates. The editor shows in the Dock and ⌘Tab, and clicking the Dock icon brings it back. ⌘Q with unsaved edits: Jeff answered "all work" to the list that included it, and didn't say whether the save prompt appeared.
- **Jeff, live:** editor ⌘S saves in Dictionary and Snippets. The dark theme reads correctly with macOS in Light mode.
- **Jeff, live, on an ad-hoc untrusted copy:** the setup Access step, the move-on after allowing, Ready, and the menu opens afterwards.
- **Log/ps:** Open at Login after a real restart (boot 05:21:23, Murmur started 05:22:00).
- **Unit tests:** every child request/reply round-trips through TOML (`child::round_trip`). Run with `cargo test --release`: the debug build trips egui's `TexturesDelta` `debug_assert` in 34 UI tests. That was already true before this session.
- **UNPROVEN:** a real second display; Windows hands-on since the theme change.

## What a green build will not tell you

- **No egui window may run in Murmur's main process on macOS.** Any of them locks the menu bar menu once it closes with Esc. Windows keeps them in-process (`#[cfg(windows)] window(...)`). A new window needs a flag, a `run_child`, and an arm in `app::main`'s `match`.
- **A child that dies reads as "no answer":** cancel for the picker and About, and `SetupOutcome::Quit` for setup, which makes Murmur exit. Check the log (`~/Library/Application Support/Murmur/murmur.log`, children log there too) if Murmur exits on first run.
- **`set_visuals` only styles the theme active at startup.** `correction_ui::dark_theme` pins `Theme::Dark` and asks for a dark native title bar. The title-bar part was ruled out as the cause of the menu lock (A/B tested with Jeff).
- The editor is `ActivationPolicy::Regular` (`platform::show_in_dock`), so it has the standard app menu and ⌘Q. `raise_titled` stays a no-op, so a second Dictionary… click still won't bring it forward. The Dock icon is the way back. Ruled by Jeff on 2026-10-06.

## Facts you'd otherwise re-derive

- The child protocol (`src/child.rs`) uses TOML wrapped in `{ v = ... }`, because TOML needs a top-level table and an absent `v` means no answer. The fix-last card keeps its own older text protocol (`correction_ui::card_process`); it wasn't migrated.
- To show the setup card: copy the bundle to the scratchpad, `codesign --force --deep -s -`, make `<scratch>/home/Library/Application Support/Murmur/` and symlink `models` into it from Jeff's real folder (otherwise it downloads the model), then `open -n --env HOME=<scratch>/home <copy>`. Don't dictate while it runs; both copies listen for Right Option.
- `git push` and `gh workflow run release.yml` each need Jeff's explicit go: auto mode blocked them as a production deploy until he said so.

## Open items

- Merge PR #34: Jeff's call, after his Windows smoke test.
- Root cause of the in-process menu lock: parked; worked around for every window.
- From earlier, still parked: the setup Access step's wording, and Input Monitoring not asked for (assumptions taken without Jeff); the pill's right-click menu and hover hint (phase 3).
- Phase 3, then phase 4 (Developer ID, notarization, DMG, a macOS job in `release.yml`, an updater).
