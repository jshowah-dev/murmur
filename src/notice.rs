//! What Murmur says when something goes wrong: what happened, what Murmur did about it, and
//! what you can do. Text only; the tray and the pill show it.

use std::path::{Path, PathBuf};

/// Windows cuts a balloon's text at 255 UTF-16 units and its title at 63.
pub const BODY_MAX: usize = 255;
pub const TITLE_MAX: usize = 63;

/// Said above the pill when the mic won't open as you press the talk key.
pub const MIC: &str = "Can't open the microphone";
/// Said above the pill when the speech model fails to load for a dictation.
pub const MODEL_LOAD: &str = "Couldn't load the speech model. Try again.";
/// Said above the pill when the pipeline panicked and restarted.
pub const RESTARTED: &str = "Something went wrong. Try again.";

/// Longest a file's reason runs: alone in a balloon, and as one of several.
const REASON_ONE: usize = 150;
const REASON_MANY: usize = 60;

/// What Murmur does while a file can't be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Defaults,
    CorrectionsOff,
    SnippetsOff,
    KeepingTerms,
    KeepingSnippets,
}

impl Effect {
    fn long(self) -> &'static str {
        match self {
            Effect::Defaults => "Using default settings for now.",
            Effect::CorrectionsOff => "Corrections are off until it's fixed.",
            Effect::SnippetsOff => "Snippets are off.",
            Effect::KeepingTerms => "Still using your previous terms.",
            Effect::KeepingSnippets => "Still using your previous snippets.",
        }
    }

    fn short(self) -> &'static str {
        match self {
            Effect::Defaults => "Using defaults.",
            Effect::CorrectionsOff => "Corrections off.",
            Effect::SnippetsOff => "Snippets off.",
            Effect::KeepingTerms => "Previous terms kept.",
            Effect::KeepingSnippets => "Previous snippets kept.",
        }
    }
}

/// A settings file Murmur couldn't use: which, where in it, why, and what Murmur did instead.
#[derive(Debug, Clone, PartialEq)]
pub struct FileProblem {
    pub path: PathBuf,
    pub line: Option<usize>,
    pub reason: String,
    pub effect: Effect,
}

impl FileProblem {
    fn name(&self) -> String {
        self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| self.path.display().to_string())
    }

    /// "Line 4: expected `=`." or just the reason.
    fn cause(&self, max: usize) -> String {
        let reason = cut(&self.reason, max);
        match self.line {
            Some(n) => format!("Line {n}: {reason}"),
            None => reason,
        }
    }
}

/// A corner balloon, and the files a click on it opens (none: a click does nothing).
#[derive(Debug, Clone, PartialEq)]
pub struct Balloon {
    pub title: String,
    pub body: String,
    pub open: Vec<PathBuf>,
}

/// The 1-based line holding byte `offset` of `text`.
pub fn line_of(text: &str, offset: usize) -> usize {
    text.as_bytes()[..offset.min(text.len())].iter().filter(|&&b| b == b'\n').count() + 1
}

/// What went wrong with the file at `path`: toml's own message and line when it's a parse
/// error, otherwise the root cause (an OS error, say). The line is found by reading the file again.
pub fn file_problem(path: &Path, e: &anyhow::Error, effect: Effect) -> FileProblem {
    let parse = e.chain().find_map(|c| c.downcast_ref::<toml::de::Error>());
    let (line, reason) = match parse {
        Some(t) => {
            let line = t.span().and_then(|s| std::fs::read_to_string(path).ok().map(|text| line_of(&text, s.start)));
            (line, t.message().to_string())
        }
        None => (None, e.root_cause().to_string()),
    };
    FileProblem { path: path.to_path_buf(), line, reason: sentence(&reason), effect }
}

/// The startup balloon for one or more files that couldn't be read.
pub fn files_balloon(problems: &[FileProblem]) -> Balloon {
    let open = problems.iter().map(|p| p.path.clone()).collect();
    if let [p] = problems {
        let body = fit(&[p.cause(REASON_ONE), p.effect.long().into()], " ", "Click to open it.");
        return Balloon { title: title(&format!("Murmur couldn't read {}", p.name())), body, open };
    }
    let lines: Vec<String> = problems
        .iter()
        .map(|p| {
            let at = p.line.map(|n| format!(" line {n}")).unwrap_or_default();
            format!("{}{at}: {} {}", p.name(), cut(&p.reason, REASON_MANY), p.effect.short())
        })
        .collect();
    let body = fit(&lines, "\n", "Click to open them.");
    Balloon { title: format!("Murmur couldn't read {} files", problems.len()), body, open }
}

/// A file edited while Murmur runs turned out broken; the previous version stays in use.
pub fn reload_balloon(p: &FileProblem) -> Balloon {
    let head = match p.line {
        Some(_) => format!("{} has a mistake", p.name()),
        None => format!("Murmur couldn't read {}", p.name()),
    };
    let body = fit(&[p.cause(REASON_ONE), p.effect.long().into()], " ", "Click to open it.");
    Balloon { title: title(&head), body, open: vec![p.path.clone()] }
}

/// A custom `model_dir` that doesn't exist. A click opens config.toml, where it's set.
pub fn model_missing_balloon(model_dir: &Path, config: &Path) -> Balloon {
    let rest = " (model_dir in config.toml). Click to open config.toml.";
    let room = BODY_MAX - units("Nothing at ") - units(rest);
    let body = format!("Nothing at {}{rest}", cut_front(&model_dir.display().to_string(), room));
    Balloon { title: "Speech model not found".into(), body, open: vec![config.to_path_buf()] }
}

/// The mic wouldn't open at startup, after a pause, or after the device went away.
pub fn mic_balloon(key: &str) -> Balloon {
    let body = format!("Check one is connected and allowed in Windows privacy settings. Murmur tries again when you press {key}.");
    Balloon { title: "Can't open the microphone".into(), body: cut(&body, BODY_MAX), open: vec![] }
}

/// A newer speech model can be downloaded from the tray.
pub fn upgrade_balloon(mb: u64) -> Balloon {
    Balloon { title: format!("A new speech model is available ({mb} MB)"), body: "Download from the tray menu.".into(), open: vec![] }
}

/// Startup's balloons in the order to show them. Each replaces the last, so the one that
/// matters most goes last: an offer, then files read wrong, then no mic, then no model.
pub fn startup(problems: &[FileProblem], mic: Option<Balloon>, model_missing: Option<Balloon>, upgrade: Option<Balloon>) -> Vec<Balloon> {
    let mut out: Vec<Balloon> = upgrade.into_iter().collect();
    if !problems.is_empty() {
        out.push(files_balloon(problems));
    }
    out.extend(mic);
    out.extend(model_missing);
    out
}

/// Fix-last's answer when dictionary.toml can't be read, so nothing could be learned.
pub fn not_learned(p: &FileProblem) -> String {
    match p.line {
        Some(n) => format!("Not learned: {} line {n} has a mistake", p.name()),
        None => format!("Not learned: {} can't be read", p.name()),
    }
}

/// One line, ending in a full stop; an OS error's "(os error 5)" already closes its sentence.
fn sentence(s: &str) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.ends_with(['.', '!', '?', ')']) { s } else { format!("{s}.") }
}

fn units(s: &str) -> usize {
    s.encode_utf16().count()
}

/// `s` cut to `max` UTF-16 units, ending in an ellipsis when cut.
fn cut(s: &str, max: usize) -> String {
    if units(s) <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut n = 0;
    for c in s.chars() {
        if n + c.len_utf16() > max - 1 {
            break;
        }
        n += c.len_utf16();
        out.push(c);
    }
    out.push('…');
    out
}

/// `s` cut to `max` UTF-16 units from the front, keeping its end (a path's folder name).
fn cut_front(s: &str, max: usize) -> String {
    if units(s) <= max {
        return s.to_string();
    }
    let mut kept: Vec<char> = Vec::new();
    let mut n = 0;
    for c in s.chars().rev() {
        if n + c.len_utf16() > max - 1 {
            break;
        }
        n += c.len_utf16();
        kept.push(c);
    }
    std::iter::once('…').chain(kept.into_iter().rev()).collect()
}

fn title(s: &str) -> String {
    cut(s, TITLE_MAX)
}

/// `parts` joined by `sep`, then `click` if it still fits; cut to the limit if even the parts don't.
fn fit(parts: &[String], sep: &str, click: &str) -> String {
    let body = parts.join(sep);
    let with = format!("{body}{sep}{click}");
    if units(&with) <= BODY_MAX { with } else { cut(&body, BODY_MAX) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp(name: &str, text: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("murmur-notice-{}-{name}", std::process::id()));
        std::fs::write(&p, text).unwrap();
        p
    }

    fn toml_error(text: &str) -> anyhow::Error {
        anyhow::Error::from(toml::from_str::<toml::Table>(text).unwrap_err()).context("parse config.toml")
    }

    fn problem(name: &str, line: Option<usize>, effect: Effect) -> FileProblem {
        FileProblem { path: PathBuf::from(format!("C:\\Users\\a\\AppData\\Roaming\\Murmur\\{name}")), line, reason: "expected `=`.".into(), effect }
    }

    fn units(s: &str) -> usize {
        s.encode_utf16().count()
    }

    #[test]
    fn line_of_counts_lines_up_to_the_offset() {
        assert_eq!(line_of("a = 1\nb\n", 0), 1);
        assert_eq!(line_of("a = 1\nb\n", 6), 2);
        assert_eq!(line_of("a = 1\r\nb = 2\r\nc\r\n", 14), 3);
        // past the end, and just after a two-byte character: no panic
        assert_eq!(line_of("é\nx", 3), 2);
        assert_eq!(line_of("é\nx", 99), 2);
        assert_eq!(line_of("é", 1), 1);
    }

    #[test]
    fn a_toml_error_gives_its_message_and_line_not_the_context() {
        let text = "# a\n# b\n# c\nbroken\n";
        let p = temp("line4.toml", text);
        let e = toml_error(text);
        let want = e.chain().find_map(|c| c.downcast_ref::<toml::de::Error>()).unwrap().message().to_string();
        let got = file_problem(&p, &e, Effect::Defaults);
        assert_eq!(got.line, Some(4));
        assert!(got.reason.starts_with(want.trim_end_matches('.')), "{:?} vs {want:?}", got.reason);
        assert!(!got.reason.contains("parse config.toml"));
        assert!(got.reason.ends_with('.'));
        assert_eq!(got.path, p);
        assert_eq!(got.effect, Effect::Defaults);
    }

    #[test]
    fn a_non_toml_error_gives_its_root_cause_and_no_line() {
        let io = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "Access is denied. (os error 5)");
        let e = anyhow::Error::from(io).context("read config.toml");
        let got = file_problem(std::path::Path::new("C:\\nowhere\\config.toml"), &e, Effect::Defaults);
        assert_eq!(got.line, None);
        assert_eq!(got.reason, "Access is denied. (os error 5)");
    }

    #[test]
    fn one_broken_file_says_what_murmur_did() {
        let b = files_balloon(&[problem("config.toml", Some(4), Effect::Defaults)]);
        assert_eq!(b.title, "Murmur couldn't read config.toml");
        assert_eq!(b.body, "Line 4: expected `=`. Using default settings for now. Click to open it.");
        assert_eq!(b.open, vec![PathBuf::from("C:\\Users\\a\\AppData\\Roaming\\Murmur\\config.toml")]);
        let b = files_balloon(&[problem("dictionary.toml", Some(12), Effect::CorrectionsOff)]);
        assert_eq!(b.body, "Line 12: expected `=`. Corrections are off until it's fixed. Click to open it.");
        let mut p = problem("snippets.toml", None, Effect::SnippetsOff);
        p.reason = "Access is denied. (os error 5)".into();
        assert_eq!(files_balloon(&[p]).body, "Access is denied. (os error 5) Snippets are off. Click to open it.");
    }

    #[test]
    fn several_broken_files_share_one_balloon() {
        let b = files_balloon(&[problem("config.toml", Some(4), Effect::Defaults), problem("dictionary.toml", Some(12), Effect::CorrectionsOff)]);
        assert_eq!(b.title, "Murmur couldn't read 2 files");
        assert_eq!(
            b.body,
            "config.toml line 4: expected `=`. Using defaults.\ndictionary.toml line 12: expected `=`. Corrections off.\nClick to open them."
        );
        assert_eq!(b.open.len(), 2);
    }

    #[test]
    fn a_reload_problem_says_the_previous_ones_are_kept() {
        let b = reload_balloon(&problem("dictionary.toml", Some(4), Effect::KeepingTerms));
        assert_eq!(b.title, "dictionary.toml has a mistake");
        assert_eq!(b.body, "Line 4: expected `=`. Still using your previous terms. Click to open it.");
        let b = reload_balloon(&problem("snippets.toml", None, Effect::KeepingSnippets));
        assert_eq!(b.title, "Murmur couldn't read snippets.toml");
        assert_eq!(b.body, "expected `=`. Still using your previous snippets. Click to open it.");
    }

    #[test]
    fn model_missing_and_mic_say_what_to_do() {
        let cfg = PathBuf::from("C:\\m\\config.toml");
        let b = model_missing_balloon(std::path::Path::new("D:\\models\\parakeet"), &cfg);
        assert_eq!(b.title, "Speech model not found");
        assert_eq!(b.body, "Nothing at D:\\models\\parakeet (model_dir in config.toml). Click to open config.toml.");
        assert_eq!(b.open, vec![cfg]);
        let b = mic_balloon("Right Ctrl");
        assert_eq!(b.title, "Can't open the microphone");
        assert_eq!(b.body, "Check one is connected and allowed in Windows privacy settings. Murmur tries again when you press Right Ctrl.");
        assert!(b.open.is_empty());
        let b = upgrade_balloon(640);
        assert_eq!(b.title, "A new speech model is available (640 MB)");
        assert_eq!(b.body, "Download from the tray menu.");
        assert!(b.open.is_empty());
    }

    #[test]
    fn every_balloon_fits_windows_limits() {
        let long = "🎤".repeat(300);
        let mut ps: Vec<FileProblem> = ["config.toml", "dictionary.toml", "snippets.toml"]
            .iter()
            .map(|n| FileProblem { reason: long.clone(), ..problem(n, Some(4), Effect::Defaults) })
            .collect();
        let deep = PathBuf::from(format!("C:\\{}\\parakeet", "deep\\".repeat(80)));
        let all = [
            files_balloon(&ps[..1]),
            files_balloon(&ps),
            reload_balloon(&ps[0]),
            model_missing_balloon(&deep, &PathBuf::from("C:\\m\\config.toml")),
            mic_balloon(&long),
        ];
        for b in &all {
            assert!(units(&b.body) <= BODY_MAX, "{} units: {}", units(&b.body), b.body);
            assert!(units(&b.title) <= TITLE_MAX, "{}", b.title);
        }
        // a cut ends in an ellipsis; the path keeps its end, where the folder name is
        assert!(all[0].body.ends_with('…') || all[0].body.ends_with("Click to open it."));
        assert!(all[3].body.contains("parakeet (model_dir in config.toml). Click to open config.toml."));
        // many files: each reason is trimmed first, so the click line survives
        ps.truncate(2);
        assert!(files_balloon(&ps).body.ends_with("Click to open them."));
    }

    #[test]
    fn startup_shows_what_matters_most_last() {
        let files = [problem("config.toml", Some(4), Effect::Defaults)];
        let mic = mic_balloon("Right Ctrl");
        let model = model_missing_balloon(std::path::Path::new("D:\\x"), std::path::Path::new("C:\\m\\config.toml"));
        let up = upgrade_balloon(640);
        let shown = startup(&files, Some(mic.clone()), Some(model.clone()), Some(up.clone()));
        // each balloon replaces the last, so the order runs from least to most important
        assert_eq!(shown, vec![up.clone(), files_balloon(&files), mic, model]);
        assert_eq!(startup(&[], None, None, Some(up.clone())), vec![up]);
        assert!(startup(&[], None, None, None).is_empty());
    }

    #[test]
    fn not_learned_names_the_file_and_line() {
        assert_eq!(not_learned(&problem("dictionary.toml", Some(12), Effect::CorrectionsOff)), "Not learned: dictionary.toml line 12 has a mistake");
        assert_eq!(not_learned(&problem("dictionary.toml", None, Effect::CorrectionsOff)), "Not learned: dictionary.toml can't be read");
    }

    #[test]
    fn pill_texts() {
        assert_eq!(MIC, "Can't open the microphone");
        assert_eq!(MODEL_LOAD, "Couldn't load the speech model. Try again.");
        assert_eq!(RESTARTED, "Something went wrong. Try again.");
    }
}
