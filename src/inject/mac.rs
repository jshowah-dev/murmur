use crate::history::InjectRecord;
use anyhow::{anyhow, Result};

// TODO(macos): NSPasteboard and a synthetic Cmd+V, which needs the Accessibility permission.
pub fn paste(_text: &str) -> Result<InjectRecord> {
    Err(anyhow!("pasting isn't supported on macOS yet"))
}
