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
