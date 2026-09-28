# Murmur Installer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship Murmur with a per-user Inno Setup installer (Start menu, Add/Remove Programs uninstaller, optional start with Windows) alongside the zip, plus a "Start with Windows" tray toggle.

**Architecture:** A new bin module `src/autostart.rs` owns the `HKCU\...\Run\Murmur` value (plus Task Manager's `StartupApproved` flag); the tray gets a `CheckMenuItem` wired to it. `installer/murmur.iss` builds `murmur-vX.Y.Z-setup.exe` from the same files as the zip and writes/clears the same registry values in `[Code]`. `release.yml` installs Inno Setup with choco and builds the installer after the zip.

**Tech Stack:** Rust (`windows` 0.62 registry API, `tray-icon` 0.24 / `muda` 0.19 `CheckMenuItem`), Inno Setup 6 (Pascal `[Code]`), GitHub Actions (windows-latest = windows-2025).

**Spec:** `docs/superpowers/specs/2026-09-28-murmur-installer-design.md`. Read it before starting; this plan argues from it.

## Global Constraints

- Branch `feat/installer` in `C:\Users\JeffLocal\git\murmur` (the public clone). Never push, merge or tag without Jeff's say-so.
- Commits: conventional prefix (`feat:`, `docs:`, `ci:`), author comes from repo config (`jshowah-dev`), end every message with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` (repo convention).
- Registry key strings, verbatim: `Software\Microsoft\Windows\CurrentVersion\Run` and `Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run`, value name `Murmur`, root `HKCU` only.
- Install dir `{localappdata}\Programs\Murmur`; `PrivilegesRequired=lowest`; publisher "Jeff Showah"; URL `https://github.com/jshowah-dev/murmur`; `AppId` GUID `3859BC9B-2892-4D3F-8616-9C7EB9B7AD57` (never change it; upgrades depend on it).
- Installer files = zip files: `murmur.exe`, `onnxruntime.dll`, `onnxruntime_providers_shared.dll`, `sherpa-onnx-c-api.dll`, `sherpa-onnx-cxx-api.dll`, `README.md`, `LICENSE`.
- Uninstall prompts, verbatim, both default **No**: "Delete the downloaded speech model (about 660 MB)? You'd need to download it again if you reinstall." then "Delete your settings, dictionary and snippets?". Silent uninstall skips both and keeps data.
- Tray label verbatim: `Start with Windows`. Error balloon body: `Couldn't change start with Windows: {error}`.
- Version comes only from `Cargo.toml`; the installer receives it as `/DAppVersion=`. Bump to `0.2.0` happens in Task 5.
- No code signing, no auto-update, no MSI (spec: out of scope).
- Bin tests need the sherpa DLLs next to the test binary: `Copy-Item target/release/*.dll target/release/deps/` once after a release build.

## Review Focus

1. **Install path with spaces** (a Windows user name like "Jeff Local" puts a space in `%LOCALAPPDATA%`): the Run value must be quoted, and `is_enabled` must still recognise it. Pinned by `run_value_for_quotes_paths_with_spaces` and `matches_quoted_path_with_spaces` in Task 1.
2. **Same file, different spelling** (case, surrounding quotes/whitespace, `..` or 8.3 short-name components between the Run value and `current_exe()`): must count as a match, or the tray shows unchecked for a copy that does start. Pinned by `matches_ignoring_case`, `matches_after_canonicalizing` in Task 1.
3. **Murmur running during upgrade or uninstall**: it must close without a "files in use / restart" prompt, since a tray app has no visible window for Restart Manager to close. Pinned by smoke steps in Task 3, with the `taskkill` fallback written out there.
4. **A zip copy and an installed copy both present**: only the copy the Run value names shows checked; ticking the other repoints the value. Pinned by `other_path_does_not_match` in Task 1 and a smoke step in Task 2.
5. **Uninstall "Yes" when the model folder is missing or half-downloaded** (`.part`, `.staging` leftovers): no error dialog, whatever is there is removed. Pinned by a smoke step in Task 3.

---

## File map

| File | Change | Responsibility |
|---|---|---|
| `src/autostart.rs` | Create | Read/write the Run + StartupApproved values; pure helpers with unit tests |
| `Cargo.toml` | Modify | Add `Win32_System_Registry` to `windows` features; version bump (Task 5) |
| `src/main.rs` | Modify | `mod autostart;`, handle `TrayEvent::ToggleAutostart` |
| `src/tray.rs` | Modify | `CheckMenuItem` "Start with Windows", `TrayEvent::ToggleAutostart`, `set_autostart_checked` |
| `installer/murmur.iss` | Create | Inno Setup script |
| `.github/workflows/release.yml` | Modify | Install Inno Setup, build installer + `.sha256`, upload/release them |
| `README.md` | Modify | Install/Uninstall/Updating/tray table/Build from source |

---

### Task 1: `autostart` module

**Files:**
- Create: `src/autostart.rs`
- Modify: `Cargo.toml` (windows features list, lines 19-38), `src/main.rs:4-14` (module list)
- Test: unit tests inside `src/autostart.rs`

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces (used by Task 2):
  - `pub fn is_enabled() -> bool`
  - `pub fn set(enabled: bool) -> anyhow::Result<()>`
  - (internal, tested) `fn run_value_for(exe: &Path) -> String`, `fn run_value_matches(value: &str, exe: &Path) -> bool`, `fn approved_enabled(bytes: Option<&[u8]>) -> bool`

- [ ] **Step 1: Add the registry feature**

In `Cargo.toml`, inside the `windows = { version = "0.62", features = [ ... ] }` list, add after `"Win32_System_Threading",`:

```toml
  "Win32_System_Registry",
```

- [ ] **Step 2: Write the module with failing tests first**

Create `src/autostart.rs` with the tests and stub helpers:

```rust
//! Start with Windows: the HKCU Run value, the same one the installer writes.

use anyhow::Result;
use std::path::Path;

fn run_value_for(_exe: &Path) -> String {
    unimplemented!()
}

fn run_value_matches(_value: &str, _exe: &Path) -> bool {
    unimplemented!()
}

fn approved_enabled(_bytes: Option<&[u8]>) -> bool {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_value_for_quotes_paths_with_spaces() {
        let exe = Path::new(r"C:\Users\Jeff Local\AppData\Local\Programs\Murmur\murmur.exe");
        assert_eq!(run_value_for(exe), r#""C:\Users\Jeff Local\AppData\Local\Programs\Murmur\murmur.exe""#);
    }

    #[test]
    fn matches_quoted_path_with_spaces() {
        let exe = Path::new(r"C:\Users\Jeff Local\murmur.exe");
        assert!(run_value_matches(r#""C:\Users\Jeff Local\murmur.exe""#, exe));
        assert!(run_value_matches(r"C:\Users\Jeff Local\murmur.exe", exe));
        assert!(run_value_matches("  \"C:\\Users\\Jeff Local\\murmur.exe\"  ", exe));
    }

    #[test]
    fn matches_ignoring_case() {
        assert!(run_value_matches(r#""c:\users\jeff\MURMUR.EXE""#, Path::new(r"C:\Users\Jeff\murmur.exe")));
    }

    #[test]
    fn other_path_does_not_match() {
        assert!(!run_value_matches(r#""C:\Tools\murmur\murmur.exe""#, Path::new(r"C:\Users\Jeff\murmur.exe")));
    }

    #[test]
    fn empty_value_does_not_match() {
        assert!(!run_value_matches("", Path::new(r"C:\x\murmur.exe")));
        assert!(!run_value_matches("\"\"", Path::new(r"C:\x\murmur.exe")));
    }

    #[test]
    fn matches_after_canonicalizing() {
        // a real file reached through a `..` component: the spelling differs, the file doesn't
        let exe = std::env::current_exe().unwrap();
        let dir = exe.parent().unwrap();
        let name = exe.file_name().unwrap();
        let roundabout = dir.join("..").join(dir.file_name().unwrap()).join(name);
        assert!(run_value_matches(&format!("\"{}\"", roundabout.display()), &exe));
    }

    #[test]
    fn approved_flag() {
        assert!(approved_enabled(None));
        assert!(approved_enabled(Some(&[])));
        assert!(approved_enabled(Some(&[0x02, 0, 0, 0])));
        assert!(!approved_enabled(Some(&[0x03, 0, 0, 0])));
        assert!(approved_enabled(Some(&[0x06])));
        assert!(!approved_enabled(Some(&[0x07])));
    }
}
```

In `src/main.rs`, add `mod autostart;` to the module list (alphabetical, before `mod caret;`):

```rust
mod audio_out;
mod autostart;
mod caret;
```

- [ ] **Step 3: Run the tests to see them fail**

Run (pwsh, repo root): `cargo test --release --locked --bin murmur autostart`
Expected: 7 tests FAIL with `not implemented`. (Dead-code warnings for the stubs are fine.)

- [ ] **Step 4: Implement the helpers and the registry I/O**

Replace everything above `#[cfg(test)]` in `src/autostart.rs` with:

```rust
//! Start with Windows: the HKCU Run value, the same one the installer writes.

use anyhow::Result;
use std::path::Path;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
use windows::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_ROUTINE_FLAGS, REG_SZ, RRF_RT_REG_BINARY, RRF_RT_REG_SZ,
};

const RUN: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
// Task Manager's "Startup apps" switch: an odd first byte means disabled
const APPROVED: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run");
const NAME: PCWSTR = w!("Murmur");

/// True when Windows will start this copy of Murmur at sign-in.
pub fn is_enabled() -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    read_string(RUN).is_some_and(|v| run_value_matches(&v, &exe)) && approved_enabled(read(APPROVED, RRF_RT_REG_BINARY).as_deref())
}

/// Points the Run value at this copy (clearing a Task Manager "disabled"), or removes it.
pub fn set(enabled: bool) -> Result<()> {
    if !enabled {
        return delete(RUN);
    }
    let exe = std::env::current_exe()?;
    let data: Vec<u16> = run_value_for(&exe).encode_utf16().chain([0]).collect();
    unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, RUN, NAME, REG_SZ.0, Some(data.as_ptr().cast()), (data.len() * 2) as u32) }.ok()?;
    delete(APPROVED)
}

/// The Run value for `exe`, quoted so a path with spaces still starts.
fn run_value_for(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}

/// Whether a Run value names `exe`, ignoring quotes, surrounding spaces, case and path spelling.
fn run_value_matches(value: &str, exe: &Path) -> bool {
    let v = value.trim().trim_matches('"');
    if v.is_empty() {
        return false;
    }
    let norm = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()).to_string_lossy().to_lowercase();
    norm(Path::new(v)) == norm(exe)
}

/// StartupApproved data: missing or an even first byte means enabled.
fn approved_enabled(bytes: Option<&[u8]>) -> bool {
    bytes.and_then(|b| b.first()).is_none_or(|b| b % 2 == 0)
}

fn read(key: PCWSTR, flags: REG_ROUTINE_FLAGS) -> Option<Vec<u8>> {
    let mut len = 0u32;
    unsafe { RegGetValueW(HKEY_CURRENT_USER, key, NAME, flags, None, None, Some(&mut len)) }.ok().ok()?;
    let mut buf = vec![0u8; len as usize];
    unsafe { RegGetValueW(HKEY_CURRENT_USER, key, NAME, flags, None, Some(buf.as_mut_ptr().cast()), Some(&mut len)) }.ok().ok()?;
    buf.truncate(len as usize);
    Some(buf)
}

fn read_string(key: PCWSTR) -> Option<String> {
    let bytes = read(key, RRF_RT_REG_SZ)?;
    let wide: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
    Some(String::from_utf16_lossy(&wide))
}

fn delete(key: PCWSTR) -> Result<()> {
    let e = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key, NAME) };
    if e != ERROR_FILE_NOT_FOUND {
        e.ok()?;
    }
    Ok(())
}
```

Notes for the implementer:
- `Option::is_none_or` needs Rust 1.82+. Check with `rustc --version`; if older, use `.map_or(true, |b| b % 2 == 0)`.
- `RegDeleteKeyValueW` returns `ERROR_FILE_NOT_FOUND` for a missing value **or** a missing key (StartupApproved\Run may not exist); both mean "already gone".
- `canonicalize` fails for paths that don't exist (the unit tests' fake paths), so they fall back to the literal spelling; that's intended.
- `is_enabled`/`set` aren't used until Task 2, so expect dead-code warnings in this task.

- [ ] **Step 5: Run the tests to see them pass**

Run: `cargo test --release --locked --bin murmur autostart`
Expected: 7 passed, 0 failed. If the test binary crashes on start (`STATUS_ENTRYPOINT_NOT_FOUND` / DLL error), run `Copy-Item target/release/*.dll target/release/deps/` and re-run.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml src/autostart.rs src/main.rs
git commit -m "feat: autostart module for the HKCU Run value" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: "Start with Windows" tray toggle

**Files:**
- Modify: `src/tray.rs:3` (imports), `:9-18` (enum), `:20-24` (struct), `:42-63` (create), after `:73` (new method)
- Modify: `src/main.rs:189-192` (tray event match)
- Modify: `README.md:28` and the tray table `:48-56`

**Interfaces:**
- Consumes (Task 1): `crate::autostart::is_enabled() -> bool`, `crate::autostart::set(bool) -> anyhow::Result<()>`
- Produces: `TrayEvent::ToggleAutostart`, `Tray::set_autostart_checked(&self, on: bool)`

- [ ] **Step 1: Add the menu item to `src/tray.rs`**

Import line 3 becomes:

```rust
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
```

Enum gains a variant before `Quit`:

```rust
    OpenConfigDir,
    ToggleAutostart,
    Quit,
```

Struct:

```rust
pub struct Tray {
    _icon: TrayIcon,
    pause: MenuItem,
    autostart: CheckMenuItem,
    ids: [(MenuId, TrayEvent); 8],
}
```

In `create()`, after `let cfg = ...;`:

```rust
        let autostart = CheckMenuItem::new("Start with Windows", true, crate::autostart::is_enabled(), None);
```

The `append_items` call becomes:

```rust
        menu.append_items(&[&pause, &fix, &hist, &PredefinedMenuItem::separator(), &dict, &snip, &cfg, &autostart, &PredefinedMenuItem::separator(), &quit])?;
```

The `ids` array gains, before the `quit` entry:

```rust
            (autostart.id().clone(), TrayEvent::ToggleAutostart),
```

and the return becomes `Ok(Tray { _icon, pause, autostart, ids })`.

After `set_paused`, add:

```rust
    /// muda flips a CheckMenuItem on click; the caller sets it back from the registry.
    pub fn set_autostart_checked(&self, on: bool) {
        self.autostart.set_checked(on);
    }
```

- [ ] **Step 2: Handle the event in `src/main.rs`**

In the `match ev` on tray events, before `TrayEvent::Quit => break,`:

```rust
                TrayEvent::ToggleAutostart => {
                    // the registry, not the menu's own check state, says what's on
                    if let Err(e) = autostart::set(!autostart::is_enabled()) {
                        log::error!("autostart: {e:#}");
                        tray.notify("Murmur", &format!("Couldn't change start with Windows: {e}"));
                    }
                    tray.set_autostart_checked(autostart::is_enabled());
                }
```

- [ ] **Step 3: Build and run all tests**

Run: `cargo build --release --locked` then `cargo test --release --locked --bin murmur` and `cargo test --release --locked --lib`
Expected: build clean with no warnings from `autostart.rs`/`tray.rs`; bin tests = previous count + 7 (was 35 → 42), lib 69, 0 failed.

- [ ] **Step 4: Smoke the toggle (real registry)**

Quit the running Murmur (tray → Quit), then start `target\release\murmur.exe`. Use this to read the registry:

```powershell
Get-ItemProperty HKCU:\Software\Microsoft\Windows\CurrentVersion\Run -Name Murmur -ErrorAction SilentlyContinue | Select-Object Murmur
Get-ItemProperty HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run -Name Murmur -ErrorAction SilentlyContinue
```

Check, in order (Jeff clicks the tray; Claude reads the registry):
1. Menu shows "Start with Windows" between "Open config folder" and the separator, unchecked (no Run value yet).
2. Click it → checked; Run value = `"C:\Users\JeffLocal\git\murmur\target\release\murmur.exe"` (quoted).
3. Click again → unchecked; Run value gone.
4. Click on again, then Task Manager → Startup apps → Murmur → Disable. Quit and restart Murmur → item shows **unchecked**. Click it → checked, StartupApproved `Murmur` value gone, Task Manager shows Enabled.
5. (Review Focus 4) Set the Run value to another path: `Set-ItemProperty HKCU:\Software\Microsoft\Windows\CurrentVersion\Run -Name Murmur -Value '"C:\Elsewhere\murmur.exe"'`, restart Murmur → unchecked; click → value now points at this exe.
6. Leave it **off** at the end (click until unchecked) so Task 3's installer smoke starts clean.

Tail `%APPDATA%\Murmur\murmur.log` for `autostart:` errors — expected none.

- [ ] **Step 5: README**

Replace line 28 (`To start Murmur with Windows, put a shortcut to `murmur.exe` in `shell:startup`.`) with:

```markdown
To start Murmur with Windows, tick **Start with Windows** in the tray menu.
```

In the tray table, after the `| Open config folder | Open `%APPDATA%\Murmur` |` row, add:

```markdown
| Start with Windows | Start Murmur when you sign in (on/off) |
```

- [ ] **Step 6: Commit**

```bash
git add src/tray.rs src/main.rs README.md
git commit -m "feat: Start with Windows toggle in the tray menu" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Inno Setup script, local build and smoke

**Files:**
- Create: `installer/murmur.iss`
- Modify: `README.md` (Install, new Uninstall section, Updating, Build from source)
- Modify: `.gitignore` (setup output)

**Interfaces:**
- Consumes: `target\release\` build output (Task 2), the registry contract from Global Constraints.
- Produces: `installer/murmur.iss` that builds `murmur-v<AppVersion>-setup.exe` in the repo root when run from the repo root as `ISCC.exe /DAppVersion=X.Y.Z installer\murmur.iss` (used by Task 4).

- [ ] **Step 1: Install Inno Setup locally (Jeff)**

Jeff runs in his own terminal (it may prompt for elevation):

```bash
winget install --id JRSoftware.InnoSetup -e
```

Then Claude checks: `Test-Path "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe"` or `Test-Path "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"` → one is `True`. Use whichever exists as `$iscc` below.

- [ ] **Step 2: Write `installer/murmur.iss`**

```ini
; Build from the repo root: ISCC.exe /DAppVersion=X.Y.Z installer\murmur.iss
#ifndef AppVersion
  #error Pass the version: /DAppVersion=X.Y.Z
#endif

[Setup]
AppId={{3859BC9B-2892-4D3F-8616-9C7EB9B7AD57}
AppName=Murmur
AppVersion={#AppVersion}
AppVerName=Murmur {#AppVersion}
AppPublisher=Jeff Showah
AppPublisherURL=https://github.com/jshowah-dev/murmur
AppSupportURL=https://github.com/jshowah-dev/murmur/issues
; paths below are relative to the repo root
SourceDir=..
OutputDir=.
OutputBaseFilename=murmur-v{#AppVersion}-setup
DefaultDirName={localappdata}\Programs\Murmur
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
SetupIconFile=assets\murmur.ico
UninstallDisplayIcon={app}\murmur.exe
UninstallDisplayName=Murmur
WizardStyle=modern
Compression=lzma2
SolidCompression=yes
CloseApplications=force
RestartApplications=no
; the autostart task's state comes from the registry, not the last install
UsePreviousTasks=no

[Tasks]
Name: "autostart"; Description: "Start Murmur with Windows"
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "target\release\murmur.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "target\release\onnxruntime.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "target\release\onnxruntime_providers_shared.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "target\release\sherpa-onnx-c-api.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "target\release\sherpa-onnx-cxx-api.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Murmur"; Filename: "{app}\murmur.exe"
Name: "{autodesktop}\Murmur"; Filename: "{app}\murmur.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\murmur.exe"; Description: "{cm:LaunchProgram,Murmur}"; Flags: nowait postinstall skipifsilent

[Code]
const
  RunKey = 'Software\Microsoft\Windows\CurrentVersion\Run';
  ApprovedKey = 'Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run';

var
  TasksPrimed: Boolean;

{ On an upgrade, show start-with-Windows as it is now (the tray may have changed it). }
procedure CurPageChanged(CurPageID: Integer);
begin
  if (CurPageID = wpSelectTasks) and not TasksPrimed then
  begin
    TasksPrimed := True;
    if WizardForm.PrevAppDir <> '' then
    begin
      if RegValueExists(HKCU, RunKey, 'Murmur') then
        WizardSelectTasks('autostart')
      else
        WizardSelectTasks('!autostart');
    end;
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
  begin
    if WizardIsTaskSelected('autostart') then
    begin
      RegWriteStringValue(HKCU, RunKey, 'Murmur', '"' + ExpandConstant('{app}\murmur.exe') + '"');
      RegDeleteValue(HKCU, ApprovedKey, 'Murmur');
    end
    else
      RegDeleteValue(HKCU, RunKey, 'Murmur');
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
  begin
    RegDeleteValue(HKCU, RunKey, 'Murmur');
    RegDeleteValue(HKCU, ApprovedKey, 'Murmur');
  end;
  if (CurUninstallStep = usPostUninstall) and not UninstallSilent then
  begin
    if MsgBox('Delete the downloaded speech model (about 660 MB)? You''d need to download it again if you reinstall.',
              mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
    begin
      DelTree(ExpandConstant('{localappdata}\Murmur\models'), True, True, True);
      RemoveDir(ExpandConstant('{localappdata}\Murmur'));
    end;
    if MsgBox('Delete your settings, dictionary and snippets?', mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
      DelTree(ExpandConstant('{userappdata}\Murmur'), True, True, True);
  end;
end;
```

Notes for the implementer:
- `AppId={{...}` — the doubled `{` is Inno's escape; the stored id is `{3859BC9B-...}`.
- `WizardForm.PrevAppDir` is empty on a fresh install, so the task keeps its default (checked). If `iscc` rejects `PrevAppDir`, replace the test with `RegKeyExists(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{3859BC9B-2892-4D3F-8616-9C7EB9B7AD57}_is1')`.
- A silent upgrade (`/SILENT`) never shows the Tasks page, so the task keeps its default (on). Accepted; not in the spec's silent-install scope.

- [ ] **Step 3: Ignore the build output**

Append to `.gitignore`:

```
/murmur-v*-setup.exe
```

- [ ] **Step 4: Build the installer**

From the repo root (pwsh), with `target\release` from Task 2 current:

```powershell
& $iscc /DAppVersion=0.1.0 installer\murmur.iss
```

Expected: `Successful compile`, and `murmur-v0.1.0-setup.exe` in the repo root (≈13 MB). (0.1.0 here is only for the smoke; Task 5 bumps the real version.)

- [ ] **Step 5: Smoke — fresh install**

Quit the dev Murmur first. Jeff runs `murmur-v0.1.0-setup.exe` (no UAC prompt expected; SmartScreen may appear since it's unsigned). Leave "Start Murmur with Windows" ticked and "Launch Murmur" ticked. Claude checks:

```powershell
Test-Path "$env:LOCALAPPDATA\Programs\Murmur\murmur.exe"                       # True
Get-ChildItem "$env:LOCALAPPDATA\Programs\Murmur" | Select-Object Name          # 5 binaries + README.md + LICENSE + unins000.*
Get-ItemProperty HKCU:\Software\Microsoft\Windows\CurrentVersion\Run -Name Murmur  # "…\Programs\Murmur\murmur.exe" quoted
Test-Path "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\Murmur.lnk"      # True
Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{3859BC9B-2892-4D3F-8616-9C7EB9B7AD57}_is1' | Select-Object DisplayName, DisplayVersion, Publisher, URLInfoAbout
Get-Process murmur | Select-Object Path                                          # the Programs\Murmur copy
```

Jeff confirms: Murmur appears in Settings → Apps with the teal icon; tray menu "Start with Windows" is **checked**; a dictation works (model already present, so no download).

- [ ] **Step 6: Smoke — upgrade over a running Murmur (Review Focus 3)**

With the installed Murmur running, Jeff untoggles "Start with Windows" in the tray, then re-runs the same setup exe. Expected:
- The Tasks page shows "Start Murmur with Windows" **unticked** (it follows the registry).
- No "files in use / restart the computer" prompt; Murmur is closed and the install completes; "Launch Murmur" restarts it.
- Run value still absent afterwards (Claude checks).

**If Murmur is not closed** (Setup reports the files in use or asks for a restart): add this to `[Code]` and rebuild, then repeat this step:

```pascal
procedure KillMurmur;
var
  Code: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM murmur.exe', '', SW_HIDE, ewWaitUntilTerminated, Code);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  KillMurmur;
  Result := '';
end;

function InitializeUninstall(): Boolean;
begin
  KillMurmur;
  Result := True;
end;
```

and record in the spec's Testing section that the fallback was needed.

- [ ] **Step 7: Smoke — sign-in autostart**

Tick "Start with Windows" again in the tray. Jeff signs out and back in. Expected: Murmur's tray icon appears without launching it by hand; `Get-Process murmur | Select-Object Path` shows the installed copy.

- [ ] **Step 8: Smoke — uninstall, both prompt orders (Review Focus 5)**

Before this, back up the dictionary so a "Yes" can't lose Jeff's terms:

```powershell
Copy-Item -Recurse "$env:APPDATA\Murmur" "$env:USERPROFILE\Downloads\murmur-appdata-backup"
```

Round 1: Jeff uninstalls from Settings → Apps with Murmur running. Expected: Murmur closes; model prompt → **Yes**; settings prompt → **No**. Claude checks: `Programs\Murmur` gone, Start-menu shortcut gone, `Run\Murmur` gone, uninstall key gone, `%LOCALAPPDATA%\Murmur\models` gone, `%APPDATA%\Murmur` still present.

Round 2: reinstall (the setup exe runs the first-run model download: let it finish, ~460 MB). Before uninstalling, simulate a half-download leftover: `New-Item "$env:LOCALAPPDATA\Murmur\models\leftover.part" -Force`. Uninstall; model prompt → **No**; settings prompt → **Yes**. Expected: `models` (with `leftover.part`) still present, `%APPDATA%\Murmur` gone.

Round 3 (missing folder): reinstall, `Remove-Item -Recurse "$env:LOCALAPPDATA\Murmur"`, uninstall, answer **Yes** to both → no error dialog.

Restore Jeff's data afterwards and put his setup back as he wants it:

```powershell
Copy-Item -Recurse -Force "$env:USERPROFILE\Downloads\murmur-appdata-backup\*" "$env:APPDATA\Murmur\"
```

Then either reinstall and let the model download, or run the dev build — Jeff's call. Delete the backup folder only when Jeff confirms his dictionary is back.

- [ ] **Step 9: README**

Replace the `## Install` numbered list (lines 24-26) with:

```markdown
1. Download `murmur-vX.Y.Z-setup.exe` from [Releases](../../releases) and run it. It installs for your user only (no admin prompt), adds Murmur to the Start menu and, if you leave the box ticked, starts it with Windows. Prefer no installer? Download `murmur-vX.Y.Z-windows-x64.zip` instead and unzip it anywhere, keeping the DLLs next to `murmur.exe`.
2. Murmur isn't code-signed yet, so Windows SmartScreen may say "Windows protected your PC". Click **More info → Run anyway**.
3. On first launch Murmur downloads the speech model (about 460 MB, one time) to `%LOCALAPPDATA%\Murmur\models`, then shows a short "you're ready" screen.
```

After the "To start Murmur with Windows…" line, add:

```markdown
### Uninstall

Settings → Apps → Murmur → Uninstall. It asks whether to also delete the speech model and your settings, dictionary and snippets; both default to keeping them. If you used the zip, quit Murmur, untick **Start with Windows** first, then delete the folder.
```

Replace the `## Updating` numbered list (lines 34-36) with:

```markdown
1. Download the new `murmur-vX.Y.Z-setup.exe` from [Releases](../../releases) and run it. It closes Murmur, updates it in place and keeps your start-with-Windows choice.
2. Zip users: quit Murmur (tray icon → Quit), unzip the new zip over the old folder, replacing the files, and start `murmur.exe`.
```

In `## Build from source`, after the "`target/release/` then holds…" paragraph, add:

````markdown
To build the installer, install [Inno Setup 6](https://jrsoftware.org/isinfo.php) (`winget install JRSoftware.InnoSetup`) and run, from the repo root after `cargo build --release`:

```bash
ISCC.exe /DAppVersion=X.Y.Z installer\murmur.iss
```
````

- [ ] **Step 10: Commit**

```bash
git add installer/murmur.iss .gitignore README.md
git commit -m "feat: per-user Inno Setup installer with uninstall prompts" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Build the installer in CI

**Files:**
- Modify: `.github/workflows/release.yml` (after the `Package` step; `Upload artifact`; `Draft release`)

**Interfaces:**
- Consumes (Task 3): `installer\murmur.iss`, run from the repo root, writes `murmur-v<ver>-setup.exe` to the repo root. `steps.ver.outputs.version`, `steps.pkg.outputs.name` (existing).
- Produces: `steps.setup.outputs.name` = `murmur-v<ver>-setup.exe`; release assets zip + setup exe + both `.sha256`.

- [ ] **Step 1: Add the two steps after `Package`**

```yaml
      # not preinstalled on windows-2025 (actions/runner-images #11644, #12947)
      - name: Install Inno Setup
        shell: pwsh
        run: |
          choco install innosetup -y --no-progress
          if ($LASTEXITCODE) { exit $LASTEXITCODE }

      - name: Build installer
        id: setup
        shell: pwsh
        run: |
          & "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe" /DAppVersion=${{ steps.ver.outputs.version }} installer\murmur.iss
          if ($LASTEXITCODE) { exit $LASTEXITCODE }
          $name = "murmur-v${{ steps.ver.outputs.version }}-setup.exe"
          $hash = (Get-FileHash $name -Algorithm SHA256).Hash.ToLower()
          [IO.File]::WriteAllText("$PWD/$name.sha256", "$hash  $name`n")
          "name=$name" >> $env:GITHUB_OUTPUT
```

- [ ] **Step 2: Ship the new files**

`Upload artifact` `path:` becomes:

```yaml
          path: |
            ${{ steps.pkg.outputs.name }}.zip
            ${{ steps.pkg.outputs.name }}.zip.sha256
            ${{ steps.setup.outputs.name }}
            ${{ steps.setup.outputs.name }}.sha256
```

`Draft release` `run:` becomes:

```yaml
        run: gh release create "${{ github.ref_name }}" --draft --title "Murmur ${{ github.ref_name }}" "${{ steps.pkg.outputs.name }}.zip" "${{ steps.pkg.outputs.name }}.zip.sha256" "${{ steps.setup.outputs.name }}" "${{ steps.setup.outputs.name }}.sha256"
```

- [ ] **Step 3: Check the YAML parses**

Run: `python -c "import yaml,sys; yaml.safe_load(open('.github/workflows/release.yml')); print('ok')"` (install `pyyaml` with `pip --user` if missing)
Expected: `ok`.

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "ci: build the installer next to the zip" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 5: STOP — CI dry run needs a push (Jeff's call)**

A `workflow_dispatch` run only sees pushed code. Ask Jeff: push `feat/installer` and run `gh workflow run release.yml --ref feat/installer -R jshowah-dev/murmur`? If yes, push with `git -c credential.helper= -c credential.helper='!gh auth git-credential' push -u origin feat/installer`, trigger it, and when it finishes: `gh run download <id> -R jshowah-dev/murmur` → 4 files; `Get-FileHash` of the setup exe matches its `.sha256`. Expected run time ≈ previous release run + ~1 min.

---

### Task 5: Version bump and wrap-up

**Files:**
- Modify: `Cargo.toml` (`version`), `Cargo.lock` (auto)

- [ ] **Step 1: Bump to 0.2.0**

In `Cargo.toml`, `version = "0.1.0"` → `version = "0.2.0"`. Run `cargo build --release` (not `--locked`: the lock's own `murmur` version entry must update), then `cargo build --release --locked` → clean.

- [ ] **Step 2: Full test run**

Run: `cargo test --release --locked --bin murmur`, `cargo test --release --locked --lib`, `cargo test --release --locked --test stt_integration -- --nocapture`
Expected: bin 42, lib 69, integration 1 passed (model is installed locally, so no "skipping" line), 0 failed.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to 0.2.0" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 4: STOP — merge, tag and publish are Jeff's**

Report to Jeff: branch `feat/installer` commits, test counts, smoke results (Tasks 2-3), whether the `taskkill` fallback was needed, CI dry-run result if run. Then ask: merge to `main`, push, tag `v0.2.0` (creates a draft release), Sandbox-check the CI setup exe, publish? Each is a separate yes. Write a handoff (handoff skill) superseding `2026-09-28-murmur-public-release-handoff.md` before ending the session.
