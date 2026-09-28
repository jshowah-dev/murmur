//! First-run model download: fetches pinned assets and unpacks the model. No UI; `setup_ui` drives it.

use sha2::{Digest, Sha256};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

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

/// Bytes the Parakeet archive unpacks to, for the unpack progress bar.
pub const PARAKEET_UNPACKED: u64 = 661_428_477;

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

pub fn tar_exe() -> PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    PathBuf::from(root).join("System32").join("tar.exe")
}

/// Unpacks a .tar.bz2 holding one top-level folder into `dir` with Windows' own tar.exe, via
/// `dir\.staging` so a crash mid-unpack leaves nothing that looks installed. The archive is
/// deleted only on success. `progress` gets the bytes unpacked so far, polled while tar runs.
pub fn extract(archive: &Path, dir: &Path, progress: &mut dyn FnMut(u64)) -> Result<PathBuf, FetchError> {
    let staging = dir.join(".staging");
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    fs::create_dir_all(&staging)?;
    let result = untar(archive, &staging, progress).and_then(|()| move_single_dir(&staging, dir));
    let _ = fs::remove_dir_all(&staging);
    let unpacked = result?;
    fs::remove_file(archive)?;
    Ok(unpacked)
}

fn untar(archive: &Path, into: &Path, progress: &mut dyn FnMut(u64)) -> Result<(), FetchError> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let tar = tar_exe();
    let mut child = std::process::Command::new(&tar)
        .arg("-xjf")
        .arg(archive)
        .arg("-C")
        .arg(into)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| FetchError::Unpack(format!("{}: {e}", tar.display())))?;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        progress(tree_size(into));
        std::thread::sleep(Duration::from_millis(250));
    };
    if !status.success() {
        return Err(FetchError::Unpack(format!("tar exit {}", status.code().unwrap_or(-1))));
    }
    progress(tree_size(into));
    Ok(())
}

/// Total bytes of the files under `dir` (0 if missing). Sizes come from `fs::metadata` on each
/// path: a directory listing's cached size reads 0 for a file tar is still writing.
pub fn tree_size(dir: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else { return 0 };
    entries
        .filter_map(|e| e.ok())
        .map(|e| {
            let path = e.path();
            match fs::metadata(&path) {
                Ok(m) if m.is_dir() => tree_size(&path),
                Ok(m) => m.len(),
                Err(_) => 0,
            }
        })
        .sum()
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

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::fs;
    use std::io::{BufRead, BufReader, Write};
    use std::net::{TcpListener, TcpStream};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

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
        let got = extract(&archive, &dir, &mut |_| {}).unwrap();
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
        let got = extract(&archive, &dir, &mut |_| {}).unwrap();
        assert!(is_installed(&got));
        assert!(!got.join("stale.txt").exists());
    }

    #[test]
    fn tree_size_counts_nested_files() {
        let dir = tmp("tree size");
        fs::create_dir_all(dir.join("a").join("b")).unwrap();
        fs::write(dir.join("top.bin"), vec![0u8; 10]).unwrap();
        fs::write(dir.join("a").join("b").join("deep.bin"), vec![0u8; 32]).unwrap();
        assert_eq!(tree_size(&dir), 42);
        assert_eq!(tree_size(&dir.join("missing")), 0);
    }

    #[test]
    fn extract_reports_unpacked_bytes() {
        let dir = tmp("extract progress");
        let archive = make_archive(&dir);
        let mut last = None;
        extract(&archive, &dir, &mut |n| last = Some(n)).unwrap();
        assert_eq!(last, Some(1), "the fixture's only file is 1 byte");
    }

    #[test]
    fn bad_archive_is_kept() {
        let dir = tmp("extract bad");
        let archive = dir.join("broken.tar.bz2");
        fs::write(&archive, b"not an archive").unwrap();
        let r = extract(&archive, &dir, &mut |_| {});
        assert!(matches!(r, Err(FetchError::Unpack(_))), "{r:?}");
        assert!(archive.exists());
        assert!(!dir.join(".staging").exists());
    }
}
