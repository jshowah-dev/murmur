# Murmur Packaging Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A fresh Windows user unzips a GitHub Release, runs `murmur.exe`, and gets a window that downloads, verifies and unpacks the speech model, then Murmur starts normally.

**Architecture:** A new UI-free library module, `model_fetch`, downloads pinned assets with resumable, SHA-256-verified HTTP Range requests (ureq) and unpacks them with Windows' own `tar.exe`. A new egui window, `setup_ui`, drives it from a worker thread before `main` starts the pipeline. A GitHub Actions workflow builds, tests and zips the exe + DLLs; tag runs create a draft Release.

**Tech Stack:** Rust 2021, `ureq` 3.4 (rustls, no gzip), `sha2` 0.11, eframe 0.36 (already a dependency), `%SystemRoot%\System32\tar.exe`, GitHub Actions on `windows-latest`.

**Spec:** `docs/superpowers/specs/2026-09-27-murmur-packaging-design.md`

**Deviation from the spec, found while verifying `ureq` at the source (for Jeff to confirm in plan review).** The spec says "30 s read stall" timeout. `ureq` 3.4 has no per-read timeout: its only body timeout is `timeout_recv_body`, a total budget per request (`ureq-3.4.2/src/config.rs:813`). So the download runs as **8 MB Range requests, each with a 120 s budget**, which fails any request slower than about 0.55 Mbit/s. A stall then surfaces as "Download interrupted" within 2 minutes, and Retry resumes. The spec is amended to match in Task 2.

## Global Constraints

- Model asset URL base: `https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/`
- `silero_vad.onnx`: 643,854 bytes, SHA-256 `9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6`
- `sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8.tar.bz2`: 482,468,385 bytes, SHA-256 `157c157bc51155e03e37d2466522a3a737dd9c72bb25f36eb18912964161e1ad`
- Everything downloads into the **parent** of `Config::model_dir_path()` (default `%LOCALAPPDATA%\Murmur\models`). The VAD file sits there (`Config::vad_model_path`) and the model folder is unpacked there.
- Auto-download only when `cfg.model_dir == Config::default().model_dir`. With a custom `model_dir`, show only the tray notice.
- Timeouts: 15 s connect, 30 s to receive response headers, 120 s per 8 MB chunk body.
- Error text: "Download interrupted (…)", "Model file not available at k2-fsa (HTTP n)", "Download corrupted; Retry starts it over", "Couldn't unpack the model (tar exit n)". IO errors pass the OS message through.
- No user data in logs. Log start, finish and errors at INFO/ERROR.
- Release zip: `murmur-vX.Y.Z-windows-x64.zip` holds `murmur.exe`, `onnxruntime.dll`, `onnxruntime_providers_shared.dll`, `sherpa-onnx-c-api.dll`, `sherpa-onnx-cxx-api.dll`, `README.md`, `LICENSE`, plus a `.zip.sha256` alongside.
- Tag runs create a **draft** Release. Never publish, push or merge without Jeff's explicit go.
- Test baseline to keep green: `--bin murmur` 31, `--lib` 48, `stt_integration` 1. Run them with the 4 DLLs in `target/release/deps` and Murmur stopped (`Stop-Process -Name murmur`).
- Work on branch `feat/packaging`, cut from `main` at the commit that adds this plan.

## Review Focus

1. **GitHub redirects every asset URL (302 → objects.githubusercontent.com).** The `Range` header must survive the redirect, or every resume silently becomes a full re-download. Live check: `curl -r 100-199 -L <vad url>` → `206 100 redirects=1`. Pinned by `redirect_keeps_range_header` (Task 2).
2. **A server answers 206 with an empty body.** Must fail as "interrupted", not loop forever re-requesting the same range. Pinned by `empty_partial_response_is_interrupted` (Task 2).
3. **A Windows user name with spaces** (`C:\Users\Jane Doe\AppData\Local\…`) must work for both the download paths and the `tar.exe` arguments. All Task 2 and Task 3 tests use temp dirs containing spaces.
4. **A leftover model folder without `encoder.int8.onnx`** (from a half-finished `setup-model.cmd` run) must be replaced by extraction, not block the rename. Pinned by `extract_replaces_partial_model_dir` (Task 3).
5. **The archive is complete but extraction failed last time.** The next launch must skip the 480 MB download and go straight to unpacking. Pinned by `existing_target_is_returned_without_request` (Task 2) + `bad_archive_is_kept` (Task 3).

---

### Task 0: Branch

- [ ] **Step 1: Create the branch**

```bash
cd C:/Users/JeffLocal/git/murmur && git switch -c feat/packaging main && git log --oneline -1
```

Expected: `docs: packaging implementation plan` at HEAD.

---

### Task 1: `model_fetch` core types

**Files:**
- Modify: `Cargo.toml` (add deps), `src/lib.rs` (add module)
- Create: `src/model_fetch.rs`

**Interfaces:**
- Produces: `pub struct Asset { pub url: String, pub file: String, pub sha256: String, pub size: u64 }`, `pub fn vad() -> Asset`, `pub fn parakeet() -> Asset`, `pub enum FetchError { Interrupted(String), Http(u16), ChecksumMismatch, Cancelled, Unpack(String), Io(std::io::Error) }` (implements `Display`, `std::error::Error`, `From<std::io::Error>`), `pub fn hex(bytes: &[u8]) -> String`, `pub fn is_installed(model_dir: &Path) -> bool`.

- [ ] **Step 1: Add dependencies**

In `Cargo.toml` `[dependencies]`, after `simplelog = "0.12"`:

```toml
ureq = { version = "3.4", default-features = false, features = ["rustls"] }
sha2 = "0.11"
```

In `src/lib.rs`, add after `pub mod history;`:

```rust
pub mod model_fetch;
```

- [ ] **Step 2: Write the failing tests**

Create `src/model_fetch.rs` containing only the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::fs;
    use std::path::PathBuf;

    fn tmp(name: &str) -> PathBuf {
        // a space in the path, like "C:\Users\Jane Doe"
        let d = std::env::temp_dir().join(format!("murmur fetch {name} {}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn hex_is_lowercase_sha256() {
        assert_eq!(
            hex(Sha256::digest(b"abc").as_slice()),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn pinned_assets_match_spec() {
        let v = vad();
        assert_eq!(v.url, "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx");
        assert_eq!(v.size, 643_854);
        assert_eq!(v.sha256, "9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6");
        let p = parakeet();
        assert_eq!(p.file, "sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8.tar.bz2");
        assert_eq!(p.size, 482_468_385);
        assert_eq!(p.sha256, "157c157bc51155e03e37d2466522a3a737dd9c72bb25f36eb18912964161e1ad");
    }

    #[test]
    fn installed_only_with_encoder() {
        let d = tmp("installed");
        assert!(!is_installed(&d));
        fs::write(d.join("encoder.int8.onnx"), b"x").unwrap();
        assert!(is_installed(&d));
    }

    #[test]
    fn error_messages_match_spec() {
        assert_eq!(FetchError::Http(404).to_string(), "Model file not available at k2-fsa (HTTP 404)");
        assert_eq!(FetchError::ChecksumMismatch.to_string(), "Download corrupted; Retry starts it over");
        assert_eq!(FetchError::Unpack("tar exit 2".into()).to_string(), "Couldn't unpack the model (tar exit 2)");
        assert!(FetchError::Interrupted("timed out".into()).to_string().starts_with("Download interrupted"));
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --release --lib model_fetch`
Expected: compile errors, `cannot find function 'hex'` / `vad` / `is_installed` / type `FetchError`.

- [ ] **Step 4: Implement**

Put this above the test module in `src/model_fetch.rs`:

```rust
//! First-run model download: fetches pinned assets and unpacks the model. No UI; `setup_ui` drives it.

use std::fmt;
use std::path::Path;

const BASE: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/";

/// A file to download, pinned by size and SHA-256 (lowercase hex).
pub struct Asset {
    pub url: String,
    pub file: String,
    pub sha256: String,
    pub size: u64,
}

pub fn vad() -> Asset {
    Asset {
        url: format!("{BASE}silero_vad.onnx"),
        file: "silero_vad.onnx".into(),
        sha256: "9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6".into(),
        size: 643_854,
    }
}

pub fn parakeet() -> Asset {
    let file = "sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8.tar.bz2";
    Asset {
        url: format!("{BASE}{file}"),
        file: file.into(),
        sha256: "157c157bc51155e03e37d2466522a3a737dd9c72bb25f36eb18912964161e1ad".into(),
        size: 482_468_385,
    }
}

#[derive(Debug)]
pub enum FetchError {
    Interrupted(String),
    Http(u16),
    ChecksumMismatch,
    Cancelled,
    Unpack(String),
    Io(std::io::Error),
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FetchError::Interrupted(e) => write!(f, "Download interrupted ({e})"),
            FetchError::Http(n) => write!(f, "Model file not available at k2-fsa (HTTP {n})"),
            FetchError::ChecksumMismatch => write!(f, "Download corrupted; Retry starts it over"),
            FetchError::Cancelled => write!(f, "Cancelled"),
            FetchError::Unpack(e) => write!(f, "Couldn't unpack the model ({e})"),
            FetchError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for FetchError {}

impl From<std::io::Error> for FetchError {
    fn from(e: std::io::Error) -> Self {
        FetchError::Io(e)
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The encoder only appears after the final rename in `extract`, so a half-unpacked model never counts.
pub fn is_installed(model_dir: &Path) -> bool {
    model_dir.join("encoder.int8.onnx").is_file()
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --release --lib model_fetch`
Expected: `4 passed`. `ureq` is unused so far; a `dead_code`/unused-crate warning is fine.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock src/lib.rs src/model_fetch.rs
git commit -m "feat(model_fetch): pinned model assets, error type, install check"
```

---

### Task 2: Resumable, verified download

**Files:**
- Modify: `src/model_fetch.rs`
- Modify: `docs/superpowers/specs/2026-09-27-murmur-packaging-design.md` (timeout amendment)

**Interfaces:**
- Consumes: `Asset`, `FetchError`, `hex` (Task 1).
- Produces: `pub struct Fetcher`, `Fetcher::new(chunk: u64, per_chunk: Duration) -> Fetcher`, `Fetcher::standard() -> Fetcher` (8 MB / 120 s), `Fetcher::download(&self, asset: &Asset, dir: &Path, progress: &mut dyn FnMut(u64), cancel: &AtomicBool) -> Result<PathBuf, FetchError>`. `progress` receives the total bytes of this asset on disk so far.

- [ ] **Step 1: Write the failing tests**

Append inside `mod tests` in `src/model_fetch.rs`. Add `use std::io::{BufRead, BufReader, Write};`, `use std::net::{TcpListener, TcpStream};`, `use std::sync::atomic::{AtomicBool, Ordering};`, `use std::sync::{Arc, Mutex};` and `use std::time::Duration;` to the test module's imports.

```rust
    #[derive(Clone, Copy, PartialEq)]
    enum Mode {
        Range,
        IgnoreRange,
        NotFound,
        Stall,
        Empty206,
        RedirectThenRange,
    }

    type Seen = Arc<Mutex<Vec<(String, Option<(usize, usize)>)>>>;

    /// Serves `data` on 127.0.0.1, one connection at a time; records (path, range) per request.
    fn serve(data: Vec<u8>, mode: Mode) -> (String, Seen) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen: Seen = Arc::default();
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { return };
                let mut reader = BufReader::new(s.try_clone().unwrap());
                let (mut path, mut range, mut line) = (String::new(), None, String::new());
                loop {
                    line.clear();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
                    }
                    let l = line.trim_end().to_ascii_lowercase();
                    if l.is_empty() {
                        break;
                    }
                    if path.is_empty() {
                        path = l.split(' ').nth(1).unwrap_or("").to_string();
                    }
                    if let Some(v) = l.strip_prefix("range: bytes=") {
                        let (a, b) = v.split_once('-').unwrap();
                        range = Some((a.parse().unwrap(), b.parse().unwrap()));
                    }
                }
                log.lock().unwrap().push((path.clone(), range));
                let _ = respond(&mut s, &data, mode, &path, range, port);
            }
        });
        (format!("http://127.0.0.1:{port}/file"), seen)
    }

    fn respond(
        s: &mut TcpStream,
        data: &[u8],
        mode: Mode,
        path: &str,
        range: Option<(usize, usize)>,
        port: u16,
    ) -> std::io::Result<()> {
        let head = |s: &mut TcpStream, status: &str, len: usize| {
            write!(s, "HTTP/1.1 {status}\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n")
        };
        match mode {
            Mode::NotFound => head(s, "404 Not Found", 0),
            Mode::Empty206 => head(s, "206 Partial Content", 0),
            Mode::Stall => {
                head(s, "206 Partial Content", data.len())?;
                s.write_all(&data[..10])?;
                std::thread::sleep(Duration::from_secs(5));
                Ok(())
            }
            Mode::RedirectThenRange if path == "/file" => write!(
                s,
                "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{port}/real\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            ),
            Mode::IgnoreRange => {
                head(s, "200 OK", data.len())?;
                s.write_all(data)
            }
            _ => match range {
                Some((a, b)) => {
                    let b = b.min(data.len() - 1);
                    head(s, "206 Partial Content", b - a + 1)?;
                    s.write_all(&data[a..=b])
                }
                None => {
                    head(s, "200 OK", data.len())?;
                    s.write_all(data)
                }
            },
        }
    }

    fn data(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i * 7 % 251) as u8).collect()
    }

    fn asset_for(url: &str, data: &[u8]) -> Asset {
        Asset { url: url.into(), file: "blob.bin".into(), sha256: hex(Sha256::digest(data).as_slice()), size: data.len() as u64 }
    }

    fn fetcher() -> Fetcher {
        Fetcher::new(1000, Duration::from_secs(5))
    }

    fn ranges(seen: &Seen) -> Vec<Option<(usize, usize)>> {
        seen.lock().unwrap().iter().map(|(_, r)| *r).collect()
    }

    #[test]
    fn fresh_download_in_chunks() {
        let d = data(2500);
        let (url, seen) = serve(d.clone(), Mode::Range);
        let dir = tmp("fresh");
        let mut last = 0;
        let got = fetcher().download(&asset_for(&url, &d), &dir, &mut |n| last = n, &AtomicBool::new(false)).unwrap();
        assert_eq!(fs::read(&got).unwrap(), d);
        assert!(!dir.join("blob.bin.part").exists());
        assert_eq!(last, 2500);
        assert_eq!(ranges(&seen), vec![Some((0, 999)), Some((1000, 1999)), Some((2000, 2499))]);
    }

    #[test]
    fn resumes_from_part() {
        let d = data(2500);
        let (url, seen) = serve(d.clone(), Mode::Range);
        let dir = tmp("resume");
        fs::write(dir.join("blob.bin.part"), &d[..1200]).unwrap();
        let got = fetcher().download(&asset_for(&url, &d), &dir, &mut |_| {}, &AtomicBool::new(false)).unwrap();
        assert_eq!(fs::read(&got).unwrap(), d);
        assert_eq!(ranges(&seen)[0], Some((1200, 2199)));
    }

    #[test]
    fn server_ignoring_range_restarts_from_zero() {
        let d = data(2500);
        let (url, seen) = serve(d.clone(), Mode::IgnoreRange);
        let dir = tmp("ignore range");
        fs::write(dir.join("blob.bin.part"), &d[..1200]).unwrap();
        let got = fetcher().download(&asset_for(&url, &d), &dir, &mut |_| {}, &AtomicBool::new(false)).unwrap();
        assert_eq!(fs::read(&got).unwrap(), d);
        assert_eq!(ranges(&seen), vec![Some((1200, 2199)), None]);
    }

    #[test]
    fn checksum_mismatch_deletes_part() {
        let d = data(2500);
        let (url, _) = serve(d.clone(), Mode::Range);
        let dir = tmp("mismatch");
        let mut asset = asset_for(&url, &d);
        asset.sha256 = "0".repeat(64);
        let r = fetcher().download(&asset, &dir, &mut |_| {}, &AtomicBool::new(false));
        assert!(matches!(r, Err(FetchError::ChecksumMismatch)));
        assert!(!dir.join("blob.bin.part").exists());
        assert!(!dir.join("blob.bin").exists());
    }

    #[test]
    fn oversize_part_restarts() {
        let d = data(2500);
        let (url, seen) = serve(d.clone(), Mode::Range);
        let dir = tmp("oversize");
        fs::write(dir.join("blob.bin.part"), vec![0u8; 3000]).unwrap();
        let got = fetcher().download(&asset_for(&url, &d), &dir, &mut |_| {}, &AtomicBool::new(false)).unwrap();
        assert_eq!(fs::read(&got).unwrap(), d);
        assert_eq!(ranges(&seen)[0], Some((0, 999)));
    }

    #[test]
    fn cancel_keeps_part() {
        let d = data(2500);
        let (url, _) = serve(d.clone(), Mode::Range);
        let dir = tmp("cancel");
        let cancel = AtomicBool::new(false);
        let r = fetcher().download(
            &asset_for(&url, &d),
            &dir,
            &mut |n| {
                if n >= 1000 {
                    cancel.store(true, Ordering::SeqCst)
                }
            },
            &cancel,
        );
        assert!(matches!(r, Err(FetchError::Cancelled)));
        let kept = fs::metadata(dir.join("blob.bin.part")).unwrap().len();
        assert!((1000..2500).contains(&kept), "kept {kept}");
        assert!(!dir.join("blob.bin").exists());
    }

    #[test]
    fn not_found_is_http_error() {
        let d = data(2500);
        let (url, _) = serve(d.clone(), Mode::NotFound);
        let r = fetcher().download(&asset_for(&url, &d), &tmp("404"), &mut |_| {}, &AtomicBool::new(false));
        assert!(matches!(r, Err(FetchError::Http(404))));
    }

    #[test]
    fn stall_is_interrupted_and_keeps_part() {
        let d = data(1000);
        let (url, _) = serve(d.clone(), Mode::Stall);
        let dir = tmp("stall");
        let r = Fetcher::new(1000, Duration::from_secs(1)).download(&asset_for(&url, &d), &dir, &mut |_| {}, &AtomicBool::new(false));
        assert!(matches!(r, Err(FetchError::Interrupted(_))), "{r:?}");
        assert_eq!(fs::metadata(dir.join("blob.bin.part")).unwrap().len(), 10);
    }

    #[test]
    fn empty_partial_response_is_interrupted() {
        let d = data(2500);
        let (url, seen) = serve(d.clone(), Mode::Empty206);
        let r = fetcher().download(&asset_for(&url, &d), &tmp("empty"), &mut |_| {}, &AtomicBool::new(false));
        assert!(matches!(r, Err(FetchError::Interrupted(_))), "{r:?}");
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    #[test]
    fn redirect_keeps_range_header() {
        let d = data(2500);
        let (url, seen) = serve(d.clone(), Mode::RedirectThenRange);
        let dir = tmp("redirect");
        fs::write(dir.join("blob.bin.part"), &d[..1200]).unwrap();
        let got = fetcher().download(&asset_for(&url, &d), &dir, &mut |_| {}, &AtomicBool::new(false)).unwrap();
        assert_eq!(fs::read(&got).unwrap(), d);
        let seen = seen.lock().unwrap();
        assert_eq!(seen[1], ("/real".to_string(), Some((1200, 2199))));
    }

    #[test]
    fn existing_target_is_returned_without_request() {
        let d = data(2500);
        let (url, seen) = serve(d.clone(), Mode::Range);
        let dir = tmp("existing");
        fs::write(dir.join("blob.bin"), &d).unwrap();
        let mut last = 0;
        fetcher().download(&asset_for(&url, &d), &dir, &mut |n| last = n, &AtomicBool::new(false)).unwrap();
        assert!(seen.lock().unwrap().is_empty());
        assert_eq!(last, 2500);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --release --lib model_fetch`
Expected: compile error `cannot find type 'Fetcher'`.

- [ ] **Step 3: Implement**

In `src/model_fetch.rs`, change the imports at the top to:

```rust
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
```

Add below `is_installed`:

```rust
/// Downloads in Range requests of `chunk` bytes. ureq only offers a total body timeout per
/// request, so chunking is what turns a stalled connection into an error within `per_chunk`.
pub struct Fetcher {
    agent: ureq::Agent,
    whole: ureq::Agent,
    chunk: u64,
}

impl Fetcher {
    pub fn new(chunk: u64, per_chunk: Duration) -> Fetcher {
        let agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            .timeout_recv_body(Some(per_chunk))
            .build()
            .into();
        // for servers that ignore Range and send the whole file in one response
        let whole = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            .build()
            .into();
        Fetcher { agent, whole, chunk }
    }

    /// 8 MB per request, 120 s each: anything slower than ~0.55 Mbit/s counts as stalled.
    pub fn standard() -> Fetcher {
        Fetcher::new(8 * 1024 * 1024, Duration::from_secs(120))
    }

    /// Downloads `asset` into `dir` via `<file>.part`, resuming a previous `.part`. The final
    /// file only appears once size and SHA-256 match. Cancel and network errors keep the `.part`.
    pub fn download(
        &self,
        asset: &Asset,
        dir: &Path,
        progress: &mut dyn FnMut(u64),
        cancel: &AtomicBool,
    ) -> Result<PathBuf, FetchError> {
        let target = dir.join(&asset.file);
        if target.is_file() {
            progress(asset.size);
            return Ok(target);
        }
        fs::create_dir_all(dir)?;
        let part = dir.join(format!("{}.part", asset.file));
        let mut have = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        if have > asset.size {
            fs::remove_file(&part)?;
            have = 0;
        }
        let mut hasher = Sha256::new();
        if have > 0 {
            hash_file(&part, &mut hasher)?;
        }
        let mut out = OpenOptions::new().create(true).append(true).open(&part)?;
        progress(have);
        while have < asset.size {
            if cancel.load(Ordering::Relaxed) {
                return Err(FetchError::Cancelled);
            }
            let end = (have + self.chunk).min(asset.size) - 1;
            let resp = self.agent.get(&asset.url).header("Range", format!("bytes={have}-{end}")).call().map_err(from_ureq)?;
            match resp.status().as_u16() {
                206 => {}
                200 => {
                    drop(out);
                    return self.download_whole(asset, &part, &target, progress, cancel);
                }
                n => return Err(FetchError::Http(n)),
            }
            let before = have;
            have = copy(resp.into_body().into_reader(), &mut out, &mut hasher, have, progress, cancel)?;
            if have == before {
                return Err(FetchError::Interrupted("server sent no data".into()));
            }
        }
        drop(out);
        finish(&part, &target, hasher, asset)
    }

    fn download_whole(
        &self,
        asset: &Asset,
        part: &Path,
        target: &Path,
        progress: &mut dyn FnMut(u64),
        cancel: &AtomicBool,
    ) -> Result<PathBuf, FetchError> {
        let resp = self.whole.get(&asset.url).call().map_err(from_ureq)?;
        let mut out = File::create(part)?;
        let mut hasher = Sha256::new();
        progress(0);
        copy(resp.into_body().into_reader(), &mut out, &mut hasher, 0, progress, cancel)?;
        drop(out);
        finish(part, target, hasher, asset)
    }
}

fn from_ureq(e: ureq::Error) -> FetchError {
    match e {
        ureq::Error::StatusCode(n) => FetchError::Http(n),
        other => FetchError::Interrupted(other.to_string()),
    }
}

/// Streams `body` onto `out`, hashing as it goes. Returns the new byte count.
fn copy(
    mut body: impl Read,
    out: &mut File,
    hasher: &mut Sha256,
    mut have: u64,
    progress: &mut dyn FnMut(u64),
    cancel: &AtomicBool,
) -> Result<u64, FetchError> {
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = body.read(&mut buf).map_err(|e| FetchError::Interrupted(e.to_string()))?;
        if n == 0 {
            return Ok(have);
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(FetchError::Cancelled);
        }
        out.write_all(&buf[..n])?;
        hasher.update(&buf[..n]);
        have += n as u64;
        progress(have);
    }
}

fn hash_file(path: &Path, hasher: &mut Sha256) -> std::io::Result<()> {
    let mut f = File::open(path)?;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            return Ok(());
        }
        hasher.update(&buf[..n]);
    }
}

fn finish(part: &Path, target: &Path, hasher: Sha256, asset: &Asset) -> Result<PathBuf, FetchError> {
    let len = fs::metadata(part)?.len();
    if len != asset.size || hex(hasher.finalize().as_slice()) != asset.sha256 {
        fs::remove_file(part)?;
        return Err(FetchError::ChecksumMismatch);
    }
    fs::rename(part, target)?;
    Ok(target.to_path_buf())
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --release --lib model_fetch`
Expected: `15 passed` (4 from Task 1 + 11 new). `stall_is_interrupted_and_keeps_part` takes about 1 s.

If `redirect_keeps_range_header` fails because `seen[1]` has range `None`, ureq dropped the header on the redirect. **Stop and report to Jeff**; don't work around it silently. Real GitHub downloads would then restart from zero on every resume.

- [ ] **Step 5: Amend the spec's timeout line**

In `docs/superpowers/specs/2026-09-27-murmur-packaging-design.md`, replace the line `  - Timeouts: 15 s connect, 30 s read stall.` with:

```markdown
  - Timeouts: 15 s connect, 30 s for response headers. The body is fetched in 8 MB Range requests with a 120 s budget each; ureq 3.4 has no per-read timeout (only `timeout_recv_body`, a total per request), so chunking is what turns a stall into "Download interrupted". A server that answers a Range request with 200 is read in one response without a body timeout.
```

and in the error table replace `| Network drop / timeout |` with `| Network drop / stall (chunk over 120 s) |`.

- [ ] **Step 6: Run the full lib suite**

Run: `cargo test --release --lib`
Expected: `63 passed` (48 existing + 15).

- [ ] **Step 7: Commit**

```bash
git add src/model_fetch.rs docs/superpowers/specs/2026-09-27-murmur-packaging-design.md
git commit -m "feat(model_fetch): chunked, resumable, SHA-256-verified download"
```

---

### Task 3: Extraction via `tar.exe`

**Files:**
- Modify: `src/model_fetch.rs`

**Interfaces:**
- Consumes: `FetchError` (Task 1).
- Produces: `pub fn extract(archive: &Path, dir: &Path) -> Result<PathBuf, FetchError>`, which returns `dir/<the archive's single top-level folder>` and deletes the archive on success. Also `pub fn tar_exe() -> PathBuf`.

- [ ] **Step 1: Write the failing tests**

Append inside `mod tests`:

```rust
    fn make_archive(dir: &Path) -> PathBuf {
        let src = dir.join("src tree");
        fs::create_dir_all(src.join("fixture-model")).unwrap();
        fs::write(src.join("fixture-model").join("encoder.int8.onnx"), b"x").unwrap();
        let archive = dir.join("fixture-model.tar.bz2");
        let st = std::process::Command::new(tar_exe())
            .arg("-cjf")
            .arg(&archive)
            .arg("-C")
            .arg(&src)
            .arg("fixture-model")
            .status()
            .unwrap();
        assert!(st.success());
        fs::remove_dir_all(&src).unwrap();
        archive
    }

    #[test]
    fn extract_moves_folder_and_deletes_archive() {
        let dir = tmp("extract ok");
        let archive = make_archive(&dir);
        let got = extract(&archive, &dir).unwrap();
        assert_eq!(got, dir.join("fixture-model"));
        assert!(is_installed(&got));
        assert!(!archive.exists());
        assert!(!dir.join(".staging").exists());
    }

    #[test]
    fn extract_replaces_partial_model_dir() {
        let dir = tmp("extract partial");
        let archive = make_archive(&dir);
        fs::create_dir_all(dir.join("fixture-model")).unwrap();
        fs::write(dir.join("fixture-model").join("stale.txt"), b"old").unwrap();
        fs::create_dir_all(dir.join(".staging").join("junk")).unwrap();
        let got = extract(&archive, &dir).unwrap();
        assert!(is_installed(&got));
        assert!(!got.join("stale.txt").exists());
    }

    #[test]
    fn bad_archive_is_kept() {
        let dir = tmp("extract bad");
        let archive = dir.join("broken.tar.bz2");
        fs::write(&archive, b"not an archive").unwrap();
        let r = extract(&archive, &dir);
        assert!(matches!(r, Err(FetchError::Unpack(_))), "{r:?}");
        assert!(archive.exists());
        assert!(!dir.join(".staging").exists());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --release --lib model_fetch`
Expected: compile error `cannot find function 'tar_exe'` / `extract`.

- [ ] **Step 3: Implement**

Add to `src/model_fetch.rs` below `finish`:

```rust
pub fn tar_exe() -> PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    PathBuf::from(root).join("System32").join("tar.exe")
}

/// Unpacks a .tar.bz2 holding one top-level folder into `dir` with Windows' own tar.exe, via
/// `dir\.staging` so a crash mid-unpack leaves nothing that looks installed. The archive is
/// deleted only on success.
pub fn extract(archive: &Path, dir: &Path) -> Result<PathBuf, FetchError> {
    let staging = dir.join(".staging");
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    fs::create_dir_all(&staging)?;
    let result = untar(archive, &staging).and_then(|()| move_single_dir(&staging, dir));
    let _ = fs::remove_dir_all(&staging);
    let unpacked = result?;
    fs::remove_file(archive)?;
    Ok(unpacked)
}

fn untar(archive: &Path, into: &Path) -> Result<(), FetchError> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let tar = tar_exe();
    let status = std::process::Command::new(&tar)
        .arg("-xjf")
        .arg(archive)
        .arg("-C")
        .arg(into)
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| FetchError::Unpack(format!("{}: {e}", tar.display())))?;
    if !status.success() {
        return Err(FetchError::Unpack(format!("tar exit {}", status.code().unwrap_or(-1))));
    }
    Ok(())
}

fn move_single_dir(staging: &Path, dir: &Path) -> Result<PathBuf, FetchError> {
    let mut dirs = fs::read_dir(staging)?.filter_map(|e| e.ok()).filter(|e| e.path().is_dir());
    let (Some(only), None) = (dirs.next(), dirs.next()) else {
        return Err(FetchError::Unpack("archive must hold exactly one folder".into()));
    };
    let dest = dir.join(only.file_name());
    if dest.exists() {
        fs::remove_dir_all(&dest)?;
    }
    fs::rename(only.path(), &dest)?;
    Ok(dest)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --release --lib model_fetch`
Expected: `18 passed`.

- [ ] **Step 5: Commit**

```bash
git add src/model_fetch.rs
git commit -m "feat(model_fetch): unpack model via System32 tar.exe through a staging dir"
```

---

### Task 4: Setup window + startup wiring

**Files:**
- Create: `src/setup_ui.rs`
- Modify: `src/main.rs:3` (imports), `src/main.rs:4-12` (mod list), `src/main.rs` between the `snippets::ensure_file` block (ends line 90) and `let mut history` (line 91), `src/main.rs:118-122` (model check)
- Modify: `src/stt.rs:14` (error text)
- Delete: `setup-model.cmd`

**Interfaces:**
- Consumes: `model_fetch::{vad, parakeet, Fetcher, FetchError, extract, is_installed}` (Tasks 1–3); `crate::correction_ui::{load_system_font, MUTED, TEXT}` (`pub(crate)`, `src/correction_ui.rs:22-23,224`); `Tray::notify(&self, title: &str, body: &str)` (`src/tray.rs:79`).
- Produces: `pub enum SetupOutcome { Installed, Quit }`, `pub fn run(models: PathBuf) -> SetupOutcome`, where `models` is the folder to download into (the parent of the model dir).

- [ ] **Step 1: Create `src/setup_ui.rs`**

```rust
use crate::correction_ui::{load_system_font, MUTED, TEXT};
use crossbeam_channel::{Receiver, Sender};
use eframe::egui::{self, Margin, ViewportCommand};
use murmur_lib::model_fetch::{self, FetchError, Fetcher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const MB: u64 = 1024 * 1024;
const TICK: Duration = Duration::from_millis(250);

pub enum SetupOutcome {
    Installed,
    Quit,
}

enum Msg {
    Progress(u64),
    Unpacking,
    Done,
    Failed(String),
}

enum Stage {
    Downloading(u64),
    Unpacking,
    Failed(String),
}

struct SetupApp {
    models: PathBuf,
    total: u64,
    stage: Stage,
    rx: Receiver<Msg>,
    cancel: Arc<AtomicBool>,
    installed: Arc<AtomicBool>,
}

impl SetupApp {
    /// Runs one download + unpack attempt on a worker thread; Retry calls this again and resumes.
    fn start(&mut self, ctx: &egui::Context) {
        let (tx, rx) = crossbeam_channel::unbounded();
        self.rx = rx;
        self.stage = Stage::Downloading(0);
        let models = self.models.clone();
        let cancel = self.cancel.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let msg = match install(&models, &cancel, &tx, &ctx) {
                Ok(()) => Msg::Done,
                Err(FetchError::Cancelled) => return,
                Err(e) => {
                    log::error!("model setup: {e}");
                    Msg::Failed(e.to_string())
                }
            };
            let _ = tx.send(msg);
            ctx.request_repaint();
        });
    }
}

fn install(models: &Path, cancel: &AtomicBool, tx: &Sender<Msg>, ctx: &egui::Context) -> Result<(), FetchError> {
    let fetcher = Fetcher::standard();
    let (vad, parakeet) = (model_fetch::vad(), model_fetch::parakeet());
    let mut last = Instant::now() - TICK;
    let mut report = |done: u64| {
        if last.elapsed() >= TICK {
            last = Instant::now();
            let _ = tx.send(Msg::Progress(done));
            ctx.request_repaint();
        }
    };
    log::info!("model setup: downloading into {}", models.display());
    fetcher.download(&vad, models, &mut |n| report(n), cancel)?;
    fetcher.download(&parakeet, models, &mut |n| report(vad.size + n), cancel)?;
    let _ = tx.send(Msg::Unpacking);
    ctx.request_repaint();
    let dir = model_fetch::extract(&models.join(&parakeet.file), models)?;
    log::info!("model setup: installed {}", dir.display());
    Ok(())
}

impl eframe::App for SetupApp {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Progress(n) => self.stage = Stage::Downloading(n),
                Msg::Unpacking => self.stage = Stage::Unpacking,
                Msg::Failed(e) => self.stage = Stage::Failed(e),
                Msg::Done => {
                    self.installed.store(true, Ordering::SeqCst);
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                    return;
                }
            }
        }
        // the title bar's X counts as Cancel; the .part stays for next launch
        if ctx.input(|i| i.viewport().close_requested()) {
            self.cancel.store(true, Ordering::SeqCst);
        }
        let mut retry = false;
        egui::Frame::new().inner_margin(Margin::same(18)).show(ui, |ui| {
            ui.label(egui::RichText::new("Murmur needs its speech model (about 460 MB) before first use.").size(13.0).color(MUTED));
            ui.add_space(12.0);
            match &self.stage {
                Stage::Downloading(n) => {
                    ui.label(
                        egui::RichText::new(format!("Downloading speech model: {} / {} MB", n / MB, self.total / MB))
                            .size(15.0)
                            .color(TEXT),
                    );
                    ui.add_space(6.0);
                    ui.add(egui::ProgressBar::new(*n as f32 / self.total as f32));
                    ui.add_space(12.0);
                    if ui.button("Cancel").clicked() {
                        self.cancel.store(true, Ordering::SeqCst);
                        ctx.send_viewport_cmd(ViewportCommand::Close);
                    }
                }
                Stage::Unpacking => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(egui::RichText::new("Unpacking…").size(15.0).color(TEXT));
                    });
                }
                Stage::Failed(e) => {
                    ui.label(egui::RichText::new(e).size(15.0).color(TEXT));
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        retry = ui.button("Retry").clicked();
                        if ui.button("Quit").clicked() {
                            ctx.send_viewport_cmd(ViewportCommand::Close);
                        }
                    });
                }
            }
        });
        if retry {
            self.start(&ctx);
        }
    }
}

/// First-run model download. Blocks until the model is installed or the user cancels or quits.
pub fn run(models: PathBuf) -> SetupOutcome {
    let installed = Arc::new(AtomicBool::new(false));
    let done = installed.clone();
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Murmur setup")
            .with_resizable(false)
            .with_inner_size([480.0, 170.0]),
        centered: true,
        ..Default::default()
    };
    let r = eframe::run_native(
        "murmur-setup",
        opts,
        Box::new(move |cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            load_system_font(&cc.egui_ctx);
            let (_, rx) = crossbeam_channel::unbounded();
            let mut app = SetupApp {
                models,
                total: model_fetch::vad().size + model_fetch::parakeet().size,
                stage: Stage::Downloading(0),
                rx,
                cancel: Arc::new(AtomicBool::new(false)),
                installed: done,
            };
            app.start(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    );
    if let Err(e) = r {
        log::error!("setup window: {e}");
    }
    if installed.load(Ordering::SeqCst) {
        SetupOutcome::Installed
    } else {
        SetupOutcome::Quit
    }
}
```

- [ ] **Step 2: Wire it into `src/main.rs`**

Line 3: add `model_fetch` to the lib import:

```rust
use murmur_lib::{audio, cleanup, config, dictionary, history, model_fetch, snippets, stt, vad};
```

In the `mod` list (lines 4–12), add after `mod pipeline;`:

```rust
mod setup_ui;
```

Insert right after the `snippets::ensure_file` block (after line 90, before `let mut history = History::new(25);`):

```rust
    // Before anything else starts: the pipeline loads the model as soon as it's spawned.
    let model_dir = cfg.model_dir_path();
    let mut model_missing = !model_fetch::is_installed(&model_dir);
    if model_missing && cfg.model_dir == Config::default().model_dir {
        let models = model_dir.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| model_dir.clone());
        match setup_ui::run(models) {
            setup_ui::SetupOutcome::Installed => model_missing = false,
            setup_ui::SetupOutcome::Quit => {
                log::info!("model setup not finished; exiting");
                return Ok(());
            }
        }
    }
```

Replace the old check (originally lines 118–122):

```rust
    let encoder = cfg.model_dir_path().join("encoder.int8.onnx");
    if let Err(e) = std::fs::metadata(&encoder) {
        log::error!("model check failed for {}: {e} (LOCALAPPDATA={:?})", encoder.display(), std::env::var("LOCALAPPDATA"));
        tray.notify("Model missing", "run setup-model.cmd");
    }
```

with:

```rust
    // only reachable with a custom model_dir: the default one is set up above
    if model_missing {
        log::error!("model missing at {} (LOCALAPPDATA={:?})", model_dir.display(), std::env::var("LOCALAPPDATA"));
        tray.notify("Model missing", &format!("Model missing at {}", model_dir.display()));
    }
```

- [ ] **Step 3: Update `src/stt.rs:14` and delete the script**

In `src/stt.rs` line 14, change `"model file missing: {} (run setup-model.cmd)"` to `"model file missing: {}"`.

```bash
git rm setup-model.cmd
git grep -n "setup-model" -- src
```

Expected: no output.

- [ ] **Step 4: Build and run the baseline**

```bash
powershell -NoProfile -Command "Stop-Process -Name murmur -ErrorAction SilentlyContinue"
cargo build --release
cargo test --release --bin murmur
cargo test --release --lib
cargo test --release --test stt_integration -- --nocapture
```

Expected: builds with no warnings from new code; `31 passed`, `66 passed` (48 + 18), `1 passed`, and no "skipping" line.

- [ ] **Step 5: Manual smoke (Jeff runs the app; Claude drives the files)**

Because of AppData virtualization, every file move under `%LOCALAPPDATA%\Murmur` is done by a `.cmd` written to `C:\Users\Public` and run with `explorer.exe` (see the Facts in the 2026-09-27 packaging handoff). Delete each script afterwards.

1. **Fresh download.** Script: `move "%LOCALAPPDATA%\Murmur\models" "%LOCALAPPDATA%\Murmur\models.bak"`. Launch `target\release\murmur.exe` via explorer. Expected: the "Murmur setup" window shows MB climbing to ~460, then "Unpacking…", then closes. The tray icon appears, and a dictation works.
2. **Resume.** Move `models` aside again and launch. At about 100 MB, kill it: `powershell -NoProfile -Command "Stop-Process -Name murmur"`. Relaunch. Expected: the counter starts near 100 MB, not 0. `murmur.log` (copied out via a script) shows no error, and `models\*.part` is gone after completion.
3. **Cancel.** Move `models` aside, launch, and click Cancel at a few MB. Expected: Murmur exits (no tray icon), and `models\sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8.tar.bz2.part` exists.
4. **Corrupt.** With the `.part` from 3, script: `echo garbage>> "%LOCALAPPDATA%\Murmur\models\sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8.tar.bz2.part"`. That makes the file's bytes wrong at the end. Launch and let it finish. Expected: "Download corrupted; Retry starts it over". Retry downloads from 0 and succeeds.
5. **Restore.** Script: `rmdir /s /q "%LOCALAPPDATA%\Murmur\models"` then `move "%LOCALAPPDATA%\Murmur\models.bak" "%LOCALAPPDATA%\Murmur\models"`. Launch. Expected: no setup window, and dictation works.

Record each result (pass/fail + what was seen) for the handoff.

- [ ] **Step 6: Commit**

```bash
git add src/setup_ui.rs src/main.rs src/stt.rs
git commit -m "feat: first-run model download window replaces setup-model.cmd"
```

---

### Task 5: Release workflow

**Files:**
- Create: `.github/workflows/release.yml`

**Interfaces:**
- Consumes: `Cargo.toml` `version = "…"` (line 3), `Cargo.lock` (committed), and the 4 DLLs that `sherpa-onnx-sys` places in `target/release`.
- Produces: the dispatch artifact `murmur-vX.Y.Z-windows-x64` (zip + `.sha256`); on a tag, a draft Release with the same two files.

- [ ] **Step 1: Write the workflow**

```yaml
name: release

on:
  push:
    tags: ['v*']
  workflow_dispatch:

permissions:
  contents: write

jobs:
  build:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2

      - name: Read version
        id: ver
        shell: pwsh
        run: |
          $v = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
          "version=$v" >> $env:GITHUB_OUTPUT
          if ('${{ github.ref_type }}' -eq 'tag' -and "v$v" -ne '${{ github.ref_name }}') {
            throw "tag ${{ github.ref_name }} does not match Cargo.toml version $v"
          }

      # sherpa-onnx-sys downloads its prebuilt DLLs into target/release during this build
      - name: Build
        run: cargo build --release --locked

      - name: Test
        shell: pwsh
        run: |
          # the ONNX tests crash on System32's older onnxruntime.dll unless the bundled DLLs sit next to the test binaries
          Copy-Item target/release/*.dll target/release/deps/
          cargo test --release --locked --bin murmur
          if ($LASTEXITCODE) { exit $LASTEXITCODE }
          cargo test --release --locked --lib
          if ($LASTEXITCODE) { exit $LASTEXITCODE }
          # no model on CI: this prints "model not installed; skipping" and proves nothing here
          cargo test --release --locked --test stt_integration -- --nocapture
          if ($LASTEXITCODE) { exit $LASTEXITCODE }

      - name: Package
        id: pkg
        shell: pwsh
        run: |
          $name = "murmur-v${{ steps.ver.outputs.version }}-windows-x64"
          $files = @('murmur.exe', 'onnxruntime.dll', 'onnxruntime_providers_shared.dll', 'sherpa-onnx-c-api.dll', 'sherpa-onnx-cxx-api.dll') |
            ForEach-Object { "target/release/$_" }
          foreach ($doc in 'README.md', 'LICENSE') {
            if (Test-Path $doc) { $files += $doc }
            elseif ('${{ github.ref_type }}' -eq 'tag') { throw "$doc is required for a release" }
          }
          Compress-Archive -Path $files -DestinationPath "$name.zip"
          $hash = (Get-FileHash "$name.zip" -Algorithm SHA256).Hash.ToLower()
          "$hash  $name.zip" | Out-File -Encoding ascii "$name.zip.sha256"
          "name=$name" >> $env:GITHUB_OUTPUT

      - name: Upload artifact
        if: github.ref_type != 'tag'
        uses: actions/upload-artifact@v4
        with:
          name: ${{ steps.pkg.outputs.name }}
          path: |
            ${{ steps.pkg.outputs.name }}.zip
            ${{ steps.pkg.outputs.name }}.zip.sha256

      - name: Draft release
        if: github.ref_type == 'tag'
        env:
          GH_TOKEN: ${{ github.token }}
        run: gh release create "${{ github.ref_name }}" --draft --title "Murmur ${{ github.ref_name }}" "${{ steps.pkg.outputs.name }}.zip" "${{ steps.pkg.outputs.name }}.zip.sha256"
```

- [ ] **Step 2: Dry-run the Package step locally**

It can't run Actions locally, but the PowerShell package logic can run. From the repo root, in `pwsh` or `powershell`:

```powershell
$env:GITHUB_OUTPUT = "$env:TEMP\gh_out.txt"; Remove-Item $env:GITHUB_OUTPUT -ErrorAction SilentlyContinue
$name = "murmur-v0.1.0-windows-x64"
$files = @('murmur.exe','onnxruntime.dll','onnxruntime_providers_shared.dll','sherpa-onnx-c-api.dll','sherpa-onnx-cxx-api.dll') | ForEach-Object { "target/release/$_" }
Compress-Archive -Path $files -DestinationPath "$env:TEMP\$name.zip" -Force
(Get-ChildItem "$env:TEMP\$name.zip").Length; Add-Type -A System.IO.Compression.FileSystem; [IO.Compression.ZipFile]::OpenRead("$env:TEMP\$name.zip").Entries.Name
Remove-Item "$env:TEMP\$name.zip"
```

Expected: a size in the tens of MB, and 5 entry names (`murmur.exe` + 4 DLLs) at the zip root, with no `target/release/` prefix.

- [ ] **Step 3: Validate the YAML parses**

Run: `python -c "import yaml,sys; yaml.safe_load(open('.github/workflows/release.yml')); print('ok')"`
Expected: `ok`. If PyYAML is missing, skip this step and note that; Task 6's run is the real check.

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "ci: release workflow (build, test, zip; draft Release on tag)"
```

---

### Task 6: CI dry run — **STOP: needs Jeff's go to merge and push**

`workflow_dispatch` only works once the workflow file is on the default branch, so this means merging `feat/packaging` into `main` and pushing to the private `jshowah-dev/murmur`.

- [ ] **Step 1: Ask Jeff** to merge `feat/packaging` → `main` and push. Wait for an explicit yes.
- [ ] **Step 2: Merge and push** (only after the yes)

```bash
git switch main && git merge --ff-only feat/packaging
git -c credential.helper= -c 'credential.helper=!gh auth git-credential' push origin main
```

- [ ] **Step 3: Run the workflow**

```bash
gh workflow run release.yml --ref main -R jshowah-dev/murmur
gh run list --workflow release.yml -R jshowah-dev/murmur --limit 1
```

Don't poll in a loop. Check once after about 12 minutes with `gh run view <id> -R jshowah-dev/murmur`.

Expected: success. The Test step log shows `31 passed`, `66 passed`, and "model not installed; skipping". An artifact `murmur-v0.1.0-windows-x64` exists.

- [ ] **Step 4: Download the artifact** to the scratchpad for Task 7:

```bash
gh run download <id> -R jshowah-dev/murmur -D <scratchpad>/ci-artifact
```

---

### Task 7: Clean-machine proof in Windows Sandbox — **Jeff runs it**

Windows Sandbox is an optional Windows feature, and turning it on is a system setting, so Jeff enables it himself ("Turn Windows features on or off" → Windows Sandbox, then reboot).

- [ ] **Step 1:** Jeff copies `murmur-v0.1.0-windows-x64.zip` into the Sandbox, unzips it and runs `murmur.exe`.
- [ ] **Step 2: Expected:** the setup window appears (the Sandbox has no model), downloads, unpacks and closes. Holding the push-to-talk key and dictating into Notepad inserts text.
- [ ] **Step 3: If it fails with "VCRUNTIME140.dll was not found"** (or similar) → **stop.** This is the spec's open item. Bring it to Jeff as a decision: static CRT for the Rust exe (`-C target-feature=+crt-static` in `.cargo/config.toml`) and/or shipping the VC++ runtime DLLs in the zip. Don't pick one unasked.
- [ ] **Step 4:** Record the result in a new handoff (`personal-vault/01-Projects/Murmur/`). Next up after #4 is #3 README + LICENSE, whose first step is verifying the Parakeet model licence at the source.
