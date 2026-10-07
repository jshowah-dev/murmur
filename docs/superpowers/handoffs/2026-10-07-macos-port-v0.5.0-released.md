# Handoff: Murmur on macOS, v0.5.0 released, signing next (2026-10-07)

**State:** Murmur 0.5.0 is published with a Mac DMG (unsigned preview) alongside the Windows installer and zip. The parts of phase 4 that don't need an Apple Developer account are done and merged. What's left is Developer ID signing, notarization and a Mac auto-updater, all waiting on Jeff's answer about an Apple Developer account.

Supersedes `docs/superpowers/handoffs/2026-10-06-macos-port-phase3-done.md`. Its state, first actions and open items are out of date. Its "What a green build will not tell you" (notifications need the bundle; a broken file is reported once; the pill is clickable only under the cursor; the `email_apps` upgrade; the browser title read in the lib crate) and "Facts you'd otherwise re-derive" still stand. The chain of earlier handoffs it lists still stands as described there.

## State

| | |
|---|---|
| `main` | `9d4c615` (merge of #37). Jeff's checkout is on `main`, clean. |
| Merged PRs | #34 phases 0–2 → `3e8e8cf`; #36 phase 3 → `425e182`; #37 DMG, CI, 0.5.0, README → `9d4c615` |
| Release | [v0.5.0](https://github.com/jshowah-dev/murmur/releases/tag/v0.5.0), published and marked latest by Jeff on 2026-10-07. Lightweight tag on `9d4c615`. Six files: `murmur-v0.5.0-macos-arm64.dmg`, `murmur-v0.5.0-setup.exe` and `murmur-v0.5.0-windows-x64.zip`, each with its `.sha256`. The notes were generated from PRs #34, #36 and #37, with nothing added by hand. |
| CI | `release.yml` has three jobs: `windows`, `macos` (`macos-latest`: build, test, `bundle.sh`, `dmg.sh`) and `release` (tag only; drafts one release with `--generate-notes`). The tag run was 37596530558, all green. |
| gh | The `jshowah-dev` token now has the `workflow` scope (added 2026-10-07), so pushes that touch `.github/workflows/` work. |
| Jeff's Mac | Running `target/release/Murmur.app`, built from phase 3 code and reporting itself as 0.4.21; it's the login item. `/Applications/Murmur.app` v0.5.0 (quarantined, ad hoc) is left from the Gatekeeper test; Jeff can keep it or bin it. |
| Branches | No feature branches left. |

## First actions, in order

1. `git status -sb` → `main`, matching `origin/main` at `9d4c615` or later.
2. Ask Jeff the open question: **does he have an Apple Developer account ($99/year)?** If he does, Developer ID signing and notarization come next. If not, there's no more Mac distribution work; ask what he wants instead (the parked items below, or something else).
3. With an account, roughly:
   - make a Developer ID Application certificate;
   - in `bundle.sh`, sign with `--options runtime` (the hardened runtime) and an entitlements file containing `com.apple.security.device.audio-input`;
   - notarize the DMG with `xcrun notarytool submit --wait`, then `xcrun stapler staple`;
   - store the certificate and the notary credentials as GitHub secrets for the `macos` job.

   Verify each of these against Apple's documentation before building on it.

## Verified, and how

- **Jeff, live, on the published 0.5.0 DMG,** downloaded with `gh` and marked as downloaded by Safari: Finder showed "Murmur Not Opened… could be malware", then **Open Anyway** in Privacy & Security, then the app ran. The log shows `murmur 0.5.0 starting` at 09:20:11, then "another instance is running; exiting", which was expected. So the README's install steps 1–2 are accurate.
- **CI:** both build jobs green on the branch run (37595263230) and the tag run (37596530558). The DMG checksum matched after download.
- **Local:** `cargo test --release --locked` on the branch, before the merge: 187 + 206 + 1 passed.
- **UNPROVEN:** a clean install on a Mac that has never had Murmur (setup's Accessibility step, the microphone prompt and the model download from the downloaded app). The update offer on a Mac (an installed 0.4.21 offering 0.5.0, whose click opens the releases page). Updating to a later version and re-allowing Accessibility.

## What a green build will not tell you

- **The Mac build is signed ad hoc,** so macOS treats every version as a new app: each update needs Open Anyway again, and Accessibility has to be removed with − and allowed again (the README says this). Only Developer ID signing fixes it.
- **`release.yml`'s `release` job runs only on a `v*` tag,** after both build jobs. Each build job checks that the tag matches `Cargo.toml`. A manual run on a branch uploads the files as run artifacts and creates no release.
- **Publishing a release makes installed Windows copies offer the update.** The tag only creates a draft. Per ship-pr, the tag and the publish are separate yes/no questions.

## Facts you'd otherwise re-derive

- **To test Gatekeeper as a real download would hit it:** `gh release download` doesn't set quarantine, so after downloading, run `xattr -w com.apple.quarantine "0081;$(printf %x $(date +%s));Safari;$(uuidgen)" <dmg>`. `spctl -a -vv` then says "rejected". Finder may not show the copied app in Applications straight away; `open /Applications/Murmur.app` works.
- **On a Mac, the murmur suite is `cargo test --release --locked`.** This is now in `~/.claude/skills/ship-pr/references/murmur.md`.
- **`gh auth refresh` needs an Enter keypress before it polls.** From this harness, run `echo | gh auth refresh -h github.com -s <scope>` in the background, and Jeff approves the code at github.com/login/device. Neither the Terminal panel's run nor the `!` prompt worked.

## Open items

- **Apple Developer account:** Jeff's answer is pending; it gates signing, notarization and the auto-updater.
- **v0.5.0 release notes:** they were generated automatically and don't call out the Mac preview's caveats. Jeff could edit them on GitHub; not raised as a task.
- **`/Applications/Murmur.app` from the test:** Jeff's call (keep it, or move to it from the dev build).
- **Parked:** the pill's hover hint; a real second display; the root cause of the in-process menu lock; the setup Access step's wording and Input Monitoring.
