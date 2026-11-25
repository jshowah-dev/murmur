//! Words the speech model mixes up that no dictionary swap can settle: "Claude" and "cloud"
//! are both real, so the words around one decide which was meant. With no clue either way the
//! model's word stays. A clue word must point one way only: "cloud model" and "Claude model",
//! "Claude Console" and "cloud console" are all said, so none of those words is a clue.

/// One pair: the two spellings, and the neighbouring words that point to each.
struct Pair {
    name: &'static str,
    word: &'static str,
    /// a word right after that means the name: "Claude Code"
    name_next: &'static [&'static str],
    /// a word right before that means the name: "ask Claude"
    name_prev: &'static [&'static str],
    /// a word right after that means the word: "cloud storage"
    word_next: &'static [&'static str],
    /// a word right before that means the word: "private cloud"
    word_prev: &'static [&'static str],
    /// a word two before, with "the" between, that means the word: "in the cloud". "The" alone
    /// is no clue: "the Claude session" is as likely as "the cloud session".
    word_prev_the: &'static [&'static str],
}

const PAIRS: &[Pair] = &[Pair {
    name: "Claude",
    word: "cloud",
    name_next: &[
        "code", "opus", "sonnet", "haiku", "fable", "app", "apps", "website", "said", "says", "wrote", "writes", "thinks", "thought",
        "suggested", "suggests", "knows", "answered", "replied", "found", "missed",
    ],
    name_prev: &["ask", "asked", "asking", "tell", "told", "telling", "thanks", "hey", "hi", "anthropic", "anthropic's"],
    word_next: &[
        "storage", "computing", "infrastructure", "environment", "environments", "function", "functions", "hosting", "native", "sync",
        "backup", "backups", "vm", "vms", "region", "regions",
    ],
    word_prev: &["private", "public", "hybrid"],
    word_prev_the: &["in", "into", "to", "from", "on", "onto", "off"],
}];

const CURLY: char = '\u{2019}';

/// A token's word, lowercased, with curly apostrophes made straight and its punctuation off.
fn core(tok: &str) -> String {
    tok.replace(CURLY, "'").trim_matches(|c: char| !c.is_alphanumeric() && c != '\'').to_lowercase()
}

/// True when the token ends a clause, closing quotes and brackets aside: `said."` and `so).`
fn ends_clause(tok: &str) -> bool {
    tok.trim_end_matches(['"', '\'', '\u{201D}', CURLY, ')', ']']).ends_with(['.', ',', '!', '?', ';', ':'])
}

/// True when `a` and `b` are the two sides of one pair, in any case: an edit between them is
/// a context call, never a swap to learn.
pub fn paired(a: &str, b: &str) -> bool {
    PAIRS.iter().any(|p| (a.eq_ignore_ascii_case(p.name) && b.eq_ignore_ascii_case(p.word)) || (a.eq_ignore_ascii_case(p.word) && b.eq_ignore_ascii_case(p.name)))
}

/// `text` with each pair word spelled the way its neighbours say it was meant.
pub fn apply(text: &str) -> String {
    let tokens: Vec<&str> = text.split(' ').collect();
    (0..tokens.len()).map(|i| decide(&tokens, i).unwrap_or_else(|| tokens[i].to_string())).collect::<Vec<_>>().join(" ")
}

/// The rewritten token at `i`, or None to leave it.
fn decide(tokens: &[&str], i: usize) -> Option<String> {
    let tok = tokens[i];
    let start = tok.find(char::is_alphanumeric)?;
    let end = tok[start..].find(|c: char| !c.is_alphanumeric()).map_or(tok.len(), |n| start + n);
    let (lead, body, rest) = (&tok[..start], &tok[start..end], &tok[end..]);
    let pair = PAIRS.iter().find(|p| body.eq_ignore_ascii_case(p.name) || body.eq_ignore_ascii_case(p.word))?;
    // "cloud's" keeps its ending; anything else after it is punctuation
    let after = rest.strip_prefix(['\'', CURLY]).and_then(|r| r.strip_prefix(['s', 'S'])).unwrap_or(rest);
    if after.starts_with(char::is_alphanumeric) {
        return None;
    }
    // neighbours skip the empty tokens double spaces leave, and stop at the clause's edge:
    // "the cloud. Claude said" splits at the period
    let next = if after.is_empty() { tokens[i + 1..].iter().find(|t| !t.is_empty()).map(|t| core(t)) } else { None };
    let before: Vec<&str> = tokens[..i].iter().rev().filter(|t| !t.is_empty()).take(2).copied().collect();
    let prev = before.first().filter(|t| !ends_clause(t)).map(|t| core(t));
    let prev2 = if prev.as_deref() == Some("the") { before.get(1).filter(|t| !ends_clause(t)).map(|t| core(t)) } else { None };
    let is = |w: &Option<String>, list: &[&str]| w.as_deref().is_some_and(|w| list.contains(&w));
    // the word after is the stronger clue: "in the Claude app" has "in the" before it but is the name
    let name = if is(&next, pair.name_next) {
        true
    } else if is(&next, pair.word_next) {
        false
    } else if is(&prev, pair.name_prev) {
        true
    } else if is(&prev, pair.word_prev) || is(&prev2, pair.word_prev_the) {
        false
    } else {
        return None;
    };
    let spelled = if name { pair.name } else { pair.word };
    // already that word: its case stays, so "Google Cloud" is left as written
    if body.eq_ignore_ascii_case(spelled) {
        return None;
    }
    // a sentence start is capitalised later, by tidy()
    Some(format!("{lead}{spelled}{rest}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_and_agent_clues_mean_claude() {
        assert_eq!(apply("open cloud code and run it"), "open Claude code and run it");
        assert_eq!(apply("ask cloud to fix it"), "ask Claude to fix it");
        assert_eq!(apply("cloud said the tests pass"), "Claude said the tests pass");
        assert_eq!(apply("switch to cloud opus"), "switch to Claude opus");
        assert_eq!(apply("log in to the cloud app"), "log in to the Claude app");
    }

    #[test]
    fn infrastructure_clues_mean_cloud() {
        assert_eq!(apply("it runs in the Claude"), "it runs in the cloud");
        assert_eq!(apply("move it to Claude storage"), "move it to cloud storage");
        assert_eq!(apply("a private Claude, mostly"), "a private cloud, mostly");
        assert_eq!(apply("pushed it to the Claude yesterday"), "pushed it to the cloud yesterday");
    }

    #[test]
    fn the_word_after_beats_the_word_before() {
        assert_eq!(apply("in the cloud app"), "in the Claude app");
        assert_eq!(apply("ask Claude storage"), "ask cloud storage");
    }

    #[test]
    fn words_said_of_both_are_no_clue() {
        for s in [
            "start a Claude session", "open the cloud session", "a cloud model or a local model", "the Claude Console", "the cloud console",
            "Claude costs too much", "run Claude instances", "the cloud API is down", "AWS Claude via Bedrock", "I like cloud",
        ] {
            assert_eq!(apply(s), s);
        }
    }

    #[test]
    fn a_possessive_is_matched_but_is_no_clue_itself() {
        assert_eq!(apply("the cloud's down again"), "the cloud's down again");
        assert_eq!(apply("ask cloud's opinion"), "ask Claude's opinion");
        assert_eq!(apply("ask cloud\u{2019}s opinion"), "ask Claude\u{2019}s opinion");
        assert_eq!(apply("in the Claude's shadow"), "in the cloud's shadow");
    }

    #[test]
    fn the_right_word_keeps_its_case() {
        assert_eq!(apply("Google Cloud storage"), "Google Cloud storage");
        assert_eq!(apply("use a private Cloud"), "use a private Cloud");
    }

    #[test]
    fn punctuation_ends_the_clue() {
        assert_eq!(apply("in the cloud. Claude said so"), "in the cloud. Claude said so");
        assert_eq!(apply("it was Claude, code reviewed"), "it was Claude, code reviewed");
        assert_eq!(apply("he said \"in the.\" Claude storage"), "he said \"in the.\" cloud storage");
        assert_eq!(apply("(ask.) cloud is fine"), "(ask.) cloud is fine");
    }

    #[test]
    fn keeps_punctuation_around_the_word() {
        assert_eq!(apply("ask (cloud)"), "ask (Claude)");
        assert_eq!(apply("ask cloud."), "ask Claude.");
        assert_eq!(apply("ask 'cloud'"), "ask 'Claude'");
    }

    #[test]
    fn double_spaces_do_not_hide_a_clue() {
        assert_eq!(apply("ask  cloud"), "ask  Claude");
        assert_eq!(apply("cloud  code"), "Claude  code");
    }

    #[test]
    fn a_sentence_start_is_left_for_tidy_to_capitalise() {
        assert_eq!(apply("Claude storage is cheap"), "cloud storage is cheap");
    }

    #[test]
    fn other_words_are_left_alone() {
        assert_eq!(apply("cloudy with clouds over Claudette and Jean-Claude"), "cloudy with clouds over Claudette and Jean-Claude");
    }

    #[test]
    fn paired_is_either_way_round_in_any_case() {
        assert!(paired("Claude", "cloud"));
        assert!(paired("CLOUD", "claude"));
        assert!(!paired("cloud", "crowd"));
    }
}
