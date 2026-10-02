# Murmur format by context (email) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** In an email app, a dictated greeting lands on its own line and a dictated sign-off on its own lines; everywhere else the output is unchanged.

**Architecture:** A pure `classify(exe, title, cfg) -> Profile` plus a thin Win32 `detect` decide whether the foreground window is email. A pure `email::format(text)` reshapes the start and end of the tidied text. `cleanup::clean` takes the profile and calls `email::format` between `tidy` and snippet restore; the pipeline detects the profile just before cleaning.

**Tech Stack:** Rust (edition as in `Cargo.toml`), `windows` 0.62 (features already enabled), `serde`/`toml`. No new dependency.

**Spec:** `docs/superpowers/specs/2026-10-02-murmur-format-by-context-design.md`

## Global Constraints

- Branch `feat/format-by-context`, worktree `C:\Users\JeffLocal\git\murmur\.claude\worktrees\musing-mcnulty-a927f7`. Never commit to `main`. Never push.
- No new crate and no new `windows` feature. `Win32_System_Threading`, `Win32_UI_WindowsAndMessaging` and `Win32_Foundation` are already in `Cargo.toml`.
- The window title is **never logged**, at any level. The only new log line is `profile: email` or `profile: plain` at INFO.
- `Profile::Plain` output must be byte-for-byte today's output. The existing `cleanup` tests are the proof and must pass unedited.
- Defaults, verbatim: `format_by_context = true`; `email_apps = ["OUTLOOK.EXE", "olk.exe", "thunderbird.exe"]`; `email_titles = ["Gmail", "Outlook", "Proton Mail", "Yahoo Mail"]`.
- Browsers, fixed in code, compared case-insensitively: `chrome.exe`, `msedge.exe`, `firefox.exe`, `brave.exe`, `opera.exe`, `vivaldi.exe`, `arc.exe`.
- Out of scope: version bump, release, any other profile, any UI.
- Commit messages end with the trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` (this repo's convention).
- **Running tests.** Plain `cargo test` from Git Bash fails in this environment. Create `%TEMP%\murmur-test.cmd` once with exactly these lines and run it as `cmd //c "%TEMP%\murmur-test.cmd" <filter>`:

  ```
  @echo off
  call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul
  cd /d C:\Users\JeffLocal\git\murmur\.claude\worktrees\musing-mcnulty-a927f7
  set CARGO_TARGET_DIR=C:\Users\JeffLocal\git\murmur\target
  cargo test --release --locked %*
  ```

  It prints a harmless `'vswhere.exe' is not recognized` line. If lib tests crash with `ORT Version is: 1.17.1` / `STATUS_ACCESS_VIOLATION`, copy `C:\Users\JeffLocal\git\murmur\target\release\onnxruntime*.dll` into `...\target\release\deps\`. If the build fails with `os error 32`, a Murmur dev build is running from `target\release`: stop and ask Jeff to quit it. Do not kill it yourself.

## Review Focus

1. A closing sentence that starts with a closer word but is not a sign-off ("…Friday. Thanks a lot.", "Thank you very much.") must stay unchanged. Test in Task 3 (`closing_sentences_are_not_signoffs`).
2. A greeting opener followed by ordinary words ("Hi, just checking in. Are you free?") must not swallow them into the greeting line. Test in Task 3 (`opener_without_a_name`).
3. Names with a title or non-ASCII letters ("Dear Mr. Smith,", "Hi José,") must not break the greeting or panic on a byte boundary. Test in Task 3 (`titles_and_non_ascii_names`).
4. A snippet placeholder at the very start or end of the text must never be read as a name or a closer. Test in Task 3 (`placeholders_are_never_names`) and Task 4 (`email_leaves_a_trailing_snippet_alone`).
5. A window whose exe or title cannot be read (empty strings), and config lists emptied by the user, must give `Plain`, not a match on the empty string. Test in Task 2 (`empty_inputs_are_plain`).

---

### Task 1: Config keys

**Files:**
- Modify: `src/config.rs` (struct `Config` at line 8, `Default` at line 27, tests at the end)

**Interfaces:**
- Produces: `Config.format_by_context: bool`, `Config.email_apps: Vec<String>`, `Config.email_titles: Vec<String>`.

- [ ] **Step 1: Write the failing tests**

Add inside `mod tests` in `src/config.rs`:

```rust
    #[test]
    fn format_by_context_defaults() {
        let c = Config::default();
        assert!(c.format_by_context);
        assert_eq!(c.email_apps, vec!["OUTLOOK.EXE", "olk.exe", "thunderbird.exe"]);
        assert_eq!(c.email_titles, vec!["Gmail", "Outlook", "Proton Mail", "Yahoo Mail"]);
    }

    #[test]
    fn a_config_written_before_format_by_context_loads_with_its_defaults() {
        let c: Config = toml::from_str("ptt_key = \"F13\"\ndebug_log = true\n").unwrap();
        assert_eq!(c.ptt_key, "F13");
        assert!(c.format_by_context);
        assert_eq!(c.email_apps, Config::default().email_apps);
        assert_eq!(c.email_titles, Config::default().email_titles);
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cmd //c "%TEMP%\murmur-test.cmd" --lib config::tests`
Expected: compile error, `no field format_by_context on type Config`.

- [ ] **Step 3: Add the fields**

In `pub struct Config`, after `debug_log`:

```rust
    /// Shape the text for the app it lands in. Only email is recognised: a dictated greeting
    /// and sign-off get their own lines. false = the same output everywhere.
    pub format_by_context: bool,
    /// Exe names treated as email, compared without regard to case.
    pub email_apps: Vec<String>,
    /// Window-title fragments treated as email. They count in browsers only.
    pub email_titles: Vec<String>,
```

In `Default::default`, after `debug_log: false,`:

```rust
            format_by_context: true,
            email_apps: ["OUTLOOK.EXE", "olk.exe", "thunderbird.exe"].iter().map(|s| s.to_string()).collect(),
            email_titles: ["Gmail", "Outlook", "Proton Mail", "Yahoo Mail"].iter().map(|s| s.to_string()).collect(),
```

- [ ] **Step 4: Run to verify they pass**

Run: `cmd //c "%TEMP%\murmur-test.cmd" --lib config::tests`
Expected: all `config::tests` pass, including the two new ones.

- [ ] **Step 5: Commit**

```bash
git add src/config.rs
git commit -m "feat(config): format_by_context, email_apps, email_titles

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Context detection

**Files:**
- Create: `src/context.rs`
- Modify: `src/lib.rs` (add `pub mod context;` after `pub mod config;`)

**Interfaces:**
- Consumes: `Config.email_apps`, `Config.email_titles` (Task 1).
- Produces:
  - `pub enum Profile { Plain, Email }` deriving `Debug, Clone, Copy, PartialEq`
  - `impl Profile { pub fn name(self) -> &'static str }` → `"plain"` / `"email"`
  - `pub fn classify(exe: &str, title: &str, cfg: &Config) -> Profile`
  - `pub fn detect(cfg: &Config) -> Profile`

- [ ] **Step 1: Write the module with tests and a stub `classify`**

Create `src/context.rs`:

```rust
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
    let _ = (exe, title, cfg);
    Profile::Plain
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
```

Add to `src/lib.rs`, after `pub mod config;`:

```rust
pub mod context;
```

- [ ] **Step 2: Run to verify the tests fail**

Run: `cmd //c "%TEMP%\murmur-test.cmd" --lib context::tests`
Expected: `mail_clients_match_by_exe_in_any_case`, `webmail_matches_by_title_in_a_browser` and `user_added_apps_and_titles_count` FAIL (`left: Plain, right: Email`); the other three pass. Unused-import warnings for the Win32 items are expected at this step.

- [ ] **Step 3: Implement `classify` and `detect`**

Replace the stub `classify` with:

```rust
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
```

Signatures verified against `windows-0.62.2` source: `OpenProcess(PROCESS_ACCESS_RIGHTS, bool, u32) -> Result<HANDLE>`, `QueryFullProcessImageNameW(HANDLE, PROCESS_NAME_FORMAT, PWSTR, *mut u32) -> Result<()>`, `GetWindowTextW(HWND, &mut [u16]) -> i32`, `GetWindowThreadProcessId(HWND, Option<*mut u32>) -> u32`.

- [ ] **Step 4: Run to verify the tests pass**

Run: `cmd //c "%TEMP%\murmur-test.cmd" --lib context::tests`
Expected: 6 passed, no warnings from `src/context.rs`.

- [ ] **Step 5: Commit**

```bash
git add src/context.rs src/lib.rs
git commit -m "feat(context): recognise an email app from the foreground window

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Email formatting

**Files:**
- Create: `src/email.rs`
- Modify: `src/lib.rs` (add `pub mod email;` after `pub mod dictionary_edit;`)

**Interfaces:**
- Produces: `pub fn format(text: &str) -> String`. Input is tidied text (single spaces, `\n` for line breaks, snippet placeholders in U+E000..=U+F8FF). Output is the same text with the greeting and sign-off reshaped; text matching neither rule is returned unchanged.

- [ ] **Step 1: Write the module with tests and a stub `format`**

Create `src/email.rs`:

```rust
//! Email shaping: a dictated greeting gets its own line, a dictated sign-off its own lines.
//! Runs on tidied text, before snippet placeholders are restored.

pub fn format(text: &str) -> String {
    text.to_string()
}

#[cfg(test)]
mod tests {
    use super::format;

    fn unchanged(text: &str) {
        assert_eq!(format(text), text);
    }

    #[test]
    fn greeting_and_signoff_together() {
        assert_eq!(
            format("Hi Sarah, thanks for the update. I'll review it tomorrow and send notes by Friday. Thanks, Jeff."),
            "Hi Sarah,\n\nThanks for the update. I'll review it tomorrow and send notes by Friday.\n\nThanks,\nJeff"
        );
    }

    #[test]
    fn greeting_rows() {
        assert_eq!(format("Hi Sarah, thanks for the update."), "Hi Sarah,\n\nThanks for the update.");
        assert_eq!(format("Hi, Sarah. Thanks for the update."), "Hi Sarah,\n\nThanks for the update.");
        assert_eq!(format("Good morning, team. The build is ready."), "Good morning team,\n\nThe build is ready.");
        assert_eq!(format("Hello. Quick question."), "Hello,\n\nQuick question.");
        assert_eq!(format("Hello there, I wanted to ask."), "Hello there,\n\nI wanted to ask.");
        assert_eq!(format("Hi Sarah and Tom, the build is ready."), "Hi Sarah and Tom,\n\nThe build is ready.");
    }

    #[test]
    fn greeting_only_ends_with_a_blank_line() {
        assert_eq!(format("Hi Sarah."), "Hi Sarah,\n\n");
        assert_eq!(format("Hi Sarah"), "Hi Sarah,\n\n");
        assert_eq!(format("Good morning."), "Good morning,\n\n");
    }

    #[test]
    fn greeting_does_not_fire() {
        unchanged("Hi Sarah and Tom and Priya and Dev, the build is ready.");
        unchanged("The build is ready. Hi Sarah, thanks.");
        unchanged("Hey Jeff can you send the file");
        unchanged("History is the topic today.");
        unchanged("\nHi Sarah, thanks for the update.");
    }

    #[test]
    fn opener_without_a_name() {
        assert_eq!(format("Hi, just checking in. Are you free?"), "Hi,\n\nJust checking in. Are you free?");
        assert_eq!(format("Hey, can you send the file?"), "Hey,\n\nCan you send the file?");
    }

    #[test]
    fn titles_and_non_ascii_names() {
        assert_eq!(format("Dear Mr. Smith, the report is attached."), "Dear Mr. Smith,\n\nThe report is attached.");
        assert_eq!(format("Hi José, ça va?"), "Hi José,\n\nÇa va?");
        assert_eq!(format("See you then. Thanks, Zoë."), "See you then.\n\nThanks,\nZoë");
    }

    #[test]
    fn signoff_rows() {
        assert_eq!(format("I'll send notes by Friday. Thanks, Jeff."), "I'll send notes by Friday.\n\nThanks,\nJeff");
        assert_eq!(format("I'll send notes by Friday. Best regards. Jeff Showah."), "I'll send notes by Friday.\n\nBest regards,\nJeff Showah");
        assert_eq!(format("I'll send notes by Friday. Thanks."), "I'll send notes by Friday.\n\nThanks.");
        assert_eq!(format("Thanks, Jeff."), "Thanks,\nJeff");
        assert_eq!(format("Is Friday okay? Thank you, Jeff"), "Is Friday okay?\n\nThank you,\nJeff");
    }

    #[test]
    fn signoff_does_not_fire() {
        unchanged("Thanks for the update, I'll look tomorrow.");
        unchanged("I said thanks, Jeff.");
        unchanged("I think that works best.");
        unchanged("Thanks.");
        unchanged("Send it over. Thanks, Jeff Robert Alan Showah.");
    }

    #[test]
    fn closing_sentences_are_not_signoffs() {
        unchanged("I'll send notes by Friday. Thanks a lot.");
        unchanged("I'll send notes by Friday. Thank you very much.");
        unchanged("I'll send notes by Friday. Thanks, I appreciate it.");
    }

    #[test]
    fn existing_line_breaks_are_not_doubled() {
        assert_eq!(format("Hi Sarah,\nThanks for the update."), "Hi Sarah,\n\nThanks for the update.");
        assert_eq!(format("Hi Sarah,\n\nThanks for the update."), "Hi Sarah,\n\nThanks for the update.");
        assert_eq!(format("Send it Friday.\n\nThanks, Jeff."), "Send it Friday.\n\nThanks,\nJeff");
        assert_eq!(format("Send it Friday.\nThanks, Jeff."), "Send it Friday.\n\nThanks,\nJeff");
        unchanged("Send it Friday. Thanks.\n");
        unchanged("Thanks.\nJeff.");
    }

    #[test]
    fn placeholders_are_never_names() {
        unchanged("\u{E000}");
        unchanged("Thanks. \u{E000}");
        unchanged("Send it Friday. Thanks, \u{E000}.");
        unchanged("Hi \u{E000}, the build is ready.");
        assert_eq!(format("Hi Sarah, \u{E000}"), "Hi Sarah,\n\n\u{E000}");
    }

    #[test]
    fn middle_of_an_email_is_untouched() {
        unchanged("");
        unchanged("The numbers look right. I'll confirm with finance on Monday.");
    }
}
```

Add to `src/lib.rs`, after `pub mod dictionary_edit;`:

```rust
pub mod email;
```

- [ ] **Step 2: Run to verify the tests fail**

Run: `cmd //c "%TEMP%\murmur-test.cmd" --lib email::tests`
Expected: `greeting_and_signoff_together`, `greeting_rows`, `greeting_only_ends_with_a_blank_line`, `opener_without_a_name`, `titles_and_non_ascii_names`, `signoff_rows`, `existing_line_breaks_are_not_doubled` and `placeholders_are_never_names` FAIL; the "does not fire" tests pass against the stub.

- [ ] **Step 3: Implement**

Replace the stub `format` (everything above `#[cfg(test)]`, keeping the `//!` header) with:

```rust
const OPENERS: &[&[&str]] = &[
    &["good", "morning"],
    &["good", "afternoon"],
    &["good", "evening"],
    &["hi"],
    &["hello"],
    &["hey"],
    &["dear"],
    &["greetings"],
];

const CLOSERS: &[&[&str]] = &[
    &["thanks", "so", "much"],
    &["thanks", "again"],
    &["many", "thanks"],
    &["thank", "you"],
    &["best", "regards"],
    &["kind", "regards"],
    &["warm", "regards"],
    &["talk", "soon"],
    &["thanks"],
    &["best"],
    &["regards"],
    &["cheers"],
    &["sincerely"],
];

/// Lowercase words that can stand where a name would after a greeting opener.
const GROUP_WORDS: &[&str] = &["there", "all", "everyone", "everybody", "team", "folks", "both", "and"];

/// Their full stop is part of the word, not the end of the greeting.
const TITLES: &[&str] = &["mr", "mrs", "ms", "dr", "prof"];

const MAX_NAME_WORDS: usize = 3;

/// Lowercased token with the recogniser's punctuation stripped.
fn bare(tok: &str) -> String {
    tok.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()
}

fn strip_end(tok: &str) -> &str {
    tok.trim_end_matches([',', '.', '!'])
}

fn is_title(tok: &str) -> bool {
    TITLES.contains(&bare(tok).as_str())
}

/// A capitalised word (or, in a greeting, a group word). Snippet placeholders are not letters,
/// so a snippet is never taken for a name.
fn name_like(tok: &str, group_ok: bool) -> bool {
    let core = strip_end(tok);
    let Some(first) = core.chars().next() else { return false };
    if !core.chars().all(|c| c.is_alphabetic() || matches!(c, '-' | '\'' | '’')) {
        return false;
    }
    first.is_uppercase() || (group_ok && GROUP_WORDS.contains(&core.to_lowercase().as_str()))
}

/// Word count of the longest phrase in `list` that `tokens` starts with.
fn leading_phrase(tokens: &[&str], list: &[&[&str]]) -> Option<usize> {
    list.iter().filter(|p| tokens.len() >= p.len() && p.iter().zip(tokens).all(|(w, t)| bare(t) == *w)).map(|p| p.len()).max()
}

/// Word count of the longest phrase in `list` that `tokens` ends with.
fn trailing_phrase(tokens: &[&str], list: &[&[&str]]) -> Option<usize> {
    list.iter()
        .filter(|p| tokens.len() >= p.len() && p.iter().zip(&tokens[tokens.len() - p.len()..]).all(|(w, t)| bare(t) == *w))
        .map(|p| p.len())
        .max()
}

fn capitalise(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// A greeting at the start of `text`: the greeting line, ending in a comma, and the text after it.
fn split_greeting(text: &str) -> Option<(String, &str)> {
    let line = &text[..text.find('\n').unwrap_or(text.len())];
    let tokens: Vec<&str> = line.split(' ').collect();
    let opener = leading_phrase(&tokens, OPENERS)?;
    let opener_end = tokens[opener - 1];
    // tokens[..end] are the greeting
    let mut end = opener;
    let mut found = opener_end.ends_with(['.', '!']) || opener == tokens.len();
    if !found {
        for j in opener..tokens.len().min(opener + MAX_NAME_WORDS) {
            if !name_like(tokens[j], true) {
                break;
            }
            if j + 1 == tokens.len() || (tokens[j].ends_with([',', '.', '!']) && !is_title(tokens[j])) {
                end = j + 1;
                found = true;
                break;
            }
        }
    }
    // "Hey, can you send it": no name, but the comma still ends the greeting
    if !found && !opener_end.ends_with(',') {
        return None;
    }
    let words: Vec<&str> = tokens[..end].iter().enumerate().map(|(i, t)| if i + 1 < end && is_title(t) { *t } else { strip_end(t) }).collect();
    let consumed = tokens[..end].iter().map(|t| t.len() + 1).sum::<usize>().min(line.len());
    Some((format!("{},", words.join(" ")), text[consumed..].trim_start_matches('\n')))
}

/// `text` with a trailing sign-off moved onto its own lines; `None` when it does not end in one.
fn split_signoff(text: &str) -> Option<String> {
    let line_start = text.rfind('\n').map_or(0, |i| i + 1);
    let tokens: Vec<&str> = text[line_start..].split(' ').collect();
    for names in 0..=MAX_NAME_WORDS.min(tokens.len() - 1) {
        let (head, name) = tokens.split_at(tokens.len() - names);
        let Some(len) = trailing_phrase(head, CLOSERS) else { continue };
        let start = head.len() - len;
        if start > 0 && !head[start - 1].ends_with(['.', '!', '?']) {
            continue;
        }
        let is_name = name.iter().enumerate().all(|(i, t)| name_like(t, false) && (i + 1 == name.len() || !t.ends_with([',', '.', '!'])));
        if !is_name {
            continue;
        }
        let closer = if name.is_empty() {
            head[start..].join(" ")
        } else {
            let words: Vec<&str> = head[start..].iter().map(|t| strip_end(t)).collect();
            format!("{},\n{}", words.join(" "), strip_end(&name.join(" ")))
        };
        let before = format!("{}{}", &text[..line_start], head[..start].join(" "));
        let before = before.trim_end_matches(['\n', ' ']);
        return Some(if before.is_empty() { format!("{}{closer}", &text[..line_start]) } else { format!("{before}\n\n{closer}") });
    }
    None
}

pub fn format(text: &str) -> String {
    match split_greeting(text) {
        Some((greeting, rest)) => {
            let rest = capitalise(rest);
            let rest = split_signoff(&rest).unwrap_or(rest);
            format!("{greeting}\n\n{rest}")
        }
        None => split_signoff(text).unwrap_or_else(|| text.to_string()),
    }
}
```

Notes for the implementer:
- `text.split(' ')` always yields at least one token (an empty one for an empty line), so `tokens.len() - 1` cannot underflow and `tokens[opener - 1]` is in range (`leading_phrase` returns at least 1).
- `consumed` lands on a byte boundary because tokens are split on the ASCII space.
- With a greeting, the sign-off is looked for in the rest only, so a greeting-only dictation ("Hi Sarah.") is never also read as a sign-off.
- The whole-dictation `"Thanks."` is unchanged because `before` is empty and the no-name closer is re-emitted as dictated.

- [ ] **Step 4: Run to verify the tests pass**

Run: `cmd //c "%TEMP%\murmur-test.cmd" --lib email::tests`
Expected: 12 passed. If a row fails, fix the code, not the expected string: every expected string is a row of the spec's tables or a Review Focus line.

- [ ] **Step 5: Commit**

```bash
git add src/email.rs src/lib.rs
git commit -m "feat(email): greeting and sign-off get their own lines

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Wire it into cleanup and the pipeline; document it

**Files:**
- Modify: `src/cleanup.rs:1-3` (imports), `src/cleanup.rs:103-113` (`clean`), its `mod tests`
- Modify: `src/pipeline.rs:1` (imports), `src/pipeline.rs:152-155` (the `cleaned` block in `stop`)
- Modify: `src/main.rs:3` (the `use murmur_lib::{…}` list)
- Modify: `README.md` (config table; new subsection before `### Dictionary`; Privacy list)

**Interfaces:**
- Consumes: `context::Profile`, `context::detect(&Config) -> Profile`, `Profile::name()` (Task 2); `email::format(&str) -> String` (Task 3); `Config.format_by_context` (Task 1).
- Produces: `cleanup::clean(text: &str, dict: &Dictionary, snippets: &Snippets, cfg: &Config, profile: Profile) -> String`.

- [ ] **Step 1: Write the failing tests**

In `src/cleanup.rs` `mod tests`, add this helper directly under the `use` lines. It shadows the glob-imported `clean`, so the existing tests keep their four-argument calls and prove `Plain` is unchanged:

```rust
    use crate::context::Profile;

    /// The existing tests run as `Plain`: its output must stay what it was before profiles.
    fn clean(text: &str, dict: &Dictionary, snippets: &Snippets, cfg: &Config) -> String {
        super::clean(text, dict, snippets, cfg, Profile::Plain)
    }

    /// No dictionary terms, so sound-alike matching cannot touch the words under test.
    fn bare_env() -> (Dictionary, Config) {
        (Dictionary::from_terms(vec![]), Config::default())
    }
```

Then add these tests at the end of `mod tests`:

```rust
    #[test]
    fn email_profile_shapes_greeting_and_signoff() {
        let (d, c) = bare_env();
        let raw = "um, hi Sarah, thanks for the update. I'll review it tomorrow and send notes by Friday. Thanks, Jeff.";
        assert_eq!(
            super::clean(raw, &d, &Snippets::default(), &c, Profile::Email),
            "Hi Sarah,\n\nThanks for the update. I'll review it tomorrow and send notes by Friday.\n\nThanks,\nJeff"
        );
        assert_eq!(
            super::clean(raw, &d, &Snippets::default(), &c, Profile::Plain),
            "Hi Sarah, thanks for the update. I'll review it tomorrow and send notes by Friday. Thanks, Jeff."
        );
    }

    #[test]
    fn email_leaves_a_trailing_snippet_alone() {
        let (d, c) = bare_env();
        let plain = super::clean("Thanks. New paragraph. My signature.", &d, &snips(), &c, Profile::Plain);
        assert_eq!(super::clean("Thanks. New paragraph. My signature.", &d, &snips(), &c, Profile::Email), plain);
        // a one-line snippet after a closer is not a name
        assert_eq!(super::clean("Send it over. Thanks, hob number.", &d, &snips(), &c, Profile::Email), "Send it over. Thanks, hawb-123.");
    }

    #[test]
    fn email_keeps_spoken_line_breaks() {
        let (d, c) = bare_env();
        assert_eq!(
            super::clean("Hi Sarah, new line, the build is ready. New paragraph. Thanks, Jeff.", &d, &Snippets::default(), &c, Profile::Email),
            "Hi Sarah,\n\nThe build is ready.\n\nThanks,\nJeff"
        );
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cmd //c "%TEMP%\murmur-test.cmd" --lib cleanup::tests`
Expected: compile error, `this function takes 4 arguments but 5 arguments were supplied`.

- [ ] **Step 3: Give `clean` the profile**

In `src/cleanup.rs`, add to the imports at the top:

```rust
use crate::context::Profile;
```

Replace the whole `pub fn clean` with:

```rust
pub fn clean(text: &str, dict: &Dictionary, snippets: &Snippets, cfg: &Config, profile: Profile) -> String {
    let s = strip_fillers(text, &cfg.fillers);
    // before the dictionary so phonetic matching cannot rewrite trigger words
    let (mut s, expansions) = snippets.mark(&s);
    s = dict.apply(&s);
    if cfg.spoken_commands {
        s = apply_commands(&s);
    }
    let s = tidy(&s);
    // trim spaces only: a leading/trailing "new line" command is deliberate
    let s = s.trim_matches(' ');
    // before the snippets come back, so their text is never reshaped or read as a name
    let s = match profile {
        Profile::Email => crate::email::format(s),
        Profile::Plain => s.to_string(),
    };
    crate::snippets::restore(&s, &expansions)
}
```

- [ ] **Step 4: Run to verify the cleanup tests pass**

Run: `cmd //c "%TEMP%\murmur-test.cmd" --lib cleanup::tests`
Expected: every pre-existing `cleanup::tests` test passes unedited, plus the 3 new ones. (The binary does not compile yet; `--lib` keeps this step to the library.)

If `email_keeps_spoken_line_breaks` fails on the first break, check what `tidy` hands to `email::format` by asserting on `super::clean(..., Profile::Plain)` for the same input, then fix `email::format` so the expected string holds.

- [ ] **Step 5: Wire the pipeline**

In `src/main.rs:3`, add `context` to the list:

```rust
use murmur_lib::{audio, cleanup, config, context, dictionary, history, model_fetch, snippets, stt, update, vad};
```

In `src/pipeline.rs`, add after `use crate::config::Config;`:

```rust
use crate::context::{self, Profile};
```

In `stop()`, replace

```rust
        let cleaned = {
            let d = self.dict.lock().unwrap_or_else(|e| e.into_inner());
            cleanup::clean(&raw, &d, &self.snippets.current, &self.cfg)
        };
```

with

```rust
        // read now, as the text is about to be pasted: the profile belongs to the window that receives it
        let profile = if self.cfg.format_by_context { context::detect(&self.cfg) } else { Profile::Plain };
        log::info!("profile: {}", profile.name());
        let cleaned = {
            let d = self.dict.lock().unwrap_or_else(|e| e.into_inner());
            cleanup::clean(&raw, &d, &self.snippets.current, &self.cfg, profile)
        };
```

- [ ] **Step 6: Document it in `README.md`**

In the Configuration table, add after the `spoken_commands` row:

```markdown
| `format_by_context` | `true` | Shape the text for the app it lands in. Today that means email: a greeting and a sign-off get their own lines. `false` = the same output everywhere |
| `email_apps` | `["OUTLOOK.EXE", "olk.exe", "thunderbird.exe"]` | Programs treated as email |
| `email_titles` | `["Gmail", "Outlook", "Proton Mail", "Yahoo Mail"]` | Window-title text treated as email, in browsers only |
```

Add this subsection immediately before `### Dictionary`:

````markdown
### Email formatting

When you dictate into an email app, Murmur puts a greeting and a sign-off on their own lines. Say "hi Sarah thanks for the update I'll send notes by Friday thanks Jeff" and you get:

```
Hi Sarah,

Thanks for the update. I'll send notes by Friday.

Thanks,
Jeff
```

Everywhere else the same words are pasted as one paragraph. Only the very start and the very end of a dictation are looked at, so an email dictated in several goes works.

Murmur recognises Outlook and Thunderbird by program name, and Gmail, Outlook, Proton Mail and Yahoo Mail in Chrome, Edge, Firefox, Brave, Opera, Vivaldi and Arc by the tab title. Add your own with `email_apps` and `email_titles`, or turn the feature off with `format_by_context = false`.

Limits: any field in a webmail tab is treated as email, including its search box; and "Thanks, Sarah." at the end is laid out as a sign-off even when you were thanking Sarah.
````

In the `## Privacy` list, add as the last bullet:

```markdown
- To recognise an email app, Murmur reads the name and window title of the program you are dictating into. They are used for that one check and never stored or logged.
```

- [ ] **Step 7: Run the whole suite**

Run: `cmd //c "%TEMP%\murmur-test.cmd"`
Expected: library, binary and integration tests all pass; no warnings in `src/context.rs`, `src/email.rs`, `src/cleanup.rs` or `src/pipeline.rs`.

- [ ] **Step 8: Check the log line carries no title**

Run: `grep -n "log::" src/context.rs src/email.rs`
Expected: no output (neither file logs).
Run: `grep -n "profile:" src/pipeline.rs`
Expected: exactly one line, the `log::info!("profile: {}", profile.name());` added in Step 5.

- [ ] **Step 9: Commit**

```bash
git add src/cleanup.rs src/pipeline.rs src/main.rs README.md
git commit -m "feat: format dictation as email when it lands in an email app

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Release build and smoke test (Jeff)

**Files:** none.

- [ ] **Step 1: Build**

Make a copy of `%TEMP%\murmur-test.cmd` as `%TEMP%\murmur-build.cmd` with the last line changed to `cargo build --release --locked`, and run `cmd //c "%TEMP%\murmur-build.cmd"`.
Expected: `Finished release`. `os error 32` or a link failure on `murmur.exe` means a Murmur is running from `target\release`: stop and ask Jeff to quit it.

- [ ] **Step 2: Hand the smoke test to Jeff**

Do not launch or relaunch Murmur from the session. Report "clean build, unit tests pass, **not smoke tested**" and give Jeff this list. He quits the installed Murmur, runs `C:\Users\JeffLocal\git\murmur\target\release\murmur.exe`, and dictates "hi Sarah thanks for the update I'll review it tomorrow and send notes by Friday thanks Jeff" into:

| Where | Expected |
|---|---|
| A new message in classic Outlook | Greeting line, blank line, body, blank line, `Thanks,` then `Jeff` |
| A Gmail compose box in a browser | Same |
| Notepad | One paragraph, as today |
| Outlook again, after adding `format_by_context = false` to the real `config.toml` and restarting | One paragraph, as today |

Then check the real log, `\\localhost\C$\Users\JeffLocal\AppData\Roaming\Murmur\murmur.log`, shows `profile: email` for the first two, `profile: plain` for the others, and no window title anywhere.

- [ ] **Step 3: Record the result**

If any row differs, capture the exact pasted text and the `raw:` log line (needs `debug_log = true`) before changing code: the usual cause is the recogniser punctuating the greeting or sign-off differently from the test rows, which is a new `email::tests` row first, then a fix.
