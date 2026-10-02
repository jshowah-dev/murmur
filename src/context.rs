//! Which kind of app the dictation is about to land in.

use crate::config::Config;
use windows::core::PWSTR;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Profile {
    Plain,
    Email,
}

impl Profile {
    pub fn name(self) -> &'static str {
        match self {
            Profile::Plain => "plain",
            Profile::Email => "email",
        }
    }
}

/// A webmail title only counts in one of these, so "Outlook migration.docx" in Word is not email.
const BROWSERS: &[&str] = &["chrome.exe", "msedge.exe", "firefox.exe", "brave.exe", "opera.exe", "vivaldi.exe", "arc.exe"];

pub fn classify(exe: &str, title: &str, cfg: &Config) -> Profile {
    let exe = exe.to_lowercase();
    if !exe.is_empty() && cfg.email_apps.iter().any(|a| a.to_lowercase() == exe) {
        return Profile::Email;
    }
    if BROWSERS.contains(&exe.as_str()) {
        let title = title.to_lowercase();
        if cfg.email_titles.iter().any(|t| !t.is_empty() && title.contains(&t.to_lowercase())) {
            return Profile::Email;
        }
    }
    Profile::Plain
}

/// The foreground window's exe file name and title. `None` when the process cannot be opened
/// (an elevated app seen from a non-elevated Murmur).
fn foreground() -> Option<(String, String)> {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut path = [0u16; 1024];
        let mut len = path.len() as u32;
        let named = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(path.as_mut_ptr()), &mut len);
        let _ = CloseHandle(process);
        named.ok()?;
        let path = String::from_utf16_lossy(&path[..len as usize]);
        let exe = path.rsplit(['\\', '/']).next().unwrap_or_default().to_string();
        let mut title = [0u16; 512];
        let n = GetWindowTextW(hwnd, &mut title).max(0) as usize;
        Some((exe, String::from_utf16_lossy(&title[..n])))
    }
}

/// The profile for the window that has the focus right now. The title is used for the match
/// and dropped: it can hold a subject line or an address, so it is never logged.
pub fn detect(cfg: &Config) -> Profile {
    match foreground() {
        Some((exe, title)) => classify(&exe, &title, cfg),
        None => Profile::Plain,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mail_clients_match_by_exe_in_any_case() {
        let c = Config::default();
        for exe in ["OUTLOOK.EXE", "outlook.exe", "olk.exe", "Thunderbird.exe"] {
            assert_eq!(classify(exe, "anything", &c), Profile::Email, "{exe}");
        }
    }

    #[test]
    fn webmail_matches_by_title_in_a_browser() {
        let c = Config::default();
        assert_eq!(classify("chrome.exe", "Inbox (3) - me@gmail.com - Gmail - Google Chrome", &c), Profile::Email);
        assert_eq!(classify("MSEDGE.EXE", "Mail - Jeff - outlook - Microsoft Edge", &c), Profile::Email);
        assert_eq!(classify("firefox.exe", "Proton Mail — Mozilla Firefox", &c), Profile::Email);
        assert_eq!(classify("chrome.exe", "Rust docs - Google Chrome", &c), Profile::Plain);
    }

    #[test]
    fn a_mail_title_outside_a_browser_is_plain() {
        let c = Config::default();
        assert_eq!(classify("WINWORD.EXE", "Outlook migration.docx - Word", &c), Profile::Plain);
        assert_eq!(classify("notepad.exe", "Gmail notes.txt - Notepad", &c), Profile::Plain);
    }

    #[test]
    fn user_added_apps_and_titles_count() {
        let mut c = Config::default();
        c.email_apps.push("MailSpring.exe".into());
        c.email_titles.push("Fastmail".into());
        assert_eq!(classify("mailspring.exe", "", &c), Profile::Email);
        assert_eq!(classify("brave.exe", "Inbox | Fastmail - Brave", &c), Profile::Email);
    }

    #[test]
    fn empty_inputs_are_plain() {
        let mut c = Config::default();
        assert_eq!(classify("", "", &c), Profile::Plain);
        // a blank entry must not match every window
        c.email_apps = vec!["".into()];
        c.email_titles = vec!["".into()];
        assert_eq!(classify("", "", &c), Profile::Plain);
        assert_eq!(classify("chrome.exe", "Rust docs - Google Chrome", &c), Profile::Plain);
        c.email_apps.clear();
        c.email_titles.clear();
        assert_eq!(classify("OUTLOOK.EXE", "Inbox - Gmail", &c), Profile::Plain);
        assert_eq!(classify("chrome.exe", "Inbox - Gmail", &c), Profile::Plain);
    }

    #[test]
    fn profile_names() {
        assert_eq!(Profile::Plain.name(), "plain");
        assert_eq!(Profile::Email.name(), "email");
    }
}
