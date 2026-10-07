//! Update checks against the latest GitHub release, and the parts of click-to-update that don't need a UI.

use crate::model_fetch::{Asset, FetchError, Fetcher};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const RELEASES_PAGE: &str = "https://github.com/jshowah-dev/murmur/releases/latest";
const LATEST_API: &str = "https://api.github.com/repos/jshowah-dev/murmur/releases/latest";
const FIRST_CHECK: Duration = Duration::from_secs(60);
// one request an hour is well inside GitHub's 60 an hour for unauthenticated callers
const CHECK_EVERY: Duration = Duration::from_secs(60 * 60);
const TICK: Duration = Duration::from_secs(15 * 60);

/// Inno Setup switches for an unattended update. `/RELAUNCH` is ours: `murmur.iss` starts Murmur
/// again after a successful install only when it's present.
#[cfg_attr(not(windows), allow(dead_code))]
pub const INSTALL_ARGS: [&str; 5] = ["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/FORCECLOSEAPPLICATIONS", "/RELAUNCH"];

/// A GitHub release with this platform's installer (the setup exe on Windows, the DMG on a Mac)
/// and its `.sha256`. `tag` has no leading "v".
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
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

/// This platform's installer in release `tag`, as `release.yml` names it.
pub fn installer_name(tag: &str) -> String {
    #[cfg(windows)]
    return windows_installer(tag);
    #[cfg(target_os = "macos")]
    return mac_installer(tag, std::env::consts::ARCH);
}

#[cfg_attr(not(windows), allow(dead_code))]
fn windows_installer(tag: &str) -> String {
    format!("murmur-v{tag}-setup.exe")
}

/// `dmg.sh` names the DMG after `uname -m`, which calls Apple silicon arm64, not aarch64.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn mac_installer(tag: &str, arch: &str) -> String {
    let arch = if arch == "aarch64" { "arm64" } else { arch };
    format!("murmur-v{tag}-macos-{arch}.dmg")
}

/// The release and this platform's installer; None if the installer or its `.sha256` is missing.
pub fn parse_release(json: &str) -> Option<Release> {
    parse_release_as(json, installer_name)
}

fn parse_release_as(json: &str, installer: impl Fn(&str) -> String) -> Option<Release> {
    let tag = latest_tag(json)?;
    let setup = installer(&tag);
    let (setup_size, setup_url) = asset(json, &setup)?;
    let (_, sha_url) = asset(json, &format!("{setup}.sha256"))?;
    Some(Release { tag, setup_url, setup_size, sha_url })
}

/// The hash from a `sha256sum`-style line ("<hex>  <file>"), lowercased.
pub fn parse_sha256_file(text: &str) -> Option<String> {
    let hash = text.trim_start_matches('\u{feff}').split_whitespace().next()?.to_ascii_lowercase();
    (hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())).then_some(hash)
}

/// Whether this is the copy an update would replace. Any other copy only gets the alert: running
/// the installer wouldn't update it, and a dev build must never replace itself.
pub fn installed_copy() -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    #[cfg(windows)]
    return std::env::var_os("LOCALAPPDATA").is_some_and(|lad| is_installed_copy(&exe, Path::new(&lad)));
    #[cfg(target_os = "macos")]
    return is_installed_bundle(&exe);
}

/// True for the copy the installer put in `<localappdata>\Programs\Murmur`.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn is_installed_copy(exe: &Path, localappdata: &Path) -> bool {
    let installed = localappdata.join("Programs").join("Murmur").join("murmur.exe");
    exe.to_string_lossy().eq_ignore_ascii_case(&installed.to_string_lossy())
}

/// The bundle an update replaces: the one the DMG says to drag to Applications.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const MAC_APP: &str = "/Applications/Murmur.app";

/// True for `/Applications/Murmur.app`. A copy run from the DMG or a dev build only gets the alert.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn is_installed_bundle(exe: &Path) -> bool {
    exe == Path::new(MAC_APP).join("Contents/MacOS/murmur")
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

/// Whether an hour has passed since `last` by the wall clock. A clock set back counts as due.
pub fn check_due(last: Option<SystemTime>, now: SystemTime) -> bool {
    last.is_none_or(|t| now.duration_since(t).map_or(true, |d| d >= CHECK_EVERY))
}

/// Checks a minute after launch, then hourly, calling `found` for each newer release seen.
/// Failures are only logged: the next check retries.
pub fn spawn_checker(found: impl Fn(Release) + Send + 'static) {
    std::thread::spawn(move || {
        std::thread::sleep(FIRST_CHECK);
        let mut last = None;
        loop {
            // sleep doesn't count time the PC spends asleep, so wake often and ask the clock
            if check_due(last, SystemTime::now()) {
                last = Some(SystemTime::now());
                match check() {
                    Ok(Some(r)) => found(r),
                    Ok(None) => log::info!("update check: up to date"),
                    Err(e) => log::info!("update check: {e}"),
                }
            }
            std::thread::sleep(TICK);
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
    let asset = Asset { url: r.setup_url.clone(), file: installer_name(&r.tag), sha256, size: r.setup_size };
    Fetcher::standard().download(&asset, dir, &mut |_| {}, &AtomicBool::new(false))
}

/// Starts the installer and returns; the caller quits Murmur so the files are free.
#[cfg(windows)]
pub fn install(setup: &Path) -> std::io::Result<()> {
    std::process::Command::new(setup).args(INSTALL_ARGS).spawn().map(drop)
}

/// Copies the new Murmur.app out of `dmg` to beside the installed one, then starts a helper that
/// swaps it in and relaunches once this process has exited; the caller quits Murmur. Launching
/// earlier would only meet the single-instance lock and exit.
#[cfg(target_os = "macos")]
pub fn install(dmg: &Path) -> std::io::Result<()> {
    use std::process::{Command, Stdio};
    let app = Path::new(MAC_APP);
    // beside the installed copy, so the swap is a rename on one volume
    let staged = app.with_file_name(".Murmur-update.app");
    let mount = dmg.with_extension("mount");
    let _ = fs::remove_dir_all(&staged);
    fs::create_dir_all(&mount)?;
    run(Command::new("hdiutil").args(["attach", "-nobrowse", "-readonly", "-noautoopen", "-mountpoint"]).arg(&mount).arg(dmg))?;
    // ditto keeps the signature intact
    let copied = run(Command::new("ditto").arg(mount.join("Murmur.app")).arg(&staged));
    let _ = run(Command::new("hdiutil").args(["detach", "-force"]).arg(&mount));
    let _ = fs::remove_dir(&mount);
    copied?;
    let log = fs::File::create(download_dir().join("install.log"))?;
    Command::new("/bin/sh")
        .args(["-c", SWAP_SCRIPT, "swap"])
        .arg(std::process::id().to_string())
        .arg(&staged)
        .arg(app)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .spawn()
        .map(drop)
}

/// `$1` pid to wait for, `$2` the staged bundle, `$3` the installed one. Moves the old bundle
/// aside before renaming the new one in and puts it back if that fails, so there's always a
/// Murmur to relaunch.
#[cfg(target_os = "macos")]
const SWAP_SCRIPT: &str = r#"
while kill -0 "$1" 2>/dev/null; do sleep 0.2; done
old="$(dirname "$3")/.Murmur-old.app"
rm -rf "$old"
if mv "$3" "$old"; then
  if mv "$2" "$3"; then rm -rf "$old"; echo "updated $3"; else mv "$old" "$3"; fi
fi
open "$3"
"#;

#[cfg(target_os = "macos")]
fn run(cmd: &mut std::process::Command) -> std::io::Result<()> {
    let out = cmd.output()?;
    if out.status.success() {
        return Ok(());
    }
    let why = String::from_utf8_lossy(&out.stderr);
    Err(std::io::Error::other(format!("{:?} failed: {}", cmd.get_program(), why.trim())))
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let r = parse_release_as(FIXTURE, windows_installer).unwrap();
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
        let r = parse_release_as(json, windows_installer).unwrap();
        assert_eq!((r.tag.as_str(), r.setup_size, r.setup_url.as_str(), r.sha_url.as_str()), ("1.2.3", 42, "https://x/setup.exe", "https://x/setup.exe.sha256"));
    }

    #[test]
    fn release_without_a_setup_asset_is_none() {
        let json = r#"{"tag_name":"v1.2.3","assets":[{"name":"murmur-v1.2.3-windows-x64.zip","size":1,"browser_download_url":"https://x/z"}]}"#;
        assert_eq!(parse_release_as(json, windows_installer), None);
        let no_sha = r#"{"tag_name":"v1.2.3","assets":[{"name":"murmur-v1.2.3-setup.exe","size":1,"browser_download_url":"https://x/s"}]}"#;
        assert_eq!(parse_release_as(no_sha, windows_installer), None);
    }

    // v0.5.0's assets as published
    const BOTH: &str = r#"{"tag_name":"v0.5.0","assets":[
        {"name":"murmur-v0.5.0-macos-arm64.dmg","size":15950118,"browser_download_url":"https://x/m.dmg"},
        {"name":"murmur-v0.5.0-macos-arm64.dmg.sha256","size":97,"browser_download_url":"https://x/m.dmg.sha256"},
        {"name":"murmur-v0.5.0-setup.exe","size":10,"browser_download_url":"https://x/s.exe"},
        {"name":"murmur-v0.5.0-setup.exe.sha256","size":90,"browser_download_url":"https://x/s.exe.sha256"}]}"#;

    #[test]
    fn each_platform_takes_its_own_installer() {
        let mac = parse_release_as(BOTH, |t| mac_installer(t, "aarch64")).unwrap();
        assert_eq!((mac.setup_url.as_str(), mac.setup_size, mac.sha_url.as_str()), ("https://x/m.dmg", 15_950_118, "https://x/m.dmg.sha256"));
        let win = parse_release_as(BOTH, windows_installer).unwrap();
        assert_eq!((win.setup_url.as_str(), win.sha_url.as_str()), ("https://x/s.exe", "https://x/s.exe.sha256"));
        // no Intel build is published
        assert_eq!(parse_release_as(BOTH, |t| mac_installer(t, "x86_64")), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn only_the_applications_bundle_updates_itself() {
        assert!(is_installed_bundle(Path::new("/Applications/Murmur.app/Contents/MacOS/murmur")));
        assert!(!is_installed_bundle(Path::new("/Users/jane/code/murmur/target/release/Murmur.app/Contents/MacOS/murmur")));
        assert!(!is_installed_bundle(Path::new("/Volumes/Murmur 0.5.0/Murmur.app/Contents/MacOS/murmur")));
        assert!(!is_installed_bundle(Path::new("/Users/jane/Applications/Murmur.app/Contents/MacOS/murmur")));
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

    // the installer's per-user folder; the Mac version will update differently
    #[cfg(windows)]
    #[test]
    fn only_the_installed_copy_updates_itself() {
        use std::path::Path;
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
        // both platforms' installers, so it passes on either
        let base = serve(vec![("/latest", BOTH.as_bytes().to_vec())]);
        let url = format!("{base}/latest");
        assert_eq!(check_at(&url, "0.4.21").unwrap().map(|r| r.tag), Some("0.5.0".into()));
        assert_eq!(check_at(&url, "0.5.0").unwrap(), None);
        assert!(check_at(&format!("{base}/missing"), "0.4.21").is_err());
    }

    #[test]
    fn download_verifies_against_the_sha256_file() {
        let setup = b"pretend installer".repeat(1000);
        let r = served_release(&setup, format!("{}  murmur-v9.9.9-setup.exe\n", hex(Sha256::digest(&setup).as_slice())));
        let dir = tmp("ok");
        let got = download(&r, &dir).unwrap();
        assert_eq!(got, dir.join(installer_name("9.9.9")));
        assert_eq!(std::fs::read(&got).unwrap(), setup);
    }

    #[test]
    fn download_refuses_a_setup_that_does_not_match() {
        let setup = b"pretend installer".to_vec();
        let r = served_release(&setup, format!("{}  murmur-v9.9.9-setup.exe\n", hex(Sha256::digest(b"something else").as_slice())));
        let dir = tmp("mismatch");
        assert!(matches!(download(&r, &dir), Err(FetchError::ChecksumMismatch)));
        assert!(!dir.join(installer_name("9.9.9")).exists());
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

    #[test]
    fn a_check_is_due_by_the_wall_clock() {
        use std::time::{Duration, SystemTime};
        let now = SystemTime::now();
        let hours = |h: u64| Duration::from_secs(h * 60 * 60);
        assert!(check_due(None, now));
        assert!(!check_due(Some(now - Duration::from_secs(59 * 60)), now));
        assert!(check_due(Some(now - hours(1)), now));
        // a laptop asleep overnight: its thread slept only minutes, but hours have passed
        assert!(check_due(Some(now - hours(8)), now));
        // the clock went back: check rather than wait for it to catch up
        assert!(check_due(Some(now + hours(1)), now));
    }

    #[test]
    fn installer_relaunches_only_when_asked() {
        let iss = include_str!("../installer/murmur.iss");
        let line = iss.lines().find(|l| l.contains("Check: RelaunchRequested")).expect("a [Run] entry for /RELAUNCH");
        assert!(line.contains(r#"Filename: "{app}\murmur.exe""#), "{line}");
        assert!(!line.contains("skipifsilent") && !line.contains("postinstall"), "{line}");
        assert!(iss.contains("CompareText(ParamStr(I), '/RELAUNCH') = 0"));
        assert!(INSTALL_ARGS.contains(&"/RELAUNCH"));
    }
}
