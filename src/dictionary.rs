use crate::config::config_dir;
pub use crate::config::{file_stamp, SaveOutcome, Stamp};
use crate::phonetic;
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Term {
    pub written: String,
    #[serde(default)]
    pub spoken: Vec<String>,
    #[serde(default = "default_true")]
    pub phonetic: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Dictionary {
    #[serde(rename = "term", default)]
    pub terms: Vec<Term>,
    #[serde(skip, default = "default_loaded_cleanly")]
    loaded_cleanly: bool,
}

fn default_loaded_cleanly() -> bool {
    true
}


const SEED: &[(&str, &[&str])] = &[
    ("HAWB", &["hob", "hawb", "haub"]),
    ("MAWB", &["mob", "mawb", "maub"]),
    ("BOL", &["bee oh el"]),
    ("WildFly", &["wild fly"]),
    ("Jira", &["jerry"]),
    ("QA", &["queue ay"]),
    ("EAR", &["ear"]),
];

pub fn path() -> PathBuf {
    config_dir().join("dictionary.toml")
}

/// Split into (leading, core, trailing) where core is the alphabetic-ish token.
fn split_token(tok: &str) -> (&str, &str, &str) {
    let start = tok.find(|c: char| c.is_alphanumeric()).unwrap_or(tok.len());
    let end = tok.rfind(|c: char| c.is_alphanumeric()).map(|i| i + 1).unwrap_or(start);
    (&tok[..start], &tok[start..end], &tok[end..])
}

/// Strip a plural/possessive suffix; returns (stem, suffix).
fn strip_suffix(core: &str) -> (&str, &str) {
    let lower = core.to_ascii_lowercase();
    for suf in ["'s", "es", "s"] {
        if lower.len() > suf.len() + 1 && lower.ends_with(suf) {
            let cut = core.len() - suf.len();
            return (&core[..cut], &core[cut..]);
        }
    }
    (core, "")
}

impl Dictionary {
    pub fn from_terms(terms: Vec<Term>) -> Self {
        Dictionary { terms, loaded_cleanly: true }
    }

    /// An empty dictionary marked as not loaded cleanly; used as a fallback when the real
    /// dictionary failed to load so it can never overwrite the on-disk file.
    pub fn empty_unloaded() -> Self {
        Dictionary { terms: vec![], loaded_cleanly: false }
    }

    pub fn seed() -> Self {
        Dictionary::from_terms(
            SEED.iter()
                .map(|(w, s)| Term { written: w.to_string(), spoken: s.iter().map(|x| x.to_string()).collect(), phonetic: true })
                .collect(),
        )
    }

    pub fn load_or_seed() -> Result<Self> {
        let p = path();
        if p.exists() {
            Self::from_toml(&std::fs::read_to_string(&p).context("read dictionary.toml")?)
        } else {
            let d = Self::seed();
            d.save()?;
            Ok(d)
        }
    }

    pub fn from_toml(s: &str) -> Result<Self> {
        Ok(toml::from_str(s).context("parse dictionary.toml")?)
    }

    pub fn to_toml(&self) -> Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&path())
    }

    pub fn save_to(&self, p: &Path) -> Result<()> {
        if !self.loaded_cleanly {
            return Err(anyhow!("dictionary was not loaded cleanly; fix dictionary.toml first"));
        }
        crate::config::write_with_backup(p, &self.to_toml()?)
    }

    /// Load for editing. The stamp is read before the file, so a write landing mid-read shows
    /// up later as a conflict rather than being silently overwritten.
    pub fn load_stamped(p: &Path) -> Result<(Self, Stamp)> {
        let stamp = file_stamp(p);
        if stamp.is_none() {
            return Ok((Self::from_terms(vec![]), None));
        }
        let d = Self::from_toml(&std::fs::read_to_string(p).context("read dictionary.toml")?)?;
        Ok((d, stamp))
    }

    /// Save only if the file is still the version stamped at load (or at the last save).
    pub fn save_if_unchanged(&self, p: &Path, stamp: Stamp) -> Result<SaveOutcome> {
        if file_stamp(p) != stamp {
            return Ok(SaveOutcome::Conflict);
        }
        self.save_to(p)?;
        Ok(SaveOutcome::Saved(file_stamp(p)))
    }

    /// All (phrase, written) pairs, longest phrase first, lowercased phrase.
    fn phrases(&self) -> Vec<(Vec<String>, &str)> {
        let mut v: Vec<(Vec<String>, &str)> = Vec::new();
        for t in &self.terms {
            v.push((vec![t.written.to_ascii_lowercase()], &t.written));
            for s in &t.spoken {
                v.push((s.to_ascii_lowercase().split_whitespace().map(String::from).collect(), &t.written));
            }
        }
        v.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        v
    }

    fn phonetic_lookup(&self, core: &str) -> Option<&str> {
        if core.is_empty() || phonetic::is_common(core) {
            return None;
        }
        let k = phonetic::key(core);
        if k.is_empty() {
            return None;
        }
        // Double Metaphone codes are short and collide across word lengths
        // (e.g. "hob"/"hobby" both -> "HP"); require equal length so an
        // unrelated longer word doesn't false-match a shorter dictionary entry.
        let close = |candidate: &str| candidate.chars().count() == core.chars().count();
        // An all-caps acronym is spelled out when spoken ("bee oh el"), so its written form
        // is never what the STT heard; matching it phonetically turns any short "PL"-sounding
        // word into "BOL". Acronyms match only through their spoken forms.
        let acronym = |w: &str| w.len() > 1 && w.chars().all(|c| c.is_ascii_uppercase());
        for t in self.terms.iter().filter(|t| t.phonetic) {
            if (!acronym(&t.written) && phonetic::key(&t.written) == k && close(&t.written))
                || t.spoken.iter().any(|s| !s.contains(' ') && phonetic::key(s) == k && close(s))
            {
                return Some(&t.written);
            }
        }
        None
    }

    pub fn apply(&self, text: &str) -> String {
        let tokens: Vec<&str> = text.split(' ').collect();
        let phrases = self.phrases();
        let mut out: Vec<String> = Vec::with_capacity(tokens.len());
        let mut i = 0;
        'outer: while i < tokens.len() {
            // exact, longest first
            for (phrase, written) in &phrases {
                let n = phrase.len();
                if i + n > tokens.len() {
                    continue;
                }
                let mut ok = true;
                for j in 0..n {
                    let (_, core, _) = split_token(tokens[i + j]);
                    let (stem, _) = if j == n - 1 { strip_suffix(core) } else { (core, "") };
                    if stem.to_ascii_lowercase() != phrase[j] {
                        ok = false;
                        break;
                    }
                }
                if ok {
                    let (lead, _, _) = split_token(tokens[i]);
                    let (_, last_core, trail) = split_token(tokens[i + n - 1]);
                    let (_, suffix) = strip_suffix(last_core);
                    out.push(format!("{lead}{written}{suffix}{trail}"));
                    i += n;
                    continue 'outer;
                }
            }
            // phonetic, single token
            let (lead, core, trail) = split_token(tokens[i]);
            let (stem, suffix) = strip_suffix(core);
            if let Some(written) = self.phonetic_lookup(stem) {
                out.push(format!("{lead}{written}{suffix}{trail}"));
            } else {
                out.push(tokens[i].to_string());
            }
            i += 1;
        }
        out.join(" ")
    }

    /// What `learn` would add for this edit, without changing the dictionary.
    pub fn preview_learn(&self, before: &str, after: &str) -> Vec<Term> {
        self.clone().learn(before, after)
    }

    /// Word-level diff of before/after; phonetically similar replace-hunks (<=3 words a side) become terms.
    pub fn learn(&mut self, before: &str, after: &str) -> Vec<Term> {
        let diff = TextDiff::from_words(before, after);
        let mut learned = Vec::new();
        let mut old: Vec<String> = Vec::new();
        let mut new: Vec<String> = Vec::new();
        let flush = |old: &mut Vec<String>, new: &mut Vec<String>, this: &mut Self, learned: &mut Vec<Term>| {
            if !old.is_empty() && !new.is_empty() && old.len() <= 3 && new.len() <= 3 {
                let o = old.join(" ");
                let n = new.join(" ");
                let (_, o_core, _) = split_token(&o);
                let (_, n_core, _) = split_token(&n);
                let (o_stem, o_suf) = strip_suffix(o_core);
                let (n_stem, n_suf) = strip_suffix(n_core);
                // Only strip the written (after) side's suffix when both sides share the
                // same suffix (e.g. "hobs"->"HAWBs"); otherwise keep the written side verbatim
                // (e.g. "hermez"->"Hermes" must not become "Herme").
                let (o_core, n_core) = if !o_suf.is_empty() && o_suf.eq_ignore_ascii_case(n_suf) {
                    (o_stem, n_stem)
                } else {
                    (o_stem, n_core)
                };
                // Never learn a common word as a spoken form: exact matching would then
                // rewrite it everywhere ("all" -> "Call" after a clipped first word).
                if !o_core.is_empty() && !n_core.is_empty() && !phonetic::is_common(o_core) && o_core.to_ascii_lowercase() != n_core.to_ascii_lowercase() && phonetic::similar(o_core, n_core) {
                    let spoken = o_core.to_ascii_lowercase();
                    if let Some(t) = this.terms.iter_mut().find(|t| t.written == n_core) {
                        if !t.spoken.contains(&spoken) {
                            t.spoken.push(spoken);
                        }
                        learned.push(t.clone());
                    } else {
                        let t = Term { written: n_core.to_string(), spoken: vec![spoken], phonetic: true };
                        this.terms.push(t.clone());
                        learned.push(t);
                    }
                }
            }
            old.clear();
            new.clear();
        };
        for change in diff.iter_all_changes() {
            let word = change.value().trim();
            match change.tag() {
                ChangeTag::Equal => flush(&mut old, &mut new, self, &mut learned),
                ChangeTag::Delete => { if !word.is_empty() { old.push(word.to_string()); } }
                ChangeTag::Insert => { if !word.is_empty() { new.push(word.to_string()); } }
            }
        }
        flush(&mut old, &mut new, self, &mut learned);
        learned
    }
}

/// dictionary.toml as the pipeline sees it: reloaded into the shared dictionary when its
/// modified time changes (the editor, Fix-last or a hand edit). A bad edit keeps the last good terms.
pub struct DictionaryFile {
    path: PathBuf,
    stamp: Stamp,
}

impl DictionaryFile {
    /// Starts from the file's current stamp: the shared dictionary was loaded from it at startup.
    pub fn new(path: PathBuf) -> Self {
        let stamp = file_stamp(&path);
        DictionaryFile { path, stamp }
    }

    /// Reload if the file changed since the last check. Returns what was wrong, once per bad edit.
    pub fn refresh(&mut self, shared: &std::sync::Mutex<Dictionary>) -> Option<crate::notice::FileProblem> {
        let stamp = file_stamp(&self.path);
        if stamp == self.stamp {
            return None;
        }
        // deleted: keep what we have; it comes back on the next save or restart
        if stamp.is_none() {
            self.stamp = stamp;
            return None;
        }
        // briefly locked (a scanner, a rename in flight): keep the old stamp so the next dictation retries
        let text = match std::fs::read_to_string(&self.path) {
            Ok(t) => t,
            Err(e) => {
                log::warn!("dictionary.toml not readable yet: {e}");
                return None;
            }
        };
        self.stamp = stamp;
        match Dictionary::from_toml(&text) {
            Ok(d) => {
                log::info!("reloaded {} dictionary terms", d.terms.len());
                *shared.lock().unwrap_or_else(|e| e.into_inner()) = d;
                None
            }
            Err(e) => {
                log::error!("dictionary.toml: {e:#}");
                // after a failed startup there are no previous terms to keep
                let effect = if shared.lock().unwrap_or_else(|e| e.into_inner()).loaded_cleanly {
                    crate::notice::Effect::KeepingTerms
                } else {
                    crate::notice::Effect::CorrectionsOff
                };
                Some(crate::notice::file_problem(&self.path, &e, effect))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict() -> Dictionary {
        Dictionary::from_terms(vec![
            Term { written: "HAWB".into(), spoken: vec!["hob".into(), "hawb".into()], phonetic: true },
            Term { written: "BOL".into(), spoken: vec!["bee oh el".into()], phonetic: true },
            Term { written: "Delgado".into(), spoken: vec![], phonetic: true },
        ])
    }

    #[test]
    fn preview_learn_reports_terms_without_mutating() {
        let d = dict();
        let before = d.terms.clone();
        let l = d.preview_learn("load the hermez truck", "load the Hermes truck");
        assert_eq!(l.iter().map(|t| t.written.as_str()).collect::<Vec<_>>(), ["Hermes"]);
        assert_eq!(d.terms, before);
    }

    #[test]
    fn learn_refuses_common_word_as_spoken_form() {
        let mut d = dict();
        assert!(d.learn("All Mr. Kowalczyk today.", "Call Mr. Kowalczyk today.").is_empty());
        assert_eq!(d.apply("all of them"), "all of them");
    }

    #[test]
    fn acronym_written_form_is_not_phonetically_matched() {
        // "bul" keys to "PL" like "BOL"; it must stay as heard (Jeff said "plus", got "BOL")
        assert_eq!(dict().apply("add the bul now"), "add the bul now");
        assert_eq!(dict().apply("the hob is late"), "the HAWB is late");
    }

    #[test]
    fn exact_single_word_case_insensitive() {
        assert_eq!(dict().apply("the Hob is late"), "the HAWB is late");
    }

    #[test]
    fn exact_multi_word_longest_first() {
        assert_eq!(dict().apply("check the bee oh el now"), "check the BOL now");
    }

    #[test]
    fn word_boundary_respected() {
        assert_eq!(dict().apply("hobby"), "hobby");
    }

    #[test]
    fn phonetic_match_for_uncommon_token() {
        assert_eq!(dict().apply("the haub arrived"), "the HAWB arrived");
    }

    #[test]
    fn phonetic_skips_common_words() {
        assert_eq!(dict().apply("a bowl of soup"), "a bowl of soup");
    }

    #[test]
    fn suffix_carry() {
        assert_eq!(dict().apply("two hobs, the hob's label"), "two HAWBs, the HAWB's label");
    }

    #[test]
    fn punctuation_preserved() {
        assert_eq!(dict().apply("Hob."), "HAWB.");
    }

    #[test]
    fn learn_similar_substitution() {
        let mut d = Dictionary::from_terms(vec![]);
        let learned = d.learn("send the hob to delgato", "send the HAWB to Delgado");
        assert_eq!(learned.len(), 2);
        assert_eq!(learned[0].written, "HAWB");
        assert_eq!(learned[0].spoken, vec!["hob"]);
        assert_eq!(d.apply("hob"), "HAWB");
    }

    #[test]
    fn learn_plural_strips_suffix() {
        let mut d = Dictionary::from_terms(vec![]);
        let learned = d.learn("two hobs", "two HAWBs");
        assert_eq!(learned.len(), 1);
        assert_eq!(learned[0].written, "HAWB");
        assert!(learned[0].spoken.contains(&"hob".to_string()));
        assert_eq!(d.apply("hobs"), "HAWBs");
    }

    #[test]
    fn learn_keeps_written_form_when_suffixes_differ() {
        let mut d = Dictionary::from_terms(vec![]);
        let learned = d.learn("send to hermez", "send to Hermes");
        assert_eq!(learned.len(), 1);
        assert_eq!(learned[0].written, "Hermes");
        assert_eq!(d.apply("hermez"), "Hermes");
    }

    #[test]
    fn learn_ignores_rewording() {
        let mut d = Dictionary::from_terms(vec![]);
        let learned = d.learn("that is very good", "that is excellent");
        assert!(learned.is_empty());
    }

    #[test]
    fn learn_appends_to_existing_term() {
        let mut d = dict();
        d.learn("the haub", "the HAWB");
        let t = d.terms.iter().find(|t| t.written == "HAWB").unwrap();
        assert!(t.spoken.contains(&"haub".to_string()));
        assert_eq!(d.terms.len(), 3);
    }

    #[test]
    fn toml_round_trip() {
        let d = dict();
        let s = d.to_toml().unwrap();
        let back = Dictionary::from_toml(&s).unwrap();
        assert_eq!(back.terms.len(), 3);
        assert_eq!(back.terms[0].written, "HAWB");
    }

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("murmur-dict-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("dictionary.toml")
    }

    #[test]
    fn save_if_unchanged_saves_when_stamp_matches() {
        let p = temp_path("match");
        dict().save_to(&p).unwrap();
        let (mut d, stamp) = Dictionary::load_stamped(&p).unwrap();
        assert!(stamp.is_some());
        d.terms.push(Term { written: "Kowalczyk".into(), spoken: vec![], phonetic: true });
        let out = d.save_if_unchanged(&p, stamp).unwrap();
        assert!(matches!(out, SaveOutcome::Saved(Some(_))));
        let (back, _) = Dictionary::load_stamped(&p).unwrap();
        assert_eq!(back.terms.len(), 4);
        assert!(p.with_extension("toml.bak").exists());
    }

    #[test]
    fn save_if_unchanged_reports_conflict_after_outside_write() {
        let p = temp_path("conflict");
        dict().save_to(&p).unwrap();
        let (d, stamp) = Dictionary::load_stamped(&p).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&p, "[[term]]\nwritten = \"Outside\"\n").unwrap();
        assert_eq!(d.save_if_unchanged(&p, stamp).unwrap(), SaveOutcome::Conflict);
        let (back, _) = Dictionary::load_stamped(&p).unwrap();
        assert_eq!(back.terms[0].written, "Outside");
    }

    #[test]
    fn save_if_unchanged_refuses_when_not_loaded_cleanly() {
        let p = temp_path("unloaded");
        dict().save_to(&p).unwrap();
        let stamp = file_stamp(&p);
        assert!(Dictionary::empty_unloaded().save_if_unchanged(&p, stamp).is_err());
        assert_eq!(Dictionary::load_stamped(&p).unwrap().0.terms.len(), 3);
    }

    #[test]
    fn load_stamped_missing_file_is_empty_with_no_stamp() {
        let p = temp_path("missing");
        let (d, stamp) = Dictionary::load_stamped(&p).unwrap();
        assert!(d.terms.is_empty());
        assert_eq!(stamp, None);
    }

    #[test]
    fn a_broken_reload_after_a_failed_startup_says_corrections_are_off() {
        let p = temp_path("reload-unloaded");
        std::fs::write(&p, "[[term]\nbroken").unwrap();
        let shared = std::sync::Mutex::new(Dictionary::empty_unloaded());
        let mut f = DictionaryFile::new(p.clone());
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&p, "# a\n# b\n# c\nbroken\n").unwrap();
        // no good terms were ever loaded, so none are kept
        assert_eq!(f.refresh(&shared).unwrap().effect, crate::notice::Effect::CorrectionsOff);
    }

    #[test]
    fn a_broken_reload_names_the_line_and_keeps_the_terms() {
        let p = temp_path("reload-line");
        dict().save_to(&p).unwrap();
        let shared = std::sync::Mutex::new(dict());
        let mut f = DictionaryFile::new(p.clone());
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&p, "# a\n# b\n# c\nbroken\n").unwrap();
        let got = f.refresh(&shared).unwrap();
        assert_eq!(got.line, Some(4));
        assert_eq!(got.effect, crate::notice::Effect::KeepingTerms);
        assert_eq!(got.path, p);
        // deleted while running: nothing to report
        std::fs::remove_file(&p).unwrap();
        assert_eq!(f.refresh(&shared), None);
    }

    #[test]
    fn dictionary_file_reloads_on_change_and_keeps_last_good() {
        let p = temp_path("reload");
        dict().save_to(&p).unwrap();
        let shared = std::sync::Mutex::new(dict());
        let mut f = DictionaryFile::new(p.clone());
        // unchanged since new(): not re-read
        assert_eq!(f.refresh(&shared), None);
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&p, "[[term]]\nwritten = \"Kowalczyk\"\nspoken = [\"kowalski\"]\n").unwrap();
        assert_eq!(f.refresh(&shared), None);
        assert_eq!(shared.lock().unwrap().apply("call kowalski"), "call Kowalczyk");
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&p, "[[term]\nbroken").unwrap();
        assert!(f.refresh(&shared).is_some());
        // reported once per bad version
        assert_eq!(f.refresh(&shared), None);
        assert_eq!(shared.lock().unwrap().terms[0].written, "Kowalczyk");
    }

    #[test]
    fn dictionary_file_revives_after_failed_startup() {
        let p = temp_path("revive");
        std::fs::write(&p, "[[term]\nbroken").unwrap();
        let shared = std::sync::Mutex::new(Dictionary::empty_unloaded());
        let mut f = DictionaryFile::new(p.clone());
        std::thread::sleep(std::time::Duration::from_millis(20));
        dict().save_to(&p).unwrap();
        assert_eq!(f.refresh(&shared), None);
        let d = shared.lock().unwrap();
        assert_eq!(d.terms.len(), 3);
        // the reloaded copy is saveable again
        d.save_to(&p).unwrap();
    }

    // macOS has no share-mode locks for a scanner to hold the file with
    #[cfg(windows)]
    #[test]
    fn unreadable_file_is_retried_without_a_notice() {
        use std::os::windows::fs::OpenOptionsExt;
        let p = temp_path("locked");
        dict().save_to(&p).unwrap();
        let shared = std::sync::Mutex::new(dict());
        let mut f = DictionaryFile::new(p.clone());
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&p, "[[term]]\nwritten = \"Kowalczyk\"\n").unwrap();
        // another process (e.g. a virus scanner) holds the file with no sharing
        let lock = std::fs::OpenOptions::new().read(true).share_mode(0).open(&p).unwrap();
        assert_eq!(f.refresh(&shared), None);
        assert_eq!(shared.lock().unwrap().terms.len(), 3);
        drop(lock);
        assert_eq!(f.refresh(&shared), None);
        assert_eq!(shared.lock().unwrap().terms[0].written, "Kowalczyk");
    }

    #[test]
    fn missing_file_keeps_current_terms() {
        let p = temp_path("gone");
        dict().save_to(&p).unwrap();
        let shared = std::sync::Mutex::new(dict());
        let mut f = DictionaryFile::new(p.clone());
        std::fs::remove_file(&p).unwrap();
        assert_eq!(f.refresh(&shared), None);
        assert_eq!(shared.lock().unwrap().terms.len(), 3);
    }
}
