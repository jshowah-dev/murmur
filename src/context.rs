//! Which kind of app the dictation is about to land in.

use crate::config::Config;
#[cfg(windows)]
use windows::core::PWSTR;
#[cfg(windows)]
use windows::Win32::Foundation::{CloseHandle, HWND};
#[cfg(windows)]
use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
#[cfg(windows)]
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
/// Exe names on Windows, bundle ids on macOS, in lower case.
const BROWSERS: &[&str] = &[
    "chrome.exe",
    "msedge.exe",
    "firefox.exe",
    "brave.exe",
    "opera.exe",
    "vivaldi.exe",
    "arc.exe",
    "com.apple.safari",
    "com.google.chrome",
    "com.microsoft.edgemac",
    "org.mozilla.firefox",
    "com.brave.browser",
    "com.operasoftware.opera",
    "com.vivaldi.vivaldi",
    "company.thebrowser.browser",
];

fn is_browser(exe: &str) -> bool {
    BROWSERS.contains(&exe.to_lowercase().as_str())
}

pub fn classify(exe: &str, title: &str, cfg: &Config) -> Profile {
    let exe = exe.to_lowercase();
    if !exe.is_empty() && cfg.email_apps.iter().any(|a| a.to_lowercase() == exe) {
        return Profile::Email;
    }
    if is_browser(&exe) {
        let title = title.to_lowercase();
        if cfg.email_titles.iter().any(|t| !t.is_empty() && title.contains(&t.to_lowercase())) {
            return Profile::Email;
        }
    }
    Profile::Plain
}

/// The foreground window and its exe file name. `None` when there is no window or the process
/// cannot be opened (an elevated app seen from a non-elevated Murmur).
#[cfg(windows)]
fn foreground() -> Option<(HWND, String)> {
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
        Some((hwnd, path.rsplit(['\\', '/']).next().unwrap_or_default().to_string()))
    }
}

#[cfg(windows)]
fn window_title(hwnd: HWND) -> String {
    let mut title = [0u16; 512];
    let n = unsafe { GetWindowTextW(hwnd, &mut title) }.max(0) as usize;
    String::from_utf16_lossy(&title[..n])
}

/// The profile for the window that has the focus right now. The title is read only for a
/// browser, used for the match and dropped: it can hold a subject line or an address, so it is
/// never logged.
#[cfg(windows)]
pub fn detect(cfg: &Config) -> Profile {
    match foreground() {
        Some((hwnd, exe)) => {
            let title = if is_browser(&exe) { window_title(hwnd) } else { String::new() };
            classify(&exe, &title, cfg)
        }
        None => Profile::Plain,
    }
}

/// The profile for the app in front right now, by its bundle id. A browser's window title is
/// read through Accessibility (Murmur is allowed it, to paste), used for the match and dropped,
/// as on Windows.
#[cfg(target_os = "macos")]
pub fn detect(cfg: &Config) -> Profile {
    let Some(app) = objc2_app_kit::NSWorkspace::sharedWorkspace().frontmostApplication() else { return Profile::Plain };
    let Some(id) = app.bundleIdentifier().map(|s| s.to_string()) else { return Profile::Plain };
    let title = if is_browser(&id) { ax::window_title(app.processIdentifier()).unwrap_or_default() } else { String::new() };
    classify(&id, &title, cfg)
}

#[cfg(target_os = "macos")]
mod ax {
    use objc2_core_foundation::{CFRetained, CFString, CFType};
    use std::ffi::c_void;
    use std::ptr::NonNull;

    type Ref = *const c_void;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementCreateApplication(pid: i32) -> Ref;
        fn AXUIElementCopyAttributeValue(element: Ref, attribute: Ref, value: *mut Ref) -> i32;
        fn AXUIElementSetMessagingTimeout(element: Ref, seconds: f32) -> i32;
    }

    /// Takes ownership of a CoreFoundation object from a Create or Copy call.
    fn owned(r: Ref) -> Option<CFRetained<CFType>> {
        NonNull::new(r as *mut CFType).map(|p| unsafe { CFRetained::from_raw(p) })
    }

    fn attribute(element: &CFType, attr: &str) -> Option<CFRetained<CFType>> {
        let attr = CFString::from_str(attr);
        let mut out = std::ptr::null();
        let err = unsafe { AXUIElementCopyAttributeValue(element as *const CFType as Ref, &*attr as *const CFString as Ref, &mut out) };
        if err == 0 { owned(out) } else { None }
    }

    /// The title of `pid`'s focused window.
    pub fn window_title(pid: i32) -> Option<String> {
        let app = owned(unsafe { AXUIElementCreateApplication(pid) })?;
        // a busy browser mustn't hold up the paste for long
        unsafe { AXUIElementSetMessagingTimeout(&*app as *const CFType as Ref, 0.25) };
        let window = attribute(&app, "AXFocusedWindow")?;
        let title = attribute(&window, "AXTitle")?;
        title.downcast_ref::<CFString>().map(|s| s.to_string())
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
    fn mac_mail_apps_and_browsers_match_by_bundle_id() {
        let c = Config::default();
        for id in ["com.apple.mail", "com.microsoft.Outlook", "org.mozilla.thunderbird"] {
            assert_eq!(classify(id, "", &c), Profile::Email, "{id}");
        }
        assert_eq!(classify("com.apple.Safari", "Inbox (3) - me@gmail.com - Gmail", &c), Profile::Email);
        assert_eq!(classify("com.google.Chrome", "Mail - Jeff - Outlook", &c), Profile::Email);
        assert_eq!(classify("com.apple.Safari", "Rust docs", &c), Profile::Plain);
        assert_eq!(classify("com.apple.TextEdit", "Gmail notes.txt", &c), Profile::Plain);
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
    fn is_browser_ignores_case() {
        assert!(is_browser("Chrome.EXE"));
        assert!(is_browser("msedge.exe"));
        assert!(!is_browser("OUTLOOK.EXE"));
        assert!(!is_browser(""));
    }

    #[test]
    fn profile_names() {
        assert_eq!(Profile::Plain.name(), "plain");
        assert_eq!(Profile::Email.name(), "email");
    }
}
