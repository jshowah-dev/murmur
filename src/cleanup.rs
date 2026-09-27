use crate::config::Config;
use crate::dictionary::Dictionary;
use crate::snippets::Snippets;

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

/// Lowercased token with STT punctuation stripped, so "New" / "line." match command words.
fn bare(tok: &str) -> String {
    tok.trim_matches(|c: char| !c.is_alphanumeric()).to_ascii_lowercase()
}

fn apply_commands(text: &str) -> String {
    let tokens: Vec<&str> = text.split(' ').collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let cur = bare(tokens[i]);
        if cur == "new" && i + 1 < tokens.len() {
            let next = bare(tokens[i + 1]);
            let brk = match next.as_str() {
                "line" => Some("\n"),
                "paragraph" => Some("\n\n"),
                _ => None,
            };
            if let Some(brk) = brk {
                // the model punctuates the command ("Thanks, New Line, Jeff."):
                // a comma left on the word before a break was a sentence end
                if let Some(prev) = out.last_mut() {
                    if prev.ends_with(',') {
                        prev.pop();
                        prev.push('.');
                    }
                }
                out.push(brk.to_string());
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

pub fn clean(text: &str, dict: &Dictionary, snippets: &Snippets, cfg: &Config) -> String {
    let s = strip_fillers(text, &cfg.fillers);
    // before the dictionary so phonetic matching cannot rewrite trigger words
    let (mut s, expansions) = snippets.mark(&s);
    s = dict.apply(&s);
    if cfg.spoken_commands {
        s = apply_commands(&s);
    }
    // trim spaces only: a leading/trailing "new line" command is deliberate
    crate::snippets::restore(tidy(&s).trim_matches(' '), &expansions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::dictionary::{Dictionary, Term};
    use crate::snippets::{Snippet, Snippets};

    fn snips() -> Snippets {
        Snippets {
            snippets: vec![
                Snippet { trigger: "my signature".into(), text: "best,
  jeff  new line".into() },
                Snippet { trigger: "hob number".into(), text: "hawb-123".into() },
            ],
        }
    }

    #[test]
    fn multi_line_snippet_is_pasted_verbatim() {
        let (d, c) = env();
        assert_eq!(clean("Thanks. New paragraph. My signature.", &d, &snips(), &c), "Thanks.

best,
  jeff  new line");
    }

    #[test]
    fn snippet_trigger_is_not_rewritten_by_the_dictionary() {
        let (d, c) = env();
        assert_eq!(clean("um, the hob number is, uh, late.", &d, &snips(), &c), "The hawb-123 is late.");
        assert_eq!(clean("Hob number.", &d, &snips(), &c), "hawb-123.");
    }

    fn env() -> (Dictionary, Config) {
        (
            Dictionary::from_terms(vec![Term { written: "HAWB".into(), spoken: vec!["hob".into()], phonetic: true }]),
            Config::default(),
        )
    }

    #[test]
    fn strips_fillers_and_fixes_spacing() {
        let (d, c) = env();
        assert_eq!(clean("um, the hob is, uh, late.", &d, &Snippets::default(), &c), "The HAWB is late.");
    }

    #[test]
    fn filler_at_sentence_start_recapitalises() {
        let (d, c) = env();
        assert_eq!(clean("Um, send it now.", &d, &Snippets::default(), &c), "Send it now.");
    }

    #[test]
    fn spoken_commands() {
        let (d, c) = env();
        assert_eq!(clean("first line new line second line new paragraph third", &d, &Snippets::default(), &c), "First line\nSecond line\n\nThird");
    }

    #[test]
    fn spoken_commands_with_stt_punctuation() {
        let (d, c) = env();
        // Parakeet punctuates: command words arrive as "New line." / "new paragraph,"
        assert_eq!(clean("The HAWB is late. New line. Send it to Delgado.", &d, &Snippets::default(), &c), "The HAWB is late.\nSend it to Delgado.");
        assert_eq!(clean("first, new paragraph, second", &d, &Snippets::default(), &c), "First.\n\nSecond");
        assert_eq!(clean("I'm in a comment new line. Next", &d, &Snippets::default(), &c), "I'm in a comment\nNext");
    }

    #[test]
    fn spoken_commands_consecutive() {
        let (d, c) = env();
        assert_eq!(clean("a new line new line b", &d, &Snippets::default(), &c), "A\n\nB");
    }

    #[test]
    fn spoken_commands_off() {
        let (d, mut c) = env();
        c.spoken_commands = false;
        assert_eq!(clean("a new line b", &d, &Snippets::default(), &c), "A new line b");
    }

    #[test]
    fn trims() {
        let (d, c) = env();
        assert_eq!(clean("  hi  ", &d, &Snippets::default(), &c), "Hi");
    }

    #[test]
    fn leading_new_line_command_is_kept() {
        let (d, c) = env();
        assert_eq!(clean("New line Put it in Delgado.", &d, &Snippets::default(), &c), "\nPut it in Delgado.");
        assert_eq!(clean("done new line", &d, &Snippets::default(), &c), "Done\n");
    }

    #[test]
    fn comma_before_new_line_command_becomes_period() {
        let (d, c) = env();
        assert_eq!(clean("Thanks, New Line, Jeff.", &d, &Snippets::default(), &c), "Thanks.\nJeff.");
        assert_eq!(clean("Thanks. New line. Jeff.", &d, &Snippets::default(), &c), "Thanks.\nJeff.");
    }
}
