# Murmur installer: Inno Setup, uninstall, start with Windows

Date: 2026-09-28. Status: design approved in chat, pending review of this written spec.

## Goal

Someone who downloads Murmur gets a normal Windows install: a Start-menu entry, an entry in Add/Remove Programs with a working uninstaller, and an optional start with Windows. The portable zip keeps shipping next to the installer. Start with Windows can also be switched from the tray menu, so the choice can change without reinstalling and zip users get it too.

## Decisions (Jeff, 2026-09-28)

| Question | Decision |
|---|---|
| Code signing | **Parked.** The installer ships unsigned. There's no signing code; this spec only records where signing would go (see CI). SignPath Foundation (free, but the publisher is shown as "SignPath Foundation" and it requires verifiable reputation) and Azure Artifact Signing ($9.99/month, publisher Jeff Showah, individuals in the US/Canada only) were weighed and deferred. |
| Installer technology | **Inno Setup 6.** It does per-user installs without an admin prompt, uninstall prompts in `[Code]`, and closes the app before replacing files. MSI's only real advantage is Group Policy deployment, which Murmur doesn't need. |
| Install scope | Per-user, `%LOCALAPPDATA%\Programs\Murmur`, no admin prompt. |
| Zip | Keeps shipping, unchanged. |
| Start with Windows | An installer checkbox **plus** a tray toggle. Both write the same `HKCU` Run value. |
| Uninstall data | **Ask about each**: the model (about 660 MB) and the settings/dictionary/snippets. Both prompts default to No. |
| Release | Ships as **v0.2.0**. Tagging and publishing are Jeff's call. |

Out of scope: code signing, auto-update, MSI, a per-machine install, a licence page, localisation, removing a model at a custom `model_dir`.

## Installer behaviour (`installer/murmur.iss`)

### Install

- Output: `murmur-vX.Y.Z-setup.exe`. `PrivilegesRequired=lowest`, `DefaultDirName={localappdata}\Programs\Murmur`, and a fixed `AppId` GUID so upgrades install in place.
- Files: `murmur.exe`, `onnxruntime.dll`, `onnxruntime_providers_shared.dll`, `sherpa-onnx-c-api.dll`, `sherpa-onnx-cxx-api.dll` from `target\release\`, plus `README.md` and `LICENSE`. This is the same set as the zip.
- The setup and uninstall icons come from `assets\murmur.ico`.
- Shortcuts: a Start-menu "Murmur" shortcut, and an optional desktop icon task (**unchecked** by default).
- A "Start Murmur with Windows" task, **checked** by default on a fresh install. When it's checked, the installer writes `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, value `Murmur` = `"{app}\murmur.exe"`, and deletes `HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run`, value `Murmur`.
- Finish page: "Launch Murmur", checked. The first run (model download, setup window) works exactly as it does today.
- Add/Remove Programs: the name "Murmur", the version, the publisher "Jeff Showah", the icon, and `AppPublisherURL` = `https://github.com/jshowah-dev/murmur`.
- No licence page.

### Upgrade

- The same `AppId` installs into the existing folder and keeps a single Add/Remove Programs entry.
- If Murmur is running, it's closed first: `CloseApplications=force`, `RestartApplications=no`. The "Launch Murmur" finish option restarts it.
- The start-with-Windows task's initial state comes from the **current** Run value: checked if `Run\Murmur` exists, unchecked if not. The installer doesn't reset your choice. Unticking it on an upgrade deletes the Run value.
- `%APPDATA%\Murmur` and `%LOCALAPPDATA%\Murmur\models` are never touched.

### Uninstall

1. Closes Murmur if it's running (the same Restart Manager mechanism).
2. Removes the program files, the shortcuts, `Run\Murmur` and `StartupApproved\Run\Murmur`.
3. Prompt: "Delete the downloaded speech model (about 660 MB)? You'd need to download it again if you reinstall." Yes/No, default **No**. Yes deletes `%LOCALAPPDATA%\Murmur\models` and then `%LOCALAPPDATA%\Murmur` if it's empty.
4. Prompt: "Delete your settings, dictionary and snippets?" Yes/No, default **No**. Yes deletes `%APPDATA%\Murmur`.
5. A silent uninstall (`/SILENT`, `/VERYSILENT`) skips both prompts and keeps all data.

If `model_dir` has been pointed elsewhere in `config.toml`, only the default location is removed. The uninstaller doesn't parse the config.

## App change: tray toggle

### `src/autostart.rs` (new; depends only on `windows`)

- `pub fn is_enabled() -> bool` is true when both:
  - `HKCU\...\Run\Murmur` exists and its value names **this** executable (`std::env::current_exe()`). The comparison ignores case and surrounding quotes, because a zip copy and an installed copy can coexist.
  - `HKCU\...\Explorer\StartupApproved\Run\Murmur` is absent, or its first byte is even. Task Manager's "Startup apps" writes an odd first byte (for example `0x03`) when it disables an entry, and an even one (`0x02`) when it enables one.
- `pub fn set(enabled: bool) -> Result<()>`:
  - On: writes `"<current_exe>"` (quoted) to `Run\Murmur`, then deletes `StartupApproved\Run\Murmur` if it's present.
  - Off: deletes `Run\Murmur`. A missing value isn't an error.
- Pure, unit-tested helpers:
  - `run_value_matches(value: &str, exe: &Path) -> bool`
  - `approved_enabled(bytes: Option<&[u8]>) -> bool`: `None` or an empty slice → true, an even first byte → true, an odd first byte → false.
- `Cargo.toml`: add the `Win32_System_Registry` feature to `windows`.

### Tray (`src/tray.rs`, `src/main.rs`)

- A `CheckMenuItem` "Start with Windows" goes after "Open config folder" and before the separator above "Quit". Its initial check is `autostart::is_enabled()`.
- A new `TrayEvent::ToggleAutostart`. `main.rs` handles it with `autostart::set(!autostart::is_enabled())` and then sets the check from a fresh `is_enabled()`, so the checkmark always reflects the registry.
- If `set` fails: the existing tray balloon "Couldn't change start with Windows: {error}", a `log::error!`, and the check is set back from `is_enabled()`.
- If the Run value names another Murmur copy, the item shows unchecked. Ticking it points the value at this copy.

## CI (`.github/workflows/release.yml`)

After **Package**, two new steps run. Nothing else changes.

1. **Install Inno Setup:** `choco install innosetup -y --no-progress`. It isn't preinstalled on the windows-2025 runner image (actions/runner-images issues #11644, #12947).
2. **Build installer:** `iscc /DAppVersion=<version> installer\murmur.iss` from the repo root (the script sets `SourceDir=..` and `OutputDir=.`, so the setup exe lands in the repo root), using the version the existing "Read version" step takes from `Cargo.toml`. That step also produces `murmur-vX.Y.Z-setup.exe.sha256` in the same LF format as the zip's.

The artifact upload (`workflow_dispatch`) and `gh release create` (tag) include the setup exe and its `.sha256`, so a release carries 4 files.

**Where signing would go (not built):** sign `target\release\murmur.exe` after Build and before Package (so both the zip and the installer carry the signed exe), then sign `murmur-vX.Y.Z-setup.exe` after it's built and before hashing.

## README

- Install: "Download `murmur-vX.Y.Z-setup.exe` (recommended) or the portable zip." The SmartScreen note stays, because the installer is unsigned, so it appears on the setup exe.
- A new line: uninstall from Settings → Apps (or Add/Remove Programs). It asks whether to keep the model and your settings.
- Updating: installer users run the new setup exe. Zip users keep the current steps.
- The tray-menu table gets a "Start with Windows" row.
- Build from source: "To build the installer locally, install Inno Setup 6 (`winget install JRSoftware.InnoSetup`) and run `iscc /DAppVersion=X.Y.Z installer\murmur.iss` after `cargo build --release`."

## Testing

| Level | What |
|---|---|
| Unit | `run_value_matches` (exact, case-different, quoted, other path, empty); `approved_enabled` (None, empty, `0x02`, `0x03`, `0x06`, `0x07`). |
| Local build | `cargo build --release --locked`, all tests, and `iscc` succeeds locally with Inno Setup installed via winget. |
| Smoke (Jeff's machine) | Install → Start-menu entry, `Run\Murmur` value, Add/Remove Programs entry. The tray toggle off/on, checking the registry each time. Disabling in Task Manager → the tray shows unchecked; ticking it re-enables. Upgrade over a running Murmur → it closes, files are replaced, the Run choice is kept. Uninstall with the model prompt Yes/settings No, then reinstall and uninstall with No/Yes → the right folders go. Sign out and back in → Murmur starts. |
| Clean machine | Jeff runs the CI-built setup exe in Windows Sandbox; SmartScreen appears on the setup exe ("More info → Run anyway"). |

What a green build won't prove: that Restart Manager actually closes the tray app (it has no visible top-level window), and that sign-in autostart works. Both are smoke-only. If `CloseApplications=force` doesn't close Murmur, the fallback is a `[Code]` `taskkill /F /IM murmur.exe` in `PrepareToInstall` and `InitializeUninstall`.
