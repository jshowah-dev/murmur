use anyhow::{anyhow, Result};

// TODO(macos phase 3): SMAppService's main-app login item.
pub fn is_enabled() -> bool {
    false
}

pub fn set(_enabled: bool) -> Result<()> {
    Err(anyhow!("opening at login isn't supported on macOS yet"))
}
