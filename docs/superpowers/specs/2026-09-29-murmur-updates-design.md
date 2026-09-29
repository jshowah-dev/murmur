# Murmur updates: app update alert, click-to-update, model-upgrade mechanism

Date: 2026-09-29. Status: design approved in chat, pending review of this written spec.

## Goal

A running Murmur notices a newer GitHub release within a day, says so once, and updates itself from one tray click: download, verify, silent install, relaunch. Separately, when a future release pins a new speech model, installs on the old default model are offered the new one instead of keeping the old one forever. No model change ships with this work.

## Decisions (Jeff, 2026-09-29)

| Question | Decision |
|---|---|
| Check timing | **Startup + daily.** First check ~60 s after launch, then every 24 h while running. About keeps its own check while it's open (earlier ruling stands). |
| Alert + click target | **Balloon + menu item.** One balloon per new version; the tray menu gains "Update to vX…" above About. tray-icon 0.24 doesn't report balloon clicks (`NIN_BALLOONUSERCLICK`), so the balloon itself isn't clickable. |
| Update flow | **Silent, installer relaunches.** Download the setup exe, verify it against the release's `.sha256`, start it with `/VERYSILENT`, quit Murmur. `murmur.iss` relaunches Murmur only when `/RELAUNCH` is passed. No helper process. |
| Model scope | **Mechanism now, model later.** Detect an install on a previous default model and offer the current pin. Which model to move to (e.g. Parakeet v3) stays a separate decision; `PREVIOUS_DEFAULTS` ships empty. |
| Model download | **Ask first.** Balloon + tray item "Download new speech model (480 MB)…"; download in the background; the new model is used from the next start. |

Out of scope: choosing or shipping a new model, code signing, a settings window, the version bump and release, Dependabot.

## Components

| Unit | File | Job |
|---|---|---|
| Release check | `src/update.rs` (new) | `Release { tag, setup_url, setup_size, sha_url }`. `parse_release(json) -> Option<Release>` pulls `tag_name`, the setup asset's `size` and the `browser_download_url`s of `murmur-v<tag>-setup.exe` and `…-setup.exe.sha256` by string search (no serde_json, like About today). `check() -> Result<Option<Release>>` does the GET (8 s timeout) and returns a release only if `is_newer(tag, VERSION)`. `latest_tag` and `is_newer` move here from `about_ui.rs`; About calls `update::check`. |
| Download + verify | `src/update.rs` | `download(&Release) -> Result<PathBuf>`: fetches the `.sha256` text, parses the first token as the hash (`parse_sha256_file`), then downloads the setup exe to `%TEMP%\Murmur\murmur-v<tag>-setup.exe` with `model_fetch::Fetcher` against an `Asset` built from `setup_url`, the parsed hash and `setup_size`. A mismatch deletes the file and returns `FetchError::ChecksumMismatch`. |
| Install | `src/update.rs` | `install(path) -> Result<()>` spawns the exe detached with `/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /FORCECLOSEAPPLICATIONS /RELAUNCH`. On `Ok`, the main loop quits Murmur the same way Quit does. |
| Installed-copy gate | `src/update.rs` | `is_installed_copy(exe: &Path, localappdata: &Path) -> bool`: true when `exe` is `<localappdata>\Programs\Murmur\murmur.exe` (case-insensitive). Otherwise the menu item reads "Get vX…" and opens the release page instead of downloading. |
| Alert-once state | `src/update.rs` | File `config_dir()\update-alerted` holding the last tag alerted. `should_alert(tag, stored) -> bool`. The balloon fires only for a tag not yet alerted; the menu item shows regardless. |
| Background checker | `src/update.rs` + `src/main.rs` | A thread: sleep 60 s, `check()`, sleep 24 h, repeat. It sends `UpdateMsg::Available(Release)` on a crossbeam channel. Errors are logged and nothing else. The main loop drains the channel like `msg_rx`. The download also runs on a thread and reports `UpdateMsg::Downloaded(PathBuf)` or `UpdateMsg::Failed(String)`. |
| Tray | `src/tray.rs` | Two hidden items above About: `update` ("Update to vX…") and `model` ("Download new speech model (480 MB)…"), new `TrayEvent::Update` and `TrayEvent::DownloadModel`. `set_update(Option<&str>)`, `set_update_busy(label)`, `set_model_offer(Option<label>)` toggle text, enabled and `set_visible`. |
| Installer relaunch | `installer/murmur.iss` | New `[Run]` entry: `Filename: "{app}\murmur.exe"; Flags: nowait; Check: RelaunchRequested`, with `RelaunchRequested` true when any `ParamStr` equals `/RELAUNCH` (case-insensitive). The existing postinstall entry stays for interactive installs. |
| Model resolution | `src/config.rs` | `PREVIOUS_DEFAULTS: &[&str] = &[]`. `is_previous_default(&self) -> bool`. `resolve_model(cfg: &mut Config, installed: impl Fn(&Path) -> bool) -> ModelState` where `ModelState` is `Current`, `Switched { old: PathBuf }` (old default in config, current default installed: `cfg.model_dir` is set to the current default **in memory**) or `UpgradeAvailable` (old default in config, current default not installed: keep the old one). `config.toml` is never rewritten: it carries comments and hand edits. |
| Model startup | `src/main.rs` | `resolve_model` runs before `model_missing` and before the pipeline is spawned, so `cfg.model_dir_path()` is right everywhere (pipeline, VAD). `Switched` deletes the old folder (not yet loaded, so not locked). `UpgradeAvailable` shows the balloon "A new speech model is available (480 MB). Download from the tray menu." and the menu item. `default_dir` also counts a previous default, so a deleted old default still gets the first-run download. |
| Model download | `src/main.rs` + `src/model_fetch.rs` | On `DownloadModel`: a thread runs `Fetcher::standard().download(parakeet())` then `extract` into the models folder, reporting percent as `UpdateMsg::ModelProgress(u8)` (menu text "Downloading speech model… 42%"), then `ModelReady` or `ModelFailed(String)`. Ready: balloon "New speech model ready. It's used from the next start." |

## Data flow

```
startup ─ resolve_model ─┬─ Current ──────────────────────────────── pipeline loads cfg.model_dir
                         ├─ Switched{old} ─ delete old ────────────── pipeline loads new default
                         └─ UpgradeAvailable ─ balloon + menu ─ click ─ download+extract ─ "used from next start"

checker thread ─ 60 s ─ check() ─ Available(r) ─ main loop ─ balloon (once per tag) + "Update to vX…"
click ─ installed copy? ─ no ─ open release page
                        └ yes ─ download thread ─ verify sha256 ─ Downloaded(p) ─ install(p) ─ quit
installer /VERYSILENT ─ closes Murmur if still running ─ installs ─ [Run] RelaunchRequested ─ murmur.exe
```

## Errors

| Failure | Behaviour |
|---|---|
| Check fails (offline, rate limit, unparseable JSON, no setup asset) | Logged; nothing shown; next 24 h tick retries. |
| Download fails or SHA mismatch | Partial file deleted; balloon with the reason; the menu item returns to "Update to vX…". |
| Installer can't be spawned | Balloon; Murmur keeps running. |
| Installer fails after Murmur quit | Inno rolls back; the old version stays installed but **Murmur is not relaunched** (`[Run]` fires only on success). Accepted: rare, and Inno's log in `%TEMP%` has the cause. |
| Model download or unpack fails | Balloon with the reason; menu item returns; the old model keeps working. `is_installed` only sees a finished unpack. |

## Testing

Unit tests, written failing first:

- `parse_release`: tag, setup size and both asset URLs from a real v0.4.4 JSON fixture; `None` when the setup asset is missing.
- `parse_sha256_file`: `"<hex>  murmur-v0.4.4-setup.exe"` → hex; rejects non-hex or short input.
- `is_installed_copy`: install path matches (case-insensitive); `target\release` and a zip folder don't.
- `should_alert`: new tag yes; same tag no; no stored file yes.
- `resolve_model` with a test list of previous defaults and a fake `installed`: all three `ModelState`s; a custom `model_dir` is always `Current`.
- `RelaunchRequested`: a test that reads `installer/murmur.iss` and asserts the `[Run]` entry has the `Check:` and no `skipifsilent`.

Debug-only (`cfg(debug_assertions)`) override for the smoke:

- `MURMUR_UPDATE_URL` replaces the GitHub API URL.

End-to-end smoke before release (the only proof of the full relaunch):

1. Build a local setup from this branch with ISCC at a higher version (e.g. `/DAppVersion=0.4.99`), write its `.sha256`, and serve both plus a release JSON from `python -m http.server`.
2. Copy a debug build over the installed `murmur.exe`, start it with `MURMUR_UPDATE_URL` pointing at the local JSON.
3. Expect: balloon within ~60 s; "Update to v0.4.99…" in the menu; click → download, verify, silent install, Murmur relaunches; the installed `murmur.exe` reports 0.4.99; autostart kept; dictation works.
4. Reinstall the published v0.4.4 setup afterwards to leave the machine clean.

The model mechanism has unit tests only until a release pins a new model.
