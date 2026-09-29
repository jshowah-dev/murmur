//! Update checks against the latest GitHub release, and the parts of click-to-update that don't need a UI.

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
