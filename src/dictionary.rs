use crate::config::config_dir;
use crate::phonetic;
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};
use std::path::PathBuf;

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

#[derive(Debug, Default, Serialize, Deserialize)]
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
    ("Delgado", &[]),
    ("Orion", &[]),
    ("WildFly", &["wild fly"]),
    ("Jira", &[]),
    ("Hermes", &["hermes", "hermes'"]),
    ("HFS", &["aitch eff ess"]),
    ("QA", &["queue ay"]),
];

fn path() -> PathBuf {
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
        if !self.loaded_cleanly {
            return Err(anyhow!("dictionary was not loaded cleanly; fix dictionary.toml first"));
        }
        std::fs::create_dir_all(config_dir())?;
        let p = path();
        if p.exists() {
            std::fs::copy(&p, p.with_extension("toml.bak")).context("backup dictionary.toml")?;
        }
        let tmp = p.with_extension("toml.tmp");
        std::fs::write(&tmp, self.to_toml()?).context("write dictionary.toml.tmp")?;
        std::fs::rename(&tmp, &p).context("rename dictionary.toml.tmp")
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
        for t in self.terms.iter().filter(|t| t.phonetic) {
            if (phonetic::key(&t.written) == k && close(&t.written))
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
                let (o_core, _) = strip_suffix(o_core);
                let (n_core, _) = strip_suffix(n_core);
                if !o_core.is_empty() && !n_core.is_empty() && o_core.to_ascii_lowercase() != n_core.to_ascii_lowercase() && phonetic::similar(o_core, n_core) {
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
}
