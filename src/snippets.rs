use crate::config::{config_dir, file_stamp, SaveOutcome, Stamp};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Snippet {
    pub trigger: String,
    pub text: String,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Snippets {
    #[serde(rename = "snippet", default, skip_serializing_if = "Vec::is_empty")]
    pub snippets: Vec<Snippet>,
}

const SEED: &str = r#"# Murmur snippets: say the trigger phrase anywhere in a dictation and the text is pasted instead.
# Triggers match whole words, ignoring case and punctuation. Text is pasted exactly as written;
# use a """multi-line""" string for more than one line. Changes apply on the next dictation.
#
# [[snippet]]
# trigger = "my signature"
# text = """
# Best,
# Your Name"""
"#;

pub fn path() -> PathBuf {
    config_dir().join("snippets.toml")
}

/// Create snippets.toml with a commented example if it does not exist yet.
pub fn ensure_file() -> Result<()> {
    let p = path();
    if !p.exists() {
        std::fs::create_dir_all(config_dir())?;
        std::fs::write(&p, SEED).context("write snippets.toml")?;
    }
    Ok(())
}

impl Snippets {
    pub fn from_toml(s: &str) -> Result<Self> {
        Ok(toml::from_str(s).context("parse snippets.toml")?)
    }

    pub fn to_toml(&self) -> Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }

    /// Load for editing. The stamp is read before the file, so a write landing mid-read shows
    /// up later as a conflict rather than being silently overwritten.
    pub fn load_stamped(p: &Path) -> Result<(Self, Stamp)> {
        let stamp = file_stamp(p);
        if stamp.is_none() {
            return Ok((Self::default(), None));
        }
        let s = Self::from_toml(&std::fs::read_to_string(p).context("read snippets.toml")?)?;
        Ok((s, stamp))
    }

    pub fn save_to(&self, p: &Path) -> Result<()> {
        crate::config::write_with_backup(p, &self.to_toml()?)
    }

    /// Save only if the file is still the version stamped at load (or at the last save).
    pub fn save_if_unchanged(&self, p: &Path, stamp: Stamp) -> Result<SaveOutcome> {
        crate::config::write_if_unchanged(p, stamp, &self.to_toml()?)
    }

    /// Replace trigger phrases with placeholder characters that later cleanup steps leave alone.
    /// Returns the marked text and the expansion for each placeholder, for `restore`.
    pub fn mark(&self, text: &str) -> (String, Vec<String>) {
        let mut triggers: Vec<(Vec<String>, &str)> = self
            .snippets
            .iter()
            .map(|s| (words(&s.trigger), s.text.as_str()))
            .filter(|(words, _)| !words.is_empty())
            .collect();
        triggers.sort_by_key(|(words, _)| std::cmp::Reverse(words.len()));

        let tokens: Vec<&str> = text.split(' ').collect();
        let mut out: Vec<String> = Vec::new();
        let mut expansions: Vec<String> = Vec::new();
        let mut i = 0;
        'tokens: while i < tokens.len() {
            for (words, expansion) in &triggers {
                let n = words.len();
                if i + n <= tokens.len() && tokens[i..i + n].iter().zip(words).all(|(t, w)| bare(t) == *w) {
                    let lead = &tokens[i][..tokens[i].len() - tokens[i].trim_start_matches(|c: char| !c.is_alphanumeric()).len()];
                    let last = tokens[i + n - 1];
                    let trail = &last[last.trim_end_matches(|c: char| !c.is_alphanumeric()).len()..];
                    let mut tok = lead.to_string();
                    tok.push(char::from_u32(PLACEHOLDER + expansions.len() as u32).expect("private-use char"));
                    // a multi-line expansion is a block (a signature): the model's "." after it is noise
                    if !expansion.contains('\n') {
                        tok.push_str(trail);
                    }
                    out.push(tok);
                    expansions.push(expansion.to_string());
                    i += n;
                    continue 'tokens;
                }
            }
            out.push(tokens[i].to_string());
            i += 1;
        }
        (out.join(" "), expansions)
    }
}

/// Start of the Unicode private-use area; placeholder k is PLACEHOLDER + k.
const PLACEHOLDER: u32 = 0xE000;

/// Lowercased word with punctuation stripped, so "Address." matches "address".
fn bare(tok: &str) -> String {
    tok.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()
}

/// The words a trigger matches: lower-cased, punctuation stripped, empties dropped.
pub fn words(trigger: &str) -> Vec<String> {
    trigger.split_whitespace().map(bare).filter(|w| !w.is_empty()).collect()
}

/// Swap each placeholder left by `Snippets::mark` for its expansion.
pub fn restore(text: &str, expansions: &[String]) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match (ch as u32).checked_sub(PLACEHOLDER).and_then(|k| expansions.get(k as usize)) {
            Some(e) => out.push_str(e),
            None => out.push(ch),
        }
    }
    out
}

/// snippets.toml, reloaded when its modified time changes; a bad edit keeps the last good set.
pub struct SnippetFile {
    path: PathBuf,
    mtime: Option<SystemTime>,
    pub current: Snippets,
}

impl SnippetFile {
    pub fn new(path: PathBuf) -> Self {
        SnippetFile { path, mtime: None, current: Snippets::default() }
    }

    /// Reload if the file changed since the last check. Returns what was wrong, once per bad edit.
    pub fn refresh(&mut self) -> Option<crate::notice::FileProblem> {
        let mtime = std::fs::metadata(&self.path).and_then(|m| m.modified()).ok();
        if mtime == self.mtime {
            return None;
        }
        self.mtime = mtime;
        if mtime.is_none() {
            self.current = Snippets::default();
            return None;
        }
        match std::fs::read_to_string(&self.path).map_err(anyhow::Error::from).and_then(|s| Snippets::from_toml(&s)) {
            Ok(s) => {
                log::info!("loaded {} snippets", s.snippets.len());
                self.current = s;
                None
            }
            Err(e) => {
                log::error!("snippets.toml: {e:#}");
                Some(crate::notice::file_problem(&self.path, &e, crate::notice::Effect::KeepingSnippets))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_broken_reload_names_the_line_and_keeps_the_snippets() {
        let p = std::env::temp_dir().join(format!("murmur-snippets-{}-reload-line.toml", std::process::id()));
        std::fs::write(&p, "# a\n# b\n# c\nbroken\n").unwrap();
        let mut f = SnippetFile::new(p.clone());
        let got = f.refresh().unwrap();
        assert_eq!(got.line, Some(4));
        assert_eq!(got.effect, crate::notice::Effect::KeepingSnippets);
        assert_eq!(f.refresh(), None);
        let _ = std::fs::remove_file(&p);
    }

    fn set(pairs: &[(&str, &str)]) -> Snippets {
        Snippets { snippets: pairs.iter().map(|(t, x)| Snippet { trigger: t.to_string(), text: x.to_string() }).collect() }
    }

    fn expand(s: &Snippets, text: &str) -> String {
        let (marked, exp) = s.mark(text);
        restore(&marked, &exp)
    }

    #[test]
    fn matches_mid_sentence_ignoring_case_and_punctuation() {
        let s = set(&[("my email address", "me@example.com")]);
        assert_eq!(expand(&s, "Send it to My email, address."), "Send it to me@example.com.");
    }

    #[test]
    fn partial_words_do_not_match() {
        let s = set(&[("my email address", "me@example.com")]);
        assert_eq!(expand(&s, "check my email addresses"), "check my email addresses");
    }

    #[test]
    fn longest_trigger_wins() {
        let s = set(&[("my email", "short"), ("my email address", "long")]);
        assert_eq!(expand(&s, "my email address and my email"), "long and short");
    }

    #[test]
    fn multi_line_drops_trailing_punctuation() {
        let s = set(&[("my signature", "Best,\nJeff")]);
        assert_eq!(expand(&s, "Thanks, my signature."), "Thanks, Best,\nJeff");
    }

    #[test]
    fn placeholder_is_not_alphanumeric_or_whitespace() {
        let s = set(&[("sig", "x")]);
        let (marked, exp) = s.mark("sig");
        assert_eq!(exp, vec!["x".to_string()]);
        assert!(marked.chars().all(|c| !c.is_alphanumeric() && !c.is_whitespace()));
    }

    #[test]
    fn parses_toml() {
        let s = Snippets::from_toml("[[snippet]]\ntrigger = \"sig\"\ntext = \"\"\"\nA\nB\"\"\"\n").unwrap();
        assert_eq!(s.snippets, vec![Snippet { trigger: "sig".into(), text: "A\nB".into() }]);
        assert!(Snippets::from_toml(SEED).unwrap().snippets.is_empty());
    }

    #[test]
    fn bad_edit_keeps_last_good_set_and_reports_once() {
        let dir = std::env::temp_dir().join(format!("murmur-snippets-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("snippets.toml");
        std::fs::write(&p, "[[snippet]]\ntrigger = \"sig\"\ntext = \"x\"\n").unwrap();
        let mut f = SnippetFile::new(p.clone());
        assert_eq!(f.refresh(), None);
        assert_eq!(f.current.snippets.len(), 1);
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&p, "[[snippet]\nbroken").unwrap();
        assert!(f.refresh().is_some());
        assert_eq!(f.refresh(), None);
        assert_eq!(f.current.snippets.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("murmur-snip-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("snippets.toml")
    }

    #[test]
    fn multi_line_text_round_trips_exactly() {
        let s = set(&[("my signature", "\nBest,\n\nJeff\n"), ("quote", "say \"\"\" then 'x'"), ("sig", "one line")]);
        let toml = s.to_toml().unwrap();
        assert!(toml.contains("\"\"\""), "multi-line text should be a \"\"\" block:\n{toml}");
        assert_eq!(Snippets::from_toml(&toml).unwrap().snippets, s.snippets);
    }

    #[test]
    fn empty_set_saves_as_an_empty_file_that_loads() {
        let toml = Snippets::default().to_toml().unwrap();
        assert!(Snippets::from_toml(&toml).unwrap().snippets.is_empty());
    }

    #[test]
    fn words_match_what_mark_matches() {
        assert_eq!(words("  My email, address. "), vec!["my", "email", "address"]);
        assert!(words(" !? ").is_empty());
    }

    #[test]
    fn load_stamped_missing_file_is_empty_with_no_stamp() {
        let p = temp_path("missing");
        let (s, stamp) = Snippets::load_stamped(&p).unwrap();
        assert!(s.snippets.is_empty());
        assert_eq!(stamp, None);
    }

    #[test]
    fn save_if_unchanged_saves_then_conflicts_after_an_outside_write() {
        let p = temp_path("conflict");
        let s = set(&[("sig", "A\nB")]);
        let SaveOutcome::Saved(stamp) = s.save_if_unchanged(&p, None).unwrap() else { panic!("first save conflicted") };
        let (back, loaded) = Snippets::load_stamped(&p).unwrap();
        assert_eq!(back.snippets, s.snippets);
        assert_eq!(loaded, stamp);
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&p, "[[snippet]]\ntrigger = \"outside\"\ntext = \"x\"\n").unwrap();
        assert_eq!(s.save_if_unchanged(&p, stamp).unwrap(), SaveOutcome::Conflict);
        assert_eq!(Snippets::load_stamped(&p).unwrap().0.snippets[0].trigger, "outside");
    }
}
