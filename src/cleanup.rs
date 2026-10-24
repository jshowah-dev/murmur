use crate::config::Config;
use crate::context::Profile;
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

/// Words that are said twice on purpose: "I know that that works", "what it is is", "no no".
const DOUBLED_ON_PURPOSE: &[&str] = &["that", "had", "is", "no", "bye", "very", "really"];
const DIGIT_WORDS: &[&str] = &["zero", "oh", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine"];
/// Short words missing from the common-word list that still start real words: "the app approval".
const JARGON: &[&str] = &["app", "repo", "prod", "auth", "env", "spec", "docs", "stat", "temp", "sync", "init", "ops"];

/// True when `prev` is a stutter of `next`: the same word again, or the start of `next` cut off.
/// A sentence end between them is a real repeat ("Done. Done."), and a cut-off that is itself a
/// real word is more likely meant ("pro professional", "app approval").
fn stutters(prev: &str, next: &str, dict: &Dictionary) -> bool {
    if prev.ends_with(['.', '!', '?']) {
        return false;
    }
    let (p, n) = (bare(prev), bare(next));
    if p.is_empty() || p.chars().any(|c| c.is_ascii_digit()) {
        return false;
    }
    if p == n {
        return !DOUBLED_ON_PURPOSE.contains(&p.as_str()) && !DIGIT_WORDS.contains(&p.as_str());
    }
    p.len() >= 2
        && n.len() > p.len() + 1
        && n.starts_with(&p)
        && !crate::phonetic::is_common(&p)
        && !JARGON.contains(&p.as_str())
        && !in_dictionary(&p, dict)
}

/// The word is a term's written form or one of its single-word spoken forms.
fn in_dictionary(word: &str, dict: &Dictionary) -> bool {
    dict.terms
        .iter()
        .any(|t| t.written.eq_ignore_ascii_case(word) || t.spoken.iter().any(|s| s.eq_ignore_ascii_case(word)))
}

/// Each stutter goes and the word after it stays, keeping that word's punctuation.
fn drop_stutters(text: &str, dict: &Dictionary) -> String {
    let mut out: Vec<&str> = Vec::new();
    for tok in text.split(' ') {
        if out.last().is_some_and(|prev| stutters(prev, tok, dict)) {
            out.pop();
        }
        out.push(tok);
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

pub fn clean(text: &str, dict: &Dictionary, snippets: &Snippets, cfg: &Config, profile: Profile) -> String {
    let mut s = strip_fillers(text, &cfg.fillers);
    // after the fillers, so "the, um, the" is seen as a repeat
    if cfg.drop_stutters {
        s = drop_stutters(&s, dict);
    }
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

/// `text` with a leading space when it would otherwise run on from `before`, the character
/// already in front of the insertion point: back-to-back dictations read "now.Where's" without it.
pub fn spaced_after(before: Option<char>, text: &str) -> String {
    let joins = |c: char| c.is_whitespace() || matches!(c, '(' | '[' | '{' | '"' | '\'' | '\u{201C}' | '\u{2018}' | '/' | '-');
    let attaches = |c: char| c.is_whitespace() || matches!(c, ',' | '.' | ';' | ':' | '!' | '?' | ')' | ']' | '}');
    match (before, text.chars().next()) {
        (Some(b), Some(t)) if !joins(b) && !attaches(t) => format!(" {text}"),
        _ => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use crate::config::Config;
    use crate::dictionary::{Dictionary, Term};
    use crate::snippets::{Snippet, Snippets};
    use crate::context::Profile;

    /// The existing tests run as `Plain`: its output must stay what it was before profiles.
    fn clean(text: &str, dict: &Dictionary, snippets: &Snippets, cfg: &Config) -> String {
        super::clean(text, dict, snippets, cfg, Profile::Plain)
    }

    /// No dictionary terms, so sound-alike matching cannot touch the words under test.
    fn bare_env() -> (Dictionary, Config) {
        (Dictionary::from_terms(vec![]), Config::default())
    }

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
    fn repeated_word_is_dropped() {
        let (d, c) = bare_env();
        assert_eq!(clean("I think the the build passed.", &d, &Snippets::default(), &c), "I think the build passed.");
        assert_eq!(clean("I, I think so.", &d, &Snippets::default(), &c), "I think so.");
        assert_eq!(clean("The the the build passed.", &d, &Snippets::default(), &c), "The build passed.");
        assert_eq!(clean("Send the, um, the file.", &d, &Snippets::default(), &c), "Send the file.");
    }

    #[test]
    fn cut_off_word_is_dropped() {
        let (d, c) = bare_env();
        assert_eq!(clean("That's not rel relevant here.", &d, &Snippets::default(), &c), "That's not relevant here.");
        assert_eq!(clean("Rel- relevant.", &d, &Snippets::default(), &c), "Relevant.");
    }

    #[test]
    fn real_word_pairs_are_kept() {
        let (d, c) = bare_env();
        for keep in [
            "I know that that works.",
            "What it is is broken.",
            "No no, keep it.",
            "Call five five five one two.",
            "She's a pro professional.",
            "It's done. Done.",
            "Put it in inside.",
            "The app approval flow.",
            "Check the repo report.",
            // a mis-start that is not a prefix stays as dictated
            "That's not rev relevant.",
        ] {
            assert_eq!(clean(keep, &d, &Snippets::default(), &c), keep);
        }
    }

    #[test]
    fn dictionary_words_are_not_cut_offs() {
        let d = Dictionary::from_terms(vec![Term { written: "Kube".into(), spoken: vec!["koob".into()], phonetic: false }]);
        let c = Config::default();
        assert_eq!(clean("Ask kube kubernetes.", &d, &Snippets::default(), &c), "Ask Kube kubernetes.");
    }

    #[test]
    fn stutters_kept_when_turned_off() {
        let (d, mut c) = bare_env();
        c.drop_stutters = false;
        assert_eq!(clean("the the rev relevant", &d, &Snippets::default(), &c), "The the rev relevant");
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

    #[test]
    fn email_profile_shapes_greeting_and_signoff() {
        let (d, c) = bare_env();
        let raw = "um, hi Sarah, thanks for the update. I'll review it tomorrow and send notes by Friday. Thanks, Jeff.";
        assert_eq!(
            super::clean(raw, &d, &Snippets::default(), &c, Profile::Email),
            "Hi Sarah,

Thanks for the update. I'll review it tomorrow and send notes by Friday.

Thanks,
Jeff"
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
            "Hi Sarah,

The build is ready.

Thanks,
Jeff"
        );
    }

    #[test]
    fn spaced_after_separates_back_to_back_dictations() {
        use super::spaced_after;
        assert_eq!(spaced_after(Some('.'), "Where's the handoff?"), " Where's the handoff?");
        assert_eq!(spaced_after(Some('w'), "And more."), " And more.");
        assert_eq!(spaced_after(None, "Start."), "Start.", "empty field or unknown");
        assert_eq!(spaced_after(Some(' '), "Next."), "Next.");
        assert_eq!(spaced_after(Some('\n'), "Next."), "Next.");
        assert_eq!(spaced_after(Some('('), "aside"), "aside");
        assert_eq!(spaced_after(Some('.'), "\nNext"), "\nNext", "a leading new line command");
        assert_eq!(spaced_after(Some('d'), ", then"), ", then");
        assert_eq!(spaced_after(Some('.'), ""), "");
    }
}
