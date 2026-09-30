# Murmur Updates Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A running Murmur finds a newer GitHub release within a day, says so once, and updates itself from one tray click; installs on a previous default model are offered the current one.

**Architecture:** A new library module `src/update.rs` holds everything testable (release parsing, checksum file, installed-copy gate, alert-once state, offer state, check/download/install). `main.rs` owns the threads' messages and the tray; `tray.rs` gains two optional menu items; `installer/murmur.iss` relaunches Murmur when passed `/RELAUNCH`. The model mechanism is `config::resolve_model`, run at startup before the pipeline loads the model.

**Tech Stack:** Rust 2021, ureq 3 (rustls), sha2, tray-icon 0.24 / muda 0.19.3, eframe 0.36, Inno Setup 6.

**Spec:** `docs/superpowers/specs/2026-09-29-murmur-updates-design.md`

## Global Constraints

- Branch `feat/updates` (already created, spec committed as `69579cb`). Don't push, don't bump the version, don't release.
- Checks: first ~60 s after launch, then every 24 h. About keeps its own check while open.
- One balloon per new version; the menu item shows regardless. Balloon text: "Murmur vX is available. Update from the tray menu."
- Install args, exactly: `/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /FORCECLOSEAPPLICATIONS /RELAUNCH`.
- Click-to-update only for the installed copy (`%LOCALAPPDATA%\Programs\Murmur\murmur.exe`, case-insensitive); other copies get "Get vX…" which opens the release page.
- `config.toml` is never rewritten by this work. `PREVIOUS_DEFAULTS` ships empty.
- Model offer text: "A new speech model is available (N MB). Download from the tray menu."; ready text: "New speech model ready. It's used from the next start."
- No serde_json: JSON is read by string search, like About today.
- No repaint loops in egui windows (History's comment: they starve the overlay and tray).
- Commit messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Comment density matches the surrounding code: short `///` on public items, a `//` line only where the why isn't obvious.

## Build and test command (read before Task 1)

The Build Tools registration is lost on this machine (`vswhere` returns nothing), so plain `cargo` fails at link time with `link: missing operand` (Git Bash's coreutils `link`). Every cargo command in this plan goes through a wrapper. Create it once:

```bash
cat > "$TEMP/murmur-cargo.cmd" <<'EOF'
@call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul
cd /d C:\Users\JeffLocal\git\murmur
cargo %*
EOF
```

Then `CARGO <args>` in this plan means:

```bash
cmd //c "$(cygpath -w "$TEMP/murmur-cargo.cmd")" <args>
```

Tests always run in release (`--release --locked`), like CI and earlier sessions; a debug build of the ORT stack is slow.

## Review Focus

1. **A second click on "Update to vX…" while the first download runs** → nothing happens (no second download or installer). Pinned by `offer_starts_once_until_it_fails` in Task 1.
2. **A newer release appears while one is already offered (v0.4.5 offered, then v0.4.6 in the same run)** → the menu moves to v0.4.6, and an older or equal tag never replaces it. Pinned by `offer_moves_only_to_a_newer_tag` in Task 1.
3. **A release without a setup asset (draft build, renamed asset)** → treated like a failed check: silent, no menu item. Pinned by `release_without_a_setup_asset_is_none` in Task 1.
4. **A `.sha256` file with a BOM, CRLF or uppercase hex** → still accepted; a short or non-hex value is refused. Pinned by `sha256_file_tolerates_bom_crlf_and_case` in Task 1.
5. **An update download failure** → the balloon names GitHub and the reason, never the model text "Model file not available at k2-fsa". Pinned by `failure_text_is_about_the_update` in Task 2.

---

### Task 1: `update.rs`, the pure parts

**Files:**
- Create: `src/update.rs`
- Create: `tests/fixtures/release-v0.4.4.json`
- Modify: `src/lib.rs` (add `pub mod update;`)
- Modify: `src/about_ui.rs` (remove `latest_tag`, `is_newer` and their two tests; import them from `murmur_lib::update`)

**Interfaces:**
- Consumes: `murmur_lib::config::config_dir() -> PathBuf`.
- Produces (all `pub` in `murmur_lib::update`):
  - `struct Release { pub tag: String, pub setup_url: String, pub setup_size: u64, pub sha_url: String }` deriving `Debug, Clone, PartialEq`. `tag` has no leading "v".
  - `fn latest_tag(json: &str) -> Option<String>`
  - `fn is_newer(latest: &str, current: &str) -> bool`
  - `fn parse_release(json: &str) -> Option<Release>`
  - `fn parse_sha256_file(text: &str) -> Option<String>` (lowercase hex)
  - `fn is_installed_copy(exe: &Path, localappdata: &Path) -> bool`
  - `fn should_alert(tag: &str, stored: Option<&str>) -> bool`, `fn read_alerted() -> Option<String>`, `fn write_alerted(tag: &str)`
  - `struct Offer` (`Default`) with `fn available(&mut self, r: Release) -> bool`, `fn release(&self) -> Option<&Release>`, `fn start(&mut self) -> Option<Release>`, `fn failed(&mut self)`

- [ ] **Step 1: Save the real release JSON as a fixture**

```bash
cd /c/Users/JeffLocal/git/murmur && gh api repos/jshowah-dev/murmur/releases/tags/v0.4.4 > tests/fixtures/release-v0.4.4.json && grep -c '"browser_download_url"' tests/fixtures/release-v0.4.4.json
```

Expected: `4`.

- [ ] **Step 2: Write the failing tests**

Create `src/update.rs` with only the tests (plus the module doc line) so they fail to compile:

```rust
//! Update checks against the latest GitHub release, and the parts of click-to-update that don't need a UI.

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const FIXTURE: &str = include_str!("../tests/fixtures/release-v0.4.4.json");

    fn release(tag: &str) -> Release {
        Release { tag: tag.into(), setup_url: format!("u{tag}"), setup_size: 1, sha_url: format!("s{tag}") }
    }

    #[test]
    fn latest_tag_reads_the_release_json() {
        let json = r#"{"url":"x","tag_name": "v0.4.3","name":"Murmur v0.4.3"}"#;
        assert_eq!(latest_tag(json).as_deref(), Some("0.4.3"));
        assert_eq!(latest_tag(r#"{"message":"Not Found"}"#), None);
    }

    #[test]
    fn versions_compare_by_number() {
        assert!(is_newer("0.4.10", "0.4.3"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("0.4.3", "0.4.3"));
        assert!(!is_newer("0.4.2", "0.4.3"));
    }

    #[test]
    fn parse_release_reads_the_real_v044_json() {
        let r = parse_release(FIXTURE).unwrap();
        assert_eq!(r.tag, "0.4.4");
        assert_eq!(r.setup_size, 10_691_109);
        assert_eq!(r.setup_url, "https://github.com/jshowah-dev/murmur/releases/download/v0.4.4/murmur-v0.4.4-setup.exe");
        assert_eq!(r.sha_url, "https://github.com/jshowah-dev/murmur/releases/download/v0.4.4/murmur-v0.4.4-setup.exe.sha256");
    }

    #[test]
    fn parse_release_allows_spaces_after_colons() {
        let json = r#"{"tag_name": "v1.2.3", "assets": [
            {"name": "murmur-v1.2.3-setup.exe", "size": 42, "browser_download_url": "https://x/setup.exe"},
            {"name": "murmur-v1.2.3-setup.exe.sha256", "size": 90, "browser_download_url": "https://x/setup.exe.sha256"}]}"#;
        let r = parse_release(json).unwrap();
        assert_eq!((r.tag.as_str(), r.setup_size, r.setup_url.as_str(), r.sha_url.as_str()), ("1.2.3", 42, "https://x/setup.exe", "https://x/setup.exe.sha256"));
    }

    #[test]
    fn release_without_a_setup_asset_is_none() {
        let json = r#"{"tag_name":"v1.2.3","assets":[{"name":"murmur-v1.2.3-windows-x64.zip","size":1,"browser_download_url":"https://x/z"}]}"#;
        assert_eq!(parse_release(json), None);
        let no_sha = r#"{"tag_name":"v1.2.3","assets":[{"name":"murmur-v1.2.3-setup.exe","size":1,"browser_download_url":"https://x/s"}]}"#;
        assert_eq!(parse_release(no_sha), None);
    }

    #[test]
    fn sha256_file_tolerates_bom_crlf_and_case() {
        let hex = "a3f16b5fdcbc23d9220e5b26e5893272610abb1ffb78d359fcae842df314cffd";
        assert_eq!(parse_sha256_file(&format!("{hex}  murmur-v0.4.4-setup.exe\n")).as_deref(), Some(hex));
        assert_eq!(parse_sha256_file(&format!("\u{feff}{}  x.exe\r\n", hex.to_uppercase())).as_deref(), Some(hex));
        assert_eq!(parse_sha256_file("abc123  x.exe"), None);
        assert_eq!(parse_sha256_file(&format!("{}zz  x.exe", &hex[..62])), None);
        assert_eq!(parse_sha256_file(""), None);
    }

    #[test]
    fn only_the_installed_copy_updates_itself() {
        let lad = Path::new(r"C:\Users\Jane Doe\AppData\Local");
        assert!(is_installed_copy(Path::new(r"C:\Users\Jane Doe\AppData\Local\Programs\Murmur\murmur.exe"), lad));
        assert!(is_installed_copy(Path::new(r"c:\users\jane doe\appdata\local\programs\murmur\MURMUR.EXE"), lad));
        assert!(!is_installed_copy(Path::new(r"C:\Users\Jane Doe\git\murmur\target\release\murmur.exe"), lad));
        assert!(!is_installed_copy(Path::new(r"C:\Users\Jane Doe\Downloads\murmur-v0.4.4-windows-x64\murmur.exe"), lad));
    }

    #[test]
    fn alerts_once_per_tag() {
        assert!(should_alert("0.4.5", None));
        assert!(should_alert("0.4.5", Some("0.4.4")));
        assert!(!should_alert("0.4.5", Some("0.4.5")));
        assert!(!should_alert("0.4.5", Some("0.4.5\r\n")));
    }

    #[test]
    fn offer_starts_once_until_it_fails() {
        let mut o = Offer::default();
        assert_eq!(o.start(), None);
        assert!(o.available(release("0.4.5")));
        assert_eq!(o.start(), Some(release("0.4.5")));
        assert_eq!(o.start(), None);
        o.failed();
        assert_eq!(o.start(), Some(release("0.4.5")));
    }

    #[test]
    fn offer_moves_only_to_a_newer_tag() {
        let mut o = Offer::default();
        assert!(o.available(release("0.4.5")));
        assert!(!o.available(release("0.4.5")));
        assert!(!o.available(release("0.4.4")));
        assert!(o.available(release("0.4.6")));
        assert_eq!(o.release().map(|r| r.tag.as_str()), Some("0.4.6"));
        // not while a download runs: the menu would change under it
        o.start();
        assert!(!o.available(release("0.4.7")));
        assert_eq!(o.release().map(|r| r.tag.as_str()), Some("0.4.6"));
    }
}
```

Add `pub mod update;` to `src/lib.rs` after `pub mod stt;` (keep the list alphabetical: it goes after `stt` and before `vad`).

- [ ] **Step 3: Run the tests to verify they fail**

Run: `CARGO test --release --locked --lib update`
Expected: compile errors, `cannot find type Release`, `cannot find function parse_release`, etc.

- [ ] **Step 4: Write the implementation**

Put this above the `#[cfg(test)]` block in `src/update.rs`:

```rust
use std::fs;
use std::path::{Path, PathBuf};

/// A GitHub release with a setup exe and its `.sha256`. `tag` has no leading "v".
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub tag: String,
    pub setup_url: String,
    pub setup_size: u64,
    pub sha_url: String,
}

/// The value of the first `"key":` at or after byte `from`: its start and its text (strings lose
/// their quotes). Good enough for GitHub's release JSON, whose values here hold no escaped quotes.
fn value_after(json: &str, key: &str, from: usize) -> Option<(usize, String)> {
    let pat = format!("\"{key}\"");
    let at = from + json.get(from..)?.find(&pat)? + pat.len();
    let rest = json[at..].trim_start().strip_prefix(':')?.trim_start();
    let start = json.len() - rest.len();
    let value = match rest.strip_prefix('"') {
        Some(s) => s[..s.find('"')?].to_string(),
        None => rest[..rest.find([',', '}', ']']).unwrap_or(rest.len())].trim_end().to_string(),
    };
    Some((start, value))
}

/// `tag_name` from a GitHub release as JSON, without its leading "v".
pub fn latest_tag(json: &str) -> Option<String> {
    Some(value_after(json, "tag_name", 0)?.1.trim_start_matches('v').to_string())
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    let parts = |v: &str| v.split('.').map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    parts(latest) > parts(current)
}

/// The size and download URL of the asset called `name`.
fn asset(json: &str, name: &str) -> Option<(u64, String)> {
    let mut from = 0;
    while let Some((at, value)) = value_after(json, "name", from) {
        if value == name {
            let (_, size) = value_after(json, "size", at)?;
            let (_, url) = value_after(json, "browser_download_url", at)?;
            return Some((size.parse().ok()?, url));
        }
        from = at;
    }
    None
}

/// The release and its setup assets; None if either the setup exe or its `.sha256` is missing.
pub fn parse_release(json: &str) -> Option<Release> {
    let tag = latest_tag(json)?;
    let setup = format!("murmur-v{tag}-setup.exe");
    let (setup_size, setup_url) = asset(json, &setup)?;
    let (_, sha_url) = asset(json, &format!("{setup}.sha256"))?;
    Some(Release { tag, setup_url, setup_size, sha_url })
}

/// The hash from a `sha256sum`-style line ("<hex>  <file>"), lowercased.
pub fn parse_sha256_file(text: &str) -> Option<String> {
    let hash = text.trim_start_matches('\u{feff}').split_whitespace().next()?.to_ascii_lowercase();
    (hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())).then_some(hash)
}

/// True for the copy the installer put in `<localappdata>\Programs\Murmur`. A zip copy or a dev
/// build only gets the alert: running the installer wouldn't update it.
pub fn is_installed_copy(exe: &Path, localappdata: &Path) -> bool {
    let installed = localappdata.join("Programs").join("Murmur").join("murmur.exe");
    exe.to_string_lossy().eq_ignore_ascii_case(&installed.to_string_lossy())
}

fn alerted_path() -> PathBuf {
    crate::config::config_dir().join("update-alerted")
}

pub fn should_alert(tag: &str, stored: Option<&str>) -> bool {
    stored.map(str::trim) != Some(tag)
}

/// The last version a balloon was shown for, so a restart doesn't repeat it.
pub fn read_alerted() -> Option<String> {
    fs::read_to_string(alerted_path()).ok()
}

pub fn write_alerted(tag: &str) {
    if let Err(e) = fs::write(alerted_path(), tag) {
        log::warn!("update-alerted: {e}");
    }
}

/// The release on offer in the tray, and whether its download is running.
#[derive(Default)]
pub struct Offer {
    release: Option<Release>,
    busy: bool,
}

impl Offer {
    /// Takes `r` if it's newer than what's on offer. True when the menu needs to change.
    /// Ignored while a download runs.
    pub fn available(&mut self, r: Release) -> bool {
        if self.busy || self.release.as_ref().is_some_and(|o| !is_newer(&r.tag, &o.tag)) {
            return false;
        }
        self.release = Some(r);
        true
    }

    pub fn release(&self) -> Option<&Release> {
        self.release.as_ref()
    }

    /// The release to download; None if nothing is on offer or a download is already running.
    pub fn start(&mut self) -> Option<Release> {
        if self.busy {
            return None;
        }
        let r = self.release.clone()?;
        self.busy = true;
        Some(r)
    }

    /// The download failed: the same release can be started again.
    pub fn failed(&mut self) {
        self.busy = false;
    }
}
```

In `src/about_ui.rs`:
- delete the `latest_tag` and `is_newer` functions and the tests `latest_tag_reads_the_release_json` and `versions_compare_by_number` (they now live in `update.rs`);
- add `use murmur_lib::update::{is_newer, latest_tag};` below the `crate::correction_ui` import.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `CARGO test --release --locked --lib update`
Expected: 10 passed.

Run: `CARGO test --release --locked --bin murmur about`
Expected: 2 passed (`model_label_names_parakeet_and_falls_back_to_the_folder`, `learned_counts_words`).

- [ ] **Step 6: Commit**

```bash
git add src/update.rs src/lib.rs src/about_ui.rs tests/fixtures/release-v0.4.4.json
git commit -m "feat(update): parse releases and track what's on offer

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: `update.rs`, check, download and install

**Files:**
- Modify: `src/update.rs`
- Modify: `src/about_ui.rs` (its `check` thread calls `update::check`; drop `LATEST_API`, and the `is_newer`/`latest_tag` import from Task 1)

**Interfaces:**
- Consumes: Task 1's `Release`, `parse_release`, `parse_sha256_file`, `is_newer`; `murmur_lib::model_fetch::{Asset, Fetcher, FetchError}` (`Fetcher::standard().download(&Asset, dir: &Path, progress: &mut dyn FnMut(u64), cancel: &AtomicBool) -> Result<PathBuf, FetchError>`, which only produces the final file after size and SHA-256 match).
- Produces (`pub` in `murmur_lib::update`):
  - `const VERSION: &str`, `const RELEASES_PAGE: &str = "https://github.com/jshowah-dev/murmur/releases/latest"`
  - `const INSTALL_ARGS: [&str; 5]`
  - `fn check_at(url: &str, current: &str) -> Result<Option<Release>, String>`, `fn check() -> Result<Option<Release>, String>`
  - `fn spawn_checker(found: impl Fn(Release) + Send + 'static)`
  - `fn download_dir() -> PathBuf`, `fn download(r: &Release, dir: &Path) -> Result<PathBuf, FetchError>`
  - `fn install(setup: &Path) -> std::io::Result<()>`
  - `fn failure_text(e: &FetchError) -> String`

- [ ] **Step 1: Write the failing tests**

Append inside `mod tests` in `src/update.rs`:

```rust
    use crate::model_fetch::{hex, FetchError};
    use sha2::{Digest, Sha256};
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    /// Serves each `(path, body)` with 200 and anything else with 404, on 127.0.0.1. Range headers
    /// are ignored, so the Fetcher takes its whole-file path.
    fn serve(routes: Vec<(&'static str, Vec<u8>)>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { return };
                let mut reader = BufReader::new(s.try_clone().unwrap());
                let (mut path, mut line) = (String::new(), String::new());
                loop {
                    line.clear();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim_end().is_empty() {
                        break;
                    }
                    if path.is_empty() {
                        path = line.split(' ').nth(1).unwrap_or("").to_string();
                    }
                }
                let body = routes.iter().find(|(p, _)| *p == path).map(|(_, b)| b.clone());
                let status = if body.is_some() { "200 OK" } else { "404 Not Found" };
                let body = body.unwrap_or_default();
                let _ = write!(s, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = s.write_all(&body);
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("murmur update {name} {}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn served_release(setup: &[u8], sha_line: String) -> Release {
        let base = serve(vec![("/setup.exe", setup.to_vec()), ("/setup.exe.sha256", sha_line.into_bytes())]);
        Release { tag: "9.9.9".into(), setup_url: format!("{base}/setup.exe"), setup_size: setup.len() as u64, sha_url: format!("{base}/setup.exe.sha256") }
    }

    #[test]
    fn check_at_reports_only_a_newer_release() {
        let base = serve(vec![("/latest", FIXTURE.as_bytes().to_vec())]);
        let url = format!("{base}/latest");
        assert_eq!(check_at(&url, "0.4.3").unwrap().map(|r| r.tag), Some("0.4.4".into()));
        assert_eq!(check_at(&url, "0.4.4").unwrap(), None);
        assert!(check_at(&format!("{base}/missing"), "0.4.3").is_err());
    }

    #[test]
    fn download_verifies_against_the_sha256_file() {
        let setup = b"pretend installer".repeat(1000);
        let r = served_release(&setup, format!("{}  murmur-v9.9.9-setup.exe\n", hex(Sha256::digest(&setup).as_slice())));
        let dir = tmp("ok");
        let got = download(&r, &dir).unwrap();
        assert_eq!(got, dir.join("murmur-v9.9.9-setup.exe"));
        assert_eq!(std::fs::read(&got).unwrap(), setup);
    }

    #[test]
    fn download_refuses_a_setup_that_does_not_match() {
        let setup = b"pretend installer".to_vec();
        let r = served_release(&setup, format!("{}  murmur-v9.9.9-setup.exe\n", hex(Sha256::digest(b"something else").as_slice())));
        let dir = tmp("mismatch");
        assert!(matches!(download(&r, &dir), Err(FetchError::ChecksumMismatch)));
        assert!(!dir.join("murmur-v9.9.9-setup.exe").exists());
    }

    #[test]
    fn download_refuses_an_unreadable_sha256_file() {
        let r = served_release(b"x", "not a hash".into());
        assert!(matches!(download(&r, &tmp("badsha")), Err(FetchError::ChecksumMismatch)));
    }

    #[test]
    fn failure_text_is_about_the_update() {
        for e in [FetchError::Http(404), FetchError::ChecksumMismatch, FetchError::Interrupted("timed out".into())] {
            let t = failure_text(&e);
            assert!(!t.contains("k2-fsa") && !t.contains("Model"), "{t}");
        }
        assert_eq!(failure_text(&FetchError::Http(404)), "GitHub answered HTTP 404");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `CARGO test --release --locked --lib update`
Expected: compile errors, `cannot find function check_at`, `download`, `failure_text`.

- [ ] **Step 3: Write the implementation**

Add to the imports at the top of `src/update.rs`:

```rust
use crate::model_fetch::{Asset, FetchError, Fetcher};
use std::sync::atomic::AtomicBool;
use std::time::Duration;
```

Add below the imports:

```rust
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const RELEASES_PAGE: &str = "https://github.com/jshowah-dev/murmur/releases/latest";
const LATEST_API: &str = "https://api.github.com/repos/jshowah-dev/murmur/releases/latest";
const FIRST_CHECK: Duration = Duration::from_secs(60);
const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// Inno Setup switches for an unattended update. `/RELAUNCH` is ours: `murmur.iss` starts Murmur
/// again after a successful install only when it's present.
pub const INSTALL_ARGS: [&str; 5] = ["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/FORCECLOSEAPPLICATIONS", "/RELAUNCH"];
```

Add below `Offer`'s impl:

```rust
fn latest_url() -> String {
    // lets a debug build test an update against a locally served release
    #[cfg(debug_assertions)]
    if let Ok(url) = std::env::var("MURMUR_UPDATE_URL") {
        return url;
    }
    LATEST_API.into()
}

fn get_text(url: &str) -> Result<String, ureq::Error> {
    let agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(8))).build().new_agent();
    agent.get(url).header("User-Agent", "murmur").call()?.body_mut().read_to_string()
}

/// The latest release at `url` if it's newer than `current`. Err covers any failure to ask or to
/// read the answer, including a release without a setup asset.
pub fn check_at(url: &str, current: &str) -> Result<Option<Release>, String> {
    let body = get_text(url).map_err(|e| e.to_string())?;
    let release = parse_release(&body).ok_or("no setup asset in the latest release")?;
    Ok(is_newer(&release.tag, current).then_some(release))
}

pub fn check() -> Result<Option<Release>, String> {
    check_at(&latest_url(), VERSION)
}

/// Checks a minute after launch, then daily, calling `found` for each newer release seen.
/// Failures are only logged: the next check retries.
pub fn spawn_checker(found: impl Fn(Release) + Send + 'static) {
    std::thread::spawn(move || {
        std::thread::sleep(FIRST_CHECK);
        loop {
            match check() {
                Ok(Some(r)) => found(r),
                Ok(None) => log::info!("update check: up to date"),
                Err(e) => log::info!("update check: {e}"),
            }
            std::thread::sleep(CHECK_EVERY);
        }
    });
}

pub fn download_dir() -> PathBuf {
    std::env::temp_dir().join("Murmur")
}

/// Downloads the setup exe into `dir`, checked against the release's `.sha256` file.
pub fn download(r: &Release, dir: &Path) -> Result<PathBuf, FetchError> {
    let text = get_text(&r.sha_url).map_err(|e| match e {
        ureq::Error::StatusCode(n) => FetchError::Http(n),
        e => FetchError::Interrupted(e.to_string()),
    })?;
    let sha256 = parse_sha256_file(&text).ok_or(FetchError::ChecksumMismatch)?;
    let asset = Asset { url: r.setup_url.clone(), file: format!("murmur-v{}-setup.exe", r.tag), sha256, size: r.setup_size };
    Fetcher::standard().download(&asset, dir, &mut |_| {}, &AtomicBool::new(false))
}

/// Starts the installer and returns; the caller quits Murmur so the files are free.
pub fn install(setup: &Path) -> std::io::Result<()> {
    std::process::Command::new(setup).args(INSTALL_ARGS).spawn().map(drop)
}

/// A failed update download, for a balloon. `FetchError`'s own text is about the model.
pub fn failure_text(e: &FetchError) -> String {
    match e {
        FetchError::Http(n) => format!("GitHub answered HTTP {n}"),
        FetchError::ChecksumMismatch => "The download didn't match its checksum".into(),
        FetchError::Interrupted(why) => format!("The download was interrupted ({why})"),
        other => other.to_string(),
    }
}
```

(If `get_text`'s chained `call()?.body_mut()` doesn't borrow-check, bind it: `let mut resp = agent.get(url).header("User-Agent", "murmur").call()?; resp.body_mut().read_to_string()`.)

In `src/about_ui.rs`:
- delete `const LATEST_API` and the `use murmur_lib::update::{is_newer, latest_tag};` line added in Task 1;
- replace the body of the spawned closure in `fn check(ctx)` with:

```rust
    std::thread::spawn(move || {
        let update = match murmur_lib::update::check() {
            Ok(Some(r)) => Update::Available(r.tag),
            Ok(None) => Update::UpToDate,
            Err(e) => {
                log::info!("update check: {e}");
                Update::Failed
            }
        };
        let _ = tx.send(update);
        ctx.request_repaint();
    });
```

- the `Duration` import in `about_ui.rs` is now unused; remove it (`use std::time::Duration;`).
- the "vX available →" link opens `murmur_lib::update::RELEASES_PAGE` instead of `format!("{REPO}/releases/latest")`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `CARGO test --release --locked --lib update`
Expected: 15 passed.

Run: `CARGO build --release --locked`
Expected: builds with no new warnings in `update.rs` or `about_ui.rs`.

- [ ] **Step 5: Commit**

```bash
git add src/update.rs src/about_ui.rs
git commit -m "feat(update): check, download and start the installer

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Installer relaunches on `/RELAUNCH`

**Files:**
- Modify: `installer/murmur.iss` (`[Run]` section, `[Code]` section)
- Modify: `src/update.rs` (one test)

**Interfaces:**
- Consumes: Task 2's `INSTALL_ARGS`.
- Produces: `murmur.iss` function `RelaunchRequested: Boolean` and a `[Run]` entry that uses it.

- [ ] **Step 1: Write the failing test**

Append inside `mod tests` in `src/update.rs`:

```rust
    #[test]
    fn installer_relaunches_only_when_asked() {
        let iss = include_str!("../installer/murmur.iss");
        let line = iss.lines().find(|l| l.contains("Check: RelaunchRequested")).expect("a [Run] entry for /RELAUNCH");
        assert!(line.contains(r#"Filename: "{app}\murmur.exe""#), "{line}");
        assert!(!line.contains("skipifsilent") && !line.contains("postinstall"), "{line}");
        assert!(iss.contains("CompareText(ParamStr(I), '/RELAUNCH') = 0"));
        assert!(INSTALL_ARGS.contains(&"/RELAUNCH"));
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `CARGO test --release --locked --lib installer_relaunches`
Expected: FAIL, "a [Run] entry for /RELAUNCH".

- [ ] **Step 3: Edit `installer/murmur.iss`**

In `[Run]`, add this line **above** the existing postinstall entry:

```
Filename: "{app}\murmur.exe"; Flags: nowait; Check: RelaunchRequested
```

In `[Code]`, after the `AutostartOnCommandLine` function, add:

```pascal
{ The in-app updater installs silently and passes /RELAUNCH, so Murmur comes back afterwards.
  A silent install without it (winget, scripts) still doesn't start Murmur. }
function RelaunchRequested: Boolean;
var
  I: Integer;
begin
  Result := False;
  for I := 1 to ParamCount do
    if CompareText(ParamStr(I), '/RELAUNCH') = 0 then
      Result := True;
end;
```

- [ ] **Step 4: Run the test, then compile the installer**

Run: `CARGO test --release --locked --lib installer_relaunches`
Expected: PASS.

Compile the script to catch Pascal errors (needs `target\release\murmur.exe` and the DLLs, which Task 2's release build produced):

```bash
cd /c/Users/JeffLocal/git/murmur && "$LOCALAPPDATA/Programs/Inno Setup 6/ISCC.exe" //Q //DAppVersion=0.0.0 installer/murmur.iss && ls murmur-v0.0.0-setup.exe && rm murmur-v0.0.0-setup.exe
```

Expected: no output from ISCC, then the file listed and removed. (Git Bash needs `//Q` and `//D…` so MSYS doesn't rewrite them as paths.)

- [ ] **Step 5: Commit**

```bash
git add installer/murmur.iss src/update.rs
git commit -m "feat(installer): relaunch Murmur after an in-app update

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Tray items and app-update wiring

**Files:**
- Modify: `src/tray.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: Task 1's `Offer`, `should_alert`, `read_alerted`, `write_alerted`, `is_installed_copy`, `Release`; Task 2's `spawn_checker`, `download_dir`, `download`, `install`, `failure_text`, `RELEASES_PAGE`.
- Produces:
  - `TrayEvent::Update` and `TrayEvent::DownloadModel` (Task 6 handles the second).
  - `Tray::set_update(&self, label: Option<&str>, enabled: bool)` and `Tray::set_model(&self, label: Option<&str>, enabled: bool)`: `None` takes the item out of the menu; `Some` sets its text and enabled state and puts it in, above "About Murmur" (model item first, then update item).
  - In `main.rs`: `enum UpdateMsg { Available(update::Release), Downloaded(PathBuf), Failed(String) }` (Task 6 adds model variants), received on `up_rx`; the main loop is labelled `'main`.

- [ ] **Step 1: Tray, write the failing test**

Menu handling can't be unit-tested without a message loop, so the test pins the pure part: where the optional items go. Add to `mod tests` in `src/tray.rs`:

```rust
    #[test]
    fn optional_items_sit_above_about() {
        // Pause, Fix last, History, ─, Dictionary, Snippets, Config, Start with Windows, ─, [model], [update], About, Quit
        assert_eq!(extras_positions(false, false), (None, None));
        assert_eq!(extras_positions(false, true), (None, Some(9)));
        assert_eq!(extras_positions(true, false), (Some(9), None));
        assert_eq!(extras_positions(true, true), (Some(9), Some(10)));
    }
```

Run: `CARGO test --release --locked --bin murmur optional_items`
Expected: compile error, `cannot find function extras_positions`.

- [ ] **Step 2: Tray, implement**

In `src/tray.rs`:

Add `use std::cell::Cell;`. Change the enum and struct:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    TogglePause,
    FixLast,
    History,
    EditDictionary,
    OpenSnippets,
    OpenConfigDir,
    ToggleAutostart,
    Update,
    DownloadModel,
    About,
    Quit,
}

pub struct Tray {
    _icon: TrayIcon,
    menu: Menu,
    pause: MenuItem,
    autostart: CheckMenuItem,
    update: MenuItem,
    model: MenuItem,
    /// whether (model, update) are in the menu
    shown: Cell<(bool, bool)>,
    ids: [(MenuId, TrayEvent); 11],
}

/// Menu position of "About Murmur" before any optional item is added.
const ABOUT_AT: usize = 9;

/// Where the model and update items go when shown: above About, model first.
fn extras_positions(model: bool, update: bool) -> (Option<usize>, Option<usize>) {
    let m = model.then_some(ABOUT_AT);
    let u = update.then_some(ABOUT_AT + model as usize);
    (m, u)
}
```

In `create()`: after `let quit = …`, add

```rust
        // not in the menu until there's something to offer
        let update = MenuItem::new("Update", true, None);
        let model = MenuItem::new("Download new speech model", true, None);
```

extend `ids` with `(update.id().clone(), TrayEvent::Update)` and `(model.id().clone(), TrayEvent::DownloadModel)` before the About entry; build the icon with `.with_menu(Box::new(menu.clone()))`; and return

```rust
        Ok(Tray { _icon, menu, pause, autostart, update, model, shown: Cell::new((false, false)), ids })
```

Add these methods to `impl Tray`:

```rust
    /// Shows the update item with `label`, or takes it out of the menu with None.
    pub fn set_update(&self, label: Option<&str>, enabled: bool) {
        let (model, _) = self.shown.get();
        self.show_extras(model, relabel(&self.update, label, enabled));
    }

    /// Shows the model-download item with `label`, or takes it out of the menu with None.
    pub fn set_model(&self, label: Option<&str>, enabled: bool) {
        let (_, update) = self.shown.get();
        self.show_extras(relabel(&self.model, label, enabled), update);
    }

    /// muda has no hidden items, so the optional ones are taken out and put back in order.
    fn show_extras(&self, model: bool, update: bool) {
        let (had_model, had_update) = self.shown.get();
        if had_model {
            let _ = self.menu.remove(&self.model);
        }
        if had_update {
            let _ = self.menu.remove(&self.update);
        }
        let (m, u) = extras_positions(model, update);
        if let Some(at) = m {
            let _ = self.menu.insert(&self.model, at);
        }
        if let Some(at) = u {
            let _ = self.menu.insert(&self.update, at);
        }
        self.shown.set((model, update));
    }
```

and this free function below `impl Tray`:

```rust
fn relabel(item: &MenuItem, label: Option<&str>, enabled: bool) -> bool {
    if let Some(text) = label {
        item.set_text(text);
        item.set_enabled(enabled);
    }
    label.is_some()
}
```

Don't run the tests yet: the bin won't compile until Step 3 adds the new `TrayEvent` arms to `main.rs`. Step 4 runs them.

- [ ] **Step 3: main.rs, wire the app update**

Add `update` to the `murmur_lib` import: `use murmur_lib::{audio, cleanup, config, dictionary, history, model_fetch, snippets, stt, update, vad};`, and `use std::path::PathBuf;`.

Above `fn main`, add:

```rust
/// Results from the update checker and download threads.
enum UpdateMsg {
    Available(update::Release),
    Downloaded(PathBuf),
    Failed(String),
}

fn update_label(tag: &str, installed: bool) -> String {
    if installed { format!("Update to v{tag}…") } else { format!("Get v{tag}…") }
}
```

After `let tray = Tray::create()?;` add:

```rust
    let (up_tx, up_rx) = unbounded::<UpdateMsg>();
    let checker_tx = up_tx.clone();
    update::spawn_checker(move |r| {
        let _ = checker_tx.send(UpdateMsg::Available(r));
    });
    let installed_copy = match (std::env::current_exe(), std::env::var_os("LOCALAPPDATA")) {
        (Ok(exe), Some(lad)) => update::is_installed_copy(&exe, std::path::Path::new(&lad)),
        _ => false,
    };
    let mut offer = update::Offer::default();
```

Label the main loop: `loop {` → `'main: loop {`, and change `TrayEvent::Quit => break,` to `TrayEvent::Quit => break 'main,`.

Add these arms to the `match ev` on tray events (before `TrayEvent::About`):

```rust
                TrayEvent::Update if !installed_copy => {
                    if offer.release().is_some() {
                        open_path(std::path::Path::new(update::RELEASES_PAGE));
                    }
                }
                TrayEvent::Update => {
                    if let Some(r) = offer.start() {
                        tray.set_update(Some("Downloading update…"), false);
                        let tx = up_tx.clone();
                        std::thread::spawn(move || {
                            let msg = match update::download(&r, &update::download_dir()) {
                                Ok(path) => UpdateMsg::Downloaded(path),
                                Err(e) => UpdateMsg::Failed(update::failure_text(&e)),
                            };
                            let _ = tx.send(msg);
                        });
                    }
                }
                TrayEvent::DownloadModel => {}
```

(`open_path` hands its argument to `explorer.exe`, which opens a URL in the default browser, the same way About's `open_url` does. `DownloadModel` gets its real body in Task 6.)

After the `while let Ok(m) = msg_rx.try_recv() { … }` block, add:

```rust
        while let Ok(m) = up_rx.try_recv() {
            match m {
                UpdateMsg::Available(r) => {
                    let tag = r.tag.clone();
                    if offer.available(r) {
                        log::info!("update available: v{tag}");
                        tray.set_update(Some(&update_label(&tag, installed_copy)), true);
                        if update::should_alert(&tag, update::read_alerted().as_deref()) {
                            tray.notify(&format!("Murmur v{tag} is available"), "Update from the tray menu.");
                            update::write_alerted(&tag);
                        }
                    }
                }
                UpdateMsg::Downloaded(path) => match update::install(&path) {
                    Ok(()) => {
                        log::info!("installing {}; quitting", path.display());
                        break 'main;
                    }
                    Err(e) => {
                        log::error!("start installer: {e}");
                        offer.failed();
                        restore_update_item(&tray, &offer, installed_copy);
                        tray.notify("Update failed", &format!("Couldn't start the installer: {e}"));
                    }
                },
                UpdateMsg::Failed(why) => {
                    log::error!("update download: {why}");
                    offer.failed();
                    restore_update_item(&tray, &offer, installed_copy);
                    tray.notify("Update failed", &why);
                }
            }
        }
```

Below `fn resting`, add:

```rust
fn restore_update_item(tray: &Tray, offer: &update::Offer, installed: bool) {
    if let Some(r) = offer.release() {
        tray.set_update(Some(&update_label(&r.tag, installed)), true);
    }
}
```

Add to `mod tests` in `main.rs`:

```rust
    #[test]
    fn update_label_depends_on_the_copy() {
        assert_eq!(update_label("0.4.5", true), "Update to v0.4.5…");
        assert_eq!(update_label("0.4.5", false), "Get v0.4.5…");
    }
```

- [ ] **Step 4: Run the tests and build**

Run: `CARGO test --release --locked --bin murmur`
Expected: all pass, including `optional_items_sit_above_about` and `update_label_depends_on_the_copy`.

Run: `CARGO clippy --release --locked --all-targets`
Expected: no warnings in `tray.rs`, `main.rs` or `update.rs` beyond the ones already on `main`, except `set_model` unused (used in Task 6).

- [ ] **Step 5: Commit**

```bash
git add src/tray.rs src/main.rs
git commit -m "feat(tray): offer and install updates from the tray

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: `config::resolve_model`

**Files:**
- Modify: `src/config.rs`

**Interfaces:**
- Consumes: `Config::default().model_dir`, `expand_env`.
- Produces (`pub` in `murmur_lib::config`):
  - `const PREVIOUS_DEFAULTS: &[&str] = &[];`
  - `enum ModelState { Current, Switched { old: PathBuf }, UpgradeAvailable }` deriving `Debug, PartialEq`
  - `fn resolve_model(cfg: &mut Config, installed: impl Fn(&Path) -> bool) -> ModelState`

Rules (from the spec, with one refinement): a `model_dir` that is neither the current default nor a previous default is `Current` and untouched. For a previous default:
- current default installed → `cfg.model_dir` = current default, `Switched { old }` (old = the expanded previous path);
- else old one installed → `UpgradeAvailable`, `cfg` unchanged;
- else neither installed → `cfg.model_dir` = current default and `Current`, so the first-run download (which only runs when `model_dir` is the default) fetches the current pin. This replaces the spec's "`default_dir` also counts a previous default" with the same outcome and no change in `main.rs`'s `default_dir` line.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `src/config.rs`:

```rust
    const OLD: &str = "%LOCALAPPDATA%\\Murmur\\models\\old-model";

    fn cfg_with(dir: &str) -> Config {
        Config { model_dir: dir.into(), ..Config::default() }
    }

    #[test]
    fn a_custom_or_current_model_dir_is_left_alone() {
        for dir in ["D:\\models\\mine", &Config::default().model_dir] {
            let mut c = cfg_with(dir);
            assert_eq!(resolve_with(&mut c, &[OLD], |_| true), ModelState::Current);
            assert_eq!(c.model_dir, dir);
        }
    }

    #[test]
    fn an_old_default_switches_once_the_new_model_is_installed() {
        let current = Config::default().model_dir_path();
        let mut c = cfg_with(OLD);
        let state = resolve_with(&mut c, &[OLD], |p| p == current.as_path());
        assert_eq!(state, ModelState::Switched { old: PathBuf::from(expand_env(OLD)) });
        assert_eq!(c.model_dir, Config::default().model_dir);
    }

    #[test]
    fn an_old_default_is_offered_the_new_model() {
        let old = PathBuf::from(expand_env(OLD));
        let mut c = cfg_with(OLD);
        assert_eq!(resolve_with(&mut c, &[OLD], |p| p == old.as_path()), ModelState::UpgradeAvailable);
        assert_eq!(c.model_dir, OLD);
    }

    #[test]
    fn an_old_default_that_is_gone_gets_the_new_default() {
        let mut c = cfg_with(OLD);
        assert_eq!(resolve_with(&mut c, &[OLD], |_| false), ModelState::Current);
        assert_eq!(c.model_dir, Config::default().model_dir);
    }

    #[test]
    fn no_previous_defaults_ship_yet() {
        assert!(PREVIOUS_DEFAULTS.is_empty());
    }
```

Run: `CARGO test --release --locked --lib config`
Expected: compile errors, `cannot find function resolve_with`, `ModelState`, `PREVIOUS_DEFAULTS`.

- [ ] **Step 2: Implement**

In `src/config.rs`, change `use std::path::PathBuf;` to `use std::path::{Path, PathBuf};` and add after `impl Default for Config`:

```rust
/// `model_dir` defaults of earlier releases, newest first. When a release pins a new model, the
/// default it replaces goes here, so installs still on it are offered the new one.
pub const PREVIOUS_DEFAULTS: &[&str] = &[];

#[derive(Debug, PartialEq)]
pub enum ModelState {
    Current,
    /// The new default model is installed; `old` is the previous default's folder, now unused.
    Switched { old: PathBuf },
    /// Still on a previous default; the current default can be downloaded.
    UpgradeAvailable,
}

/// Moves a config on a previous default model to the current default once that's installed.
/// Only `cfg` in memory changes: config.toml keeps its comments and hand edits.
pub fn resolve_model(cfg: &mut Config, installed: impl Fn(&Path) -> bool) -> ModelState {
    resolve_with(cfg, PREVIOUS_DEFAULTS, installed)
}

fn resolve_with(cfg: &mut Config, previous: &[&str], installed: impl Fn(&Path) -> bool) -> ModelState {
    if !previous.contains(&cfg.model_dir.as_str()) {
        return ModelState::Current;
    }
    let old = cfg.model_dir_path();
    let current = Config::default();
    if installed(&current.model_dir_path()) {
        cfg.model_dir = current.model_dir;
        ModelState::Switched { old }
    } else if installed(&old) {
        ModelState::UpgradeAvailable
    } else {
        cfg.model_dir = current.model_dir;
        ModelState::Current
    }
}
```

- [ ] **Step 3: Run the tests to verify they pass**

Run: `CARGO test --release --locked --lib config`
Expected: all config tests pass (the 5 new ones included).

- [ ] **Step 4: Commit**

```bash
git add src/config.rs
git commit -m "feat(config): move installs off a previous default model

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Model offer and download at startup

**Files:**
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: Task 5's `config::resolve_model`, `ModelState`; Task 4's `Tray::set_model`, `TrayEvent::DownloadModel`, `UpdateMsg`, `up_tx`, `'main`; `model_fetch::{Fetcher, parakeet, extract, is_installed}`.
- Produces: `UpdateMsg::{ModelProgress(u8), ModelUnpacking, ModelReady, ModelFailed(String)}`; `fn model_offer_label() -> String`; `fn percent(done: u64, total: u64) -> u8`.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `main.rs`:

```rust
    #[test]
    fn model_offer_names_the_download_size() {
        assert_eq!(model_offer_label(), "Download new speech model (482 MB)…");
    }

    #[test]
    fn percent_is_whole_and_capped() {
        assert_eq!(percent(0, 482_468_385), 0);
        assert_eq!(percent(241_234_192, 482_468_385), 49);
        assert_eq!(percent(482_468_385, 482_468_385), 100);
        assert_eq!(percent(10, 0), 100);
    }
```

Run: `CARGO test --release --locked --bin murmur -- model_offer percent_is`
Expected: compile errors, `cannot find function model_offer_label`, `percent`.

- [ ] **Step 2: Implement the helpers and messages**

Extend `UpdateMsg`:

```rust
enum UpdateMsg {
    Available(update::Release),
    Downloaded(PathBuf),
    Failed(String),
    ModelProgress(u8),
    ModelUnpacking,
    ModelReady,
    ModelFailed(String),
}
```

Add below `update_label`:

```rust
fn model_offer_label() -> String {
    format!("Download new speech model ({} MB)…", model_fetch::parakeet().size / 1_000_000)
}

fn percent(done: u64, total: u64) -> u8 {
    if total == 0 { 100 } else { (done.min(total) * 100 / total) as u8 }
}

/// Downloads and unpacks the current default model next to the old one, reporting on `tx`.
/// A cancelled run (Murmur quit) keeps the `.part`, and the next attempt resumes it.
fn download_model(models: PathBuf, tx: crossbeam_channel::Sender<UpdateMsg>) {
    let asset = model_fetch::parakeet();
    let mut last = u8::MAX;
    let result = model_fetch::Fetcher::standard()
        .download(&asset, &models, &mut |n| {
            let p = percent(n, asset.size);
            if p != last {
                last = p;
                let _ = tx.send(UpdateMsg::ModelProgress(p));
            }
        }, &std::sync::atomic::AtomicBool::new(false))
        .and_then(|archive| {
            let _ = tx.send(UpdateMsg::ModelUnpacking);
            model_fetch::extract(&archive, &models, &mut |_| {})
        });
    let _ = tx.send(match result {
        Ok(dir) => {
            log::info!("model upgrade: installed {}", dir.display());
            UpdateMsg::ModelReady
        }
        Err(e) => UpdateMsg::ModelFailed(e.to_string()),
    });
}
```

Run: `CARGO test --release --locked --bin murmur -- model_offer percent_is`
Expected: 2 passed.

- [ ] **Step 3: Wire startup, the tray event and the messages**

In `main()`:

1. `let cfg = match Config::load_or_create() {` → `let mut cfg = match …`.
2. Directly before `let model_dir = cfg.model_dir_path();`, add:

```rust
    let model_state = config::resolve_model(&mut cfg, model_fetch::is_installed);
    if let config::ModelState::Switched { old } = &model_state {
        // not loaded yet, so nothing holds its files open
        log::info!("model upgrade: now on {}; removing {}", cfg.model_dir, old.display());
        if let Err(e) = std::fs::remove_dir_all(old) {
            log::warn!("remove old model: {e}");
        }
    }
```

3. After the `for msg in &startup_errors { … }` loop (the tray exists by then), add:

```rust
    if model_state == config::ModelState::UpgradeAvailable {
        tray.notify(
            &format!("A new speech model is available ({} MB)", model_fetch::parakeet().size / 1_000_000),
            "Download from the tray menu.",
        );
        tray.set_model(Some(&model_offer_label()), true);
    }
```

4. Replace the placeholder arm `TrayEvent::DownloadModel => {}` with:

```rust
                TrayEvent::DownloadModel => {
                    tray.set_model(Some("Downloading speech model… 0%"), false);
                    let models = Config::default().model_dir_path().parent().map(PathBuf::from).unwrap_or_default();
                    let tx = up_tx.clone();
                    std::thread::spawn(move || download_model(models, tx));
                }
```

5. Add these arms to the `match m` in the `up_rx` loop:

```rust
                UpdateMsg::ModelProgress(p) => tray.set_model(Some(&format!("Downloading speech model… {p}%")), false),
                UpdateMsg::ModelUnpacking => tray.set_model(Some("Unpacking speech model…"), false),
                UpdateMsg::ModelReady => {
                    tray.set_model(None, false);
                    tray.notify("New speech model ready", "It's used from the next start.");
                }
                UpdateMsg::ModelFailed(why) => {
                    log::error!("model upgrade: {why}");
                    tray.set_model(Some(&model_offer_label()), true);
                    tray.notify("Speech model download failed", &why);
                }
```

The tray item only exists while `model_state` is `UpgradeAvailable`, and it's disabled while a download runs, so a second download can't start.

- [ ] **Step 4: Run all tests and clippy**

Run: `CARGO test --release --locked`
Expected: all pass (lib, bin, and the `stt_integration` test).

Run: `CARGO clippy --release --locked --all-targets`
Expected: no warnings in `main.rs`, `tray.rs`, `update.rs`, `config.rs` beyond those already on `main`.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs
git commit -m "feat(model): offer the current model to installs on an old default

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: End-to-end smoke of click-to-update (with Jeff)

This proves the full path, relaunch included, before any release. It changes Jeff's installed Murmur, so **confirm with Jeff before Step 3**, and restore v0.4.4 at the end.

**Files:** none committed. Working files go in the session scratchpad: set `SCRATCH` to its path (the system prompt names it) in each shell command below.

- [ ] **Step 1: Build a "newer" setup from this branch**

```bash
cd /c/Users/JeffLocal/git/murmur && CARGO build --release --locked \
 && "$LOCALAPPDATA/Programs/Inno Setup 6/ISCC.exe" //Q //DAppVersion=0.4.99 installer/murmur.iss \
 && mkdir -p "$SCRATCH/rel" && mv murmur-v0.4.99-setup.exe "$SCRATCH/rel/" \
 && (cd "$SCRATCH/rel" && sha256sum murmur-v0.4.99-setup.exe > murmur-v0.4.99-setup.exe.sha256 && stat -c %s murmur-v0.4.99-setup.exe)
```

Expected: the setup's size in bytes. The exe inside still reports 0.4.4 (`Cargo.toml` is not bumped); the smoke checks the installer ran by the Add/Remove Programs version (`AppVersion=0.4.99`) and the relaunch, not by `murmur.exe`'s version.

- [ ] **Step 2: Serve a release JSON for it**

Write `$SCRATCH/rel/latest` (replace `SIZE` with Step 1's number):

```json
{"tag_name":"v0.4.99","assets":[
 {"name":"murmur-v0.4.99-setup.exe","size":SIZE,"browser_download_url":"http://127.0.0.1:8765/murmur-v0.4.99-setup.exe"},
 {"name":"murmur-v0.4.99-setup.exe.sha256","size":90,"browser_download_url":"http://127.0.0.1:8765/murmur-v0.4.99-setup.exe.sha256"}]}
```

Don't start the server yet: Step 3 first checks a failed check with it down.

- [ ] **Step 3: Put a debug build in the installed location (Jeff confirms first)**

`MURMUR_UPDATE_URL` only works in debug builds.

```bash
CARGO build --locked
```

Quit the installed Murmur from its tray (Quit). Back up and replace:

```bash
I="$LOCALAPPDATA/Programs/Murmur"; cp "$I/murmur.exe" "$SCRATCH/murmur-0.4.4.exe" && cp target/debug/murmur.exe "$I/murmur.exe"
```

The DLLs in the install folder are the same sherpa-onnx/ORT versions (`Cargo.lock` unchanged), so the debug exe runs there.

**Negative check first (server down).** Start it with the override (Git Bash): `MURMUR_UPDATE_URL=http://127.0.0.1:8765/latest "$LOCALAPPDATA/Programs/Murmur/murmur.exe" &`. Wait 70 s. Expected: no balloon, no update item in the menu, and `murmur.log` (`%APPDATA%\Murmur\murmur.log`) has an `update check:` line with a connection error. Quit it from the tray.

Now start the server in the background: `cd "$SCRATCH/rel" && python -m http.server 8765 --bind 127.0.0.1`.
Check: `curl -s http://127.0.0.1:8765/latest | head -c 40` → `{"tag_name":"v0.4.99"`.

Start the debug build with the override again, the same way.

- [ ] **Step 4: Watch it update**

Expected, in order:
1. ~60 s after launch: a balloon "Murmur v0.4.99 is available", and `murmur.log` has `update available: v0.4.99`.
2. Tray menu (in the ^ overflow) shows "Update to v0.4.99…" above "About Murmur".
3. Click it: the item reads "Downloading update…" (disabled); the server log shows GETs for the `.sha256` then the setup exe; `murmur.log` has `installing …murmur-v0.4.99-setup.exe; quitting`.
4. Within ~15 s, Murmur is running again from the install folder: `powershell -NoProfile -c "Get-Process murmur | Select Path, StartTime"` shows a start time after the click.
5. `reg query "HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\{3859BC9B-2892-4D3F-8616-9C7EB9B7AD57}_is1" //v DisplayVersion` → `0.4.99`.
6. `reg query "HKCU\Software\Microsoft\Windows\CurrentVersion\Run" //v Murmur` → still present (autostart kept).
7. Jeff dictates once: text lands.
8. `cat "$APPDATA/Murmur/update-alerted"` → `0.4.99`. (The relaunched copy is a release build, which ignores `MURMUR_UPDATE_URL`, so the no-repeat balloon is covered by `alerts_once_per_tag`, not here.)

- [ ] **Step 5: Restore Jeff's machine**

Quit Murmur. Download and run the published v0.4.4 setup, verifying its SHA first:

```bash
cd "$SCRATCH" && gh release download v0.4.4 -R jshowah-dev/murmur -p 'murmur-v0.4.4-setup.exe*' --clobber && sha256sum -c murmur-v0.4.4-setup.exe.sha256 && ./murmur-v0.4.4-setup.exe //VERYSILENT //SUPPRESSMSGBOXES //NORESTART
```

Expected: `murmur-v0.4.4-setup.exe: OK`, then the Uninstall key's `DisplayVersion` is `0.4.4`. Start Murmur (Start menu), delete `%APPDATA%\Murmur\update-alerted`, stop the http server.

Record the smoke results (each numbered expectation: seen or not) for the handoff; nothing to commit.

---

## Out of scope (from the spec)

Choosing or shipping a new model, code signing, a settings window, the version bump and release, Dependabot.
