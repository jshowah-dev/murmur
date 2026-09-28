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
