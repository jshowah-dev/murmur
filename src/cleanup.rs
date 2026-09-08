#![allow(dead_code)]

use crate::config::Config;
use crate::dictionary::Dictionary;

/// Join VAD-segment transcripts; capitalise after a segment that ended a sentence.
pub fn join_parts(parts: &[String]) -> String {
    let mut out = String::new();
    for p in parts.iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        if out.is_empty() {
            out.push_str(p);
            continue;
        }
        let ends_sentence = out.ends_with(['.', '!', '?']);
        out.push(' ');
        if ends_sentence {
            let mut cs = p.chars();
            if let Some(f) = cs.next() {
                out.extend(f.to_uppercase());
                out.push_str(cs.as_str());
            }
        } else {
            out.push_str(p);
        }
    }
    out
}

fn strip_fillers(text: &str, fillers: &[String]) -> String {
    let mut out: Vec<String> = Vec::new();
    for tok in text.split(' ') {
        let core: String = tok.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_ascii_lowercase();
        if fillers.iter().any(|f| f == &core) {
            // keep the filler's trailing sentence punctuation on the previous token
            if tok.ends_with(['.', '!', '?']) {
                if let Some(last) = out.last_mut() {
                    if !last.ends_with(['.', '!', '?']) {
                        last.push_str(&tok[tok.len() - 1..]);
                    }
                }
            } else if tok.ends_with(',') {
                // drop a redundant trailing comma left on the previous token
                if let Some(last) = out.last_mut() {
                    if last.ends_with(',') {
                        last.pop();
                    }
                }
            }
            continue;
        }
        out.push(tok.to_string());
    }
    out.join(" ")
}

fn apply_commands(text: &str) -> String {
    let tokens: Vec<&str> = text.split(' ').collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let cur = tokens[i].to_ascii_lowercase();
        if cur == "new" && i + 1 < tokens.len() {
            let next = tokens[i + 1].to_ascii_lowercase();
            if next == "line" {
                out.push("\n".to_string());
                i += 2;
                continue;
            } else if next == "paragraph" {
                out.push("\n\n".to_string());
                i += 2;
                continue;
            }
        }
        out.push(tokens[i].to_string());
        i += 1;
    }
    let mut result = String::new();
    for (idx, tok) in out.iter().enumerate() {
        if idx == 0 || tok.starts_with('\n') || result.ends_with('\n') {
            result.push_str(tok);
        } else {
            result.push(' ');
            result.push_str(tok);
        }
    }
    result
}

fn tidy(text: &str) -> String {
    // collapse runs of spaces, drop space before punctuation, capitalise sentence starts
    let mut s = text.split(' ').filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" ");
    for p in [",", ".", "!", "?", ";", ":"] {
        s = s.replace(&format!(" {p}"), p);
    }
    let mut out = String::with_capacity(s.len());
    let mut cap = true;
    for ch in s.chars() {
        if cap && ch.is_alphabetic() {
            out.extend(ch.to_uppercase());
            cap = false;
        } else {
            out.push(ch);
        }
        if matches!(ch, '.' | '!' | '?' | '\n') {
            cap = true;
        } else if !ch.is_whitespace() {
            cap = false;
        }
    }
    out
}

pub fn clean(text: &str, dict: &Dictionary, cfg: &Config) -> String {
    let mut s = strip_fillers(text, &cfg.fillers);
    s = dict.apply(&s);
    if cfg.spoken_commands {
        s = apply_commands(&s);
    }
    tidy(&s).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::dictionary::{Dictionary, Term};

    fn env() -> (Dictionary, Config) {
        (
            Dictionary::from_terms(vec![Term { written: "HAWB".into(), spoken: vec!["hob".into()], phonetic: true }]),
            Config::default(),
        )
    }

    #[test]
    fn strips_fillers_and_fixes_spacing() {
        let (d, c) = env();
        assert_eq!(clean("um, the hob is, uh, late.", &d, &c), "The HAWB is late.");
    }

    #[test]
    fn filler_at_sentence_start_recapitalises() {
        let (d, c) = env();
        assert_eq!(clean("Um, send it now.", &d, &c), "Send it now.");
    }

    #[test]
    fn spoken_commands() {
        let (d, c) = env();
        assert_eq!(clean("first line new line second line new paragraph third", &d, &c), "First line\nSecond line\n\nThird");
    }

    #[test]
    fn spoken_commands_consecutive() {
        let (d, c) = env();
        assert_eq!(clean("a new line new line b", &d, &c), "A\n\nB");
    }

    #[test]
    fn spoken_commands_off() {
        let (d, mut c) = env();
        c.spoken_commands = false;
        assert_eq!(clean("a new line b", &d, &c), "A new line b");
    }

    #[test]
    fn join_parts_repairs_segment_boundaries() {
        assert_eq!(join_parts(&["Hello there.".into(), "how are you".into()]), "Hello there. How are you");
        assert_eq!(join_parts(&["one".into(), "".into(), "two".into()]), "one two");
    }

    #[test]
    fn trims() {
        let (d, c) = env();
        assert_eq!(clean("  hi  ", &d, &c), "Hi");
    }
}
