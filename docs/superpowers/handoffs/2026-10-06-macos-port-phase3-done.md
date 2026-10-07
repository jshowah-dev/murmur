# Handoff: Murmur on macOS, phase 3 done, phase 4 next (2026-10-06)

**State:** phases 0–3 are merged into `main`. Phase 3 is [jshowah-dev/murmur#36](https://github.com/jshowah-dev/murmur/pull/36) at `425e182`, smoke-tested by Jeff on both Mac and Windows. No `TODO(macos` remains in `src`. Phase 4 (distribution) hasn't started and is waiting on Jeff's answer about an Apple Developer account.

Supersedes `docs/superpowers/handoffs/2026-10-06-macos-port-phase2-done.md`. Its state, first actions and open items are out of date. Its "What a green build will not tell you" (no egui window in the main process; a dead child means no answer) and "Facts you'd otherwise re-derive" still stand. Earlier handoffs that still stand:
- `2026-10-06-macos-port-phase2.md`: the traps (never call `finishLaunching`; winit closes every NSWindow when an egui loop exits) and the facts (only Jeff's real clicks test the menu bar; `screencapture` fails; ask before sending keystrokes; don't use `sample`).
- `2026-10-06-macos-port-phase2-wrap.md`: `vad::GAIN` and the dropped pre-roll, and how to debug dictation with WAV dumps.
- `2026-10-05-macos-port.md`: layout, build/run/test (`./scripts/macos/bundle.sh`), signing (the self-signed "Murmur Dev" cert), restarting, and the phase 4 list.

## State

| | |
|---|---|
| `main` | `425e182` (merge of #36). Jeff's checkout is on `main`, clean. |
| Merged PRs | #34 phases 0–2 → `3e8e8cf`; #36 phase 3 → `425e182` |
| Phase 3 commits | `3abb017` notifications · `d94c511` email formatting · `6b4a70c` the pill's right-click menu |
| Windows CI | Green on `6b4a70c`, 0 warnings (run 37570710464) |
| Branches | `feat/macos-phase0` and `feat/macos-phase3` deleted, locally and on GitHub, at Jeff's go. |
| Running app | Jeff's `target/release/Murmur.app` at `6b4a70c`'s code (same as `main`), a login item, notifications allowed |
| Jeff's config | `email_apps` was upgraded to the six-entry list; `config.toml.bak` holds the previous version |
| This handoff | Committed straight to `main`, at Jeff's go. |

## First actions, in order

1. `git status -sb` → `main`, matching `origin/main` at `425e182` (or later, if Jeff merged more).
2. Ask Jeff the open question: **does he have an Apple Developer account ($99/year)?** Developer ID signing and notarization need it. Without them, macOS's Gatekeeper blocks a downloaded Murmur.
3. With an account: phase 4 in full. Without one: start with the parts that don't need it. Those are a DMG, a macOS job in `release.yml` (ad-hoc signed and clearly unsigned), and the README rewritten for both systems.

## Verified, and how

- **Jeff, live, on macOS:**
  - A notification for a broken `snippets.toml` appeared, and clicking it in Notification Center opened the file in TextEdit.
  - Email formatting in Mail, and in Gmail in a browser. The log showed `profile: email` at 04:00:26 and 04:02:21.
  - The pill's menu: right-click opens it, its items work, the focus stays in the app you were in, clicks beside the pill pass through, and the menu bar menu still works afterwards.
- **Jeff, Windows smoke** for #34 and #36: passed. He reported pass/fail only, with no per-item detail.
- **Unit tests** (`cargo test --release`): bundle-id matching, the `email_apps` rewrite and its parse round-trip, and the child round-trips.
- **UNPROVEN:**
  - Clicking the "update available" notification, which needs a newer release to exist. It shares its click path with the tested notice.
  - The TextEdit plain-text control dictation: the log has no line for it.
  - A real second display.

## What a green build will not tell you

- **Notifications need the bundle.** `platform::notify` and `init_notifications` return early without a bundle id, because `UNUserNotificationCenter` throws outside an app bundle (in tests and `cargo run`). Notifications share one request id, `"murmur"`, so a new one replaces the last, and `BALLOON_CLICKED` refers to the last one, as on Windows.
- **Murmur only reports a broken file once,** until the file changes again. A second dictation doesn't post again, and that's not a bug.
- **The pill takes clicks only while the cursor is inside its rect** (`Panel::set_clickable`, run every tick from `Overlay::animate`). `platform::pump` swallows a `RightMouseDown` on the pill's window number and raises `take_pill_right_click`. Don't make the panel clickable all the time: it would block clicks for the app underneath.
- **The `email_apps` upgrade rewrites `config.toml` on both systems,** but only when the list is exactly the old Windows default `["OUTLOOK.EXE", "olk.exe", "thunderbird.exe"]`.
- **The browser window title is read through Accessibility** in `murmur_lib::context` (lib crate, its own small `ax` module; it can't reach the bin crate's `caret` helpers). It's never logged.

## Facts you'd otherwise re-derive

- **git-guard hook:** it blocked `gh pr merge 36` even after Jeff said "merge it" in chat ("Jeff did not approve this PR merge"), and Jeff merged it himself. `gh pr merge 34` went through earlier the same day. Don't retry a blocked merge; ask Jeff. `git push` and `gh workflow run release.yml` each need his explicit go: auto mode blocks them as a production deploy until then.
- **To trigger a notice for a test:** back up `~/Library/Application Support/Murmur/snippets.toml` to the scratchpad, append a non-TOML line, and Jeff dictates. Restore the file afterwards, and have Jeff close TextEdit without saving.
- **Mac browsers:** Safari, Chrome, Edge, Firefox, Brave, Opera, Vivaldi and Arc are in `context::BROWSERS`, by lower-case bundle id.

## Open items

- **Apple Developer account:** Jeff's answer is pending, and it gates phase 4's signing and notarization.
- **Phase 4:** Developer ID, hardened runtime with the microphone entitlement, notarization, a DMG, a macOS job in `release.yml`, a Mac updater (today "update available" opens the releases page), and the README for both systems. Neither the README nor `release.yml` mentions macOS today, and the latest release (v0.4.21) has an empty description.
- **Parked:** the pill's hover hint; a real second display; the root cause of the in-process menu lock; the setup Access step's wording and not asking for Input Monitoring (Jeff approved the step as it is in his test on 2026-10-06).
