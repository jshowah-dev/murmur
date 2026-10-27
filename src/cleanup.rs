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

/// Said between two like items, these take back the first: "Tuesday, no, Wednesday".
const MARKERS: &[&[&str]] = &[&["no"], &["i", "mean"], &["actually"], &["sorry"]];
const DAYS: &[&str] = &["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday", "today", "tomorrow", "tonight", "yesterday"];
const MONTHS: &[&str] = &["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];
const NUMBER_WORDS: &[&str] = &[
    "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve", "thirteen",
    "fourteen", "fifteen", "sixteen", "seventeen", "eighteen", "nineteen", "twenty", "thirty", "forty", "fifty", "sixty",
    "seventy", "eighty", "ninety", "hundred",
];

#[derive(PartialEq)]
enum Kind {
    Day,
    Month,
    /// Times too: "3, I mean 4pm".
    Number,
    Name,
}

/// What kind of item `tok` is, if any. A capitalised word is a name unless it opens the sentence
/// or is "I"; a month must be capitalised, so "may" stays a verb.
fn kind(tok: &str, opens_sentence: bool) -> Option<Kind> {
    let w = bare(tok);
    let capital = tok.trim_start_matches(|c: char| !c.is_alphanumeric()).starts_with(|c: char| c.is_uppercase());
    if DAYS.contains(&w.as_str()) {
        Some(Kind::Day)
    } else if capital && MONTHS.contains(&w.as_str()) {
        Some(Kind::Month)
    } else if w.starts_with(|c: char| c.is_ascii_digit()) || NUMBER_WORDS.contains(&w.as_str()) {
        Some(Kind::Number)
    } else if capital && !opens_sentence && w != "i" && !w.starts_with("i'") {
        Some(Kind::Name)
    } else {
        None
    }
}

/// How many tokens from `at` make up a marker, if one starts there. Only its last word may carry
/// a comma: "No." is an answer.
fn marker_len(tokens: &[String], at: usize) -> Option<usize> {
    MARKERS.iter().find_map(|m| {
        let said = tokens.get(at..at + m.len())?;
        let fits = said.iter().zip(m.iter()).enumerate().all(|(k, (tok, word))| {
            let tok = if k + 1 == m.len() { tok.strip_suffix(',').unwrap_or(tok) } else { tok.as_str() };
            tok.eq_ignore_ascii_case(word)
        });
        fits.then_some(m.len())
    })
}

/// "Tuesday, no, Wednesday" → "Wednesday": a marker between two items of the same kind keeps
/// only the second. The first must run on into the marker, so "Is it Tuesday? No, Wednesday."
/// stays an answer.
fn drop_self_corrections(text: &str) -> String {
    let mut tokens: Vec<String> = text.split(' ').filter(|t| !t.is_empty()).map(String::from).collect();
    let before = tokens.len();
    let mut i = 0;
    while i < tokens.len() {
        let x = tokens[i].strip_suffix(',').unwrap_or(&tokens[i]);
        let runs_on = x.ends_with(|c: char| c.is_alphanumeric() || c == '%');
        let opens = i == 0 || tokens[i - 1].ends_with(['.', '!', '?']);
        let y = marker_len(&tokens, i + 1).map(|n| i + 1 + n);
        match (runs_on, y) {
            (true, Some(y)) if y < tokens.len() && kind(&tokens[i], opens).is_some_and(|k| kind(&tokens[y], false) == Some(k)) => {
                // stay on i: "Tuesday, no, Wednesday, no, Thursday" goes again
                tokens.drain(i..y);
            }
            _ => i += 1,
        }
    }
    // rebuilt only on a change, so spacing elsewhere is left as it was
    if tokens.len() == before { text.to_string() } else { tokens.join(" ") }
}

/// Helper verbs that open a question when flipped before their subject: "is there", "can you".
/// "had" is left out: "Had I known, ..." opens a statement far more often than a question.
const HELPERS: &[&str] = &[
    "is", "are", "was", "were", "am", "do", "does", "did", "can", "could", "will", "would", "should", "shall", "may",
    "might", "must", "have", "has", "isn't", "aren't", "wasn't", "weren't", "don't", "doesn't", "didn't", "can't",
    "couldn't", "won't", "wouldn't", "shouldn't", "haven't", "hasn't", "hadn't", "mustn't",
];
/// Subjects of a flipped helper. "do" and "have" take only people, so "Do it now." and
/// "Have a look." stay commands.
const PEOPLE: &[&str] = &["i", "you", "we", "they", "he", "she", "anyone", "anybody", "someone", "somebody", "everyone"];
const THINGS: &[&str] = &[
    "it", "there", "this", "that", "these", "those", "the", "a", "an", "my", "your", "our", "their", "his", "her", "its",
    "any", "anything", "something", "everything", "all",
];
const WH: &[&str] = &["what", "why", "how", "where", "when", "who"];
const WH_IS: &[&str] = &["what's", "why's", "how's", "where's", "when's", "who's"];
/// Words that lead into a question without changing it: "So, can you check it."
const LEADS: &[&str] = &["and", "but", "so", "okay", "ok", "also", "well", "hey", "then", "now"];

fn flipped(helper: &str, subject: &str) -> bool {
    HELPERS.contains(&helper)
        && (PEOPLE.contains(&subject) || (!matches!(helper, "do" | "have") && THINGS.contains(&subject)))
}

/// A sentence shaped like a question: a flipped helper or wh-word at the start ("Do we ship
/// today", "What do you think"), or a tag at the end ("..., right", "..., isn't it").
fn is_question(sentence: &str) -> bool {
    let words = |s: &str| s.split_whitespace().map(|t| bare(&t.replace('\u{2019}', "'"))).collect::<Vec<_>>();
    let all = words(sentence);
    let w: Vec<&str> = all.iter().map(String::as_str).skip_while(|w| LEADS.contains(w)).collect();
    let opens = match w.as_slice() {
        [first, ..] if WH_IS.contains(first) => true,
        [first, second, ..] if WH.contains(first) => HELPERS.contains(second),
        [first, second, ..] => flipped(first, second),
        _ => false,
    };
    let tagged = sentence.rsplit_once(',').is_some_and(|(_, tail)| match words(tail).as_slice() {
        [one] => one == "right" || one == "correct",
        [helper, subject] => flipped(helper, subject),
        _ => false,
    });
    opens || tagged
}

/// A question-shaped sentence ending in "." ends in "?" instead. A "." inside a word ("3.5",
/// "showah.co") is not a sentence end.
fn mark_questions(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((i, ch)) = chars.next() {
        let at_end = chars.peek().is_none_or(|(_, c)| c.is_whitespace());
        if ch == '\n' || (matches!(ch, '.' | '!' | '?') && at_end) {
            let sentence = &text[start..i];
            out.push_str(sentence);
            out.push(if ch == '.' && is_question(sentence) { '?' } else { ch });
            start = i + ch.len_utf8();
        }
    }
    out.push_str(&text[start..]);
    out
}

/// Lowercased token with STT punctuation stripped, so "New" / "line." match command words.
fn bare(tok: &str) -> String {
    tok.trim_matches(|c: char| !c.is_alphanumeric()).to_ascii_lowercase()
}

fn is_scratch_words(words: &[String]) -> bool {
    matches!(words, [verb, that] if (verb == "scratch" || verb == "delete") && that == "that")
}

/// The whole dictation is "scratch that" or "delete that": undo the one before it.
pub fn is_scratch(raw: &str, cfg: &Config) -> bool {
    let words: Vec<String> = strip_fillers(raw, &cfg.fillers).split_whitespace().map(bare).collect();
    cfg.spoken_commands && is_scratch_words(&words)
}

/// The dictation ends in "Scratch that.", so nothing of it is left to paste.
pub fn scratches_itself(raw: &str, cfg: &Config) -> bool {
    let s = strip_fillers(raw, &cfg.fillers);
    cfg.spoken_commands && !s.trim().is_empty() && drop_scratched(&s).is_empty()
}

/// The dictation opens with "Scratch that." and goes on, so the new text replaces the one before it.
pub fn opens_with_scratch(raw: &str, cfg: &Config) -> bool {
    let s = strip_fillers(raw, &cfg.fillers);
    let tokens: Vec<&str> = s.split_whitespace().collect();
    let Some(end) = tokens.iter().position(|t| t.ends_with(['.', '!', '?'])) else { return false };
    let words: Vec<String> = tokens[..=end].iter().map(|t| bare(t)).collect();
    cfg.spoken_commands && is_scratch_words(&words) && end + 1 < tokens.len()
}

/// What follows the last "Scratch that." said as its own sentence; the text before it is dropped.
fn drop_scratched(text: &str) -> String {
    let tokens: Vec<&str> = text.split(' ').filter(|t| !t.is_empty()).collect();
    let mut keep_from = 0;
    let mut start = 0;
    for (i, tok) in tokens.iter().enumerate() {
        if tok.ends_with(['.', '!', '?']) {
            let words: Vec<String> = tokens[start..=i].iter().map(|t| bare(t)).collect();
            if is_scratch_words(&words) {
                keep_from = i + 1;
            }
            start = i + 1;
        }
    }
    tokens[keep_from..].join(" ")
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
    let mut prev = ' ';
    for ch in s.chars() {
        // a dot inside a word is not a sentence end: "showah.co", not "showah.Co"
        if cap && matches!(prev, '.' | '!' | '?') && !ch.is_whitespace() {
            cap = false;
        }
        // the model runs a currency sign onto the word before it: "costs$125"
        if matches!(ch, '$' | '€' | '£') && prev.is_alphanumeric() {
            out.push(' ');
        }
        if cap && ch.is_alphabetic() {
            out.extend(ch.to_uppercase());
            cap = false;
        } else {
            out.push(ch);
        }
        if matches!(ch, '.' | '!' | '?' | '\n') {
            cap = true;
        } else if !ch.is_whitespace() && !matches!(ch, '(' | '[' | '{' | '"' | '\u{201C}') {
            cap = false;
        }
        prev = ch;
    }
    out
}

/// Words a path or address does not run on from: "the path is slash users" → "is /users".
const NO_GLUE: &[&str] = &[
    "is", "are", "was", "it's", "at", "in", "to", "from", "into", "under", "on", "of", "and", "or", "called", "named", "type",
];

/// Spoken symbols: "showah dot co" → "showah.co", "open paren that close paren" → "(that)".
/// A symbol word after "a", "an" or "the" stays a word ("a dash of"). "literal dot" keeps the
/// word; "symbol dash" always gives the symbol. Email only takes "@" and a dot between two words.
fn apply_symbols(text: &str, symbols: &std::collections::BTreeMap<String, String>, profile: Profile) -> String {
    // a blank key in config.toml would match nowhere and everywhere
    let mut phrases: Vec<(Vec<&str>, &str)> = symbols
        .iter()
        .map(|(said, sym)| (said.split_whitespace().collect::<Vec<_>>(), sym.as_str()))
        .filter(|(words, _)| !words.is_empty())
        .collect();
    phrases.sort_by_key(|(words, _)| std::cmp::Reverse(words.len()));
    let tokens: Vec<&str> = text.split(' ').collect();
    // the phrase at `i`, with its length; only its last word may carry punctuation
    let phrase_at = |i: usize| {
        phrases.iter().find_map(|(words, sym)| {
            let toks = tokens.get(i..i + words.len())?;
            let fits = words.iter().zip(toks).enumerate().all(|(k, (w, t))| {
                bare(t).eq_ignore_ascii_case(w) && (k + 1 == words.len() || t.chars().all(char::is_alphanumeric))
            });
            fits.then_some((words.len(), *sym))
        })
    };
    // each piece with whether it joins the piece before and the piece after it
    let mut pieces: Vec<(String, bool, bool)> = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let lead = bare(tokens[i]);
        let forced = lead == "symbol" && phrase_at(i + 1).is_some();
        if lead == "literal" {
            if let Some((n, _)) = phrase_at(i + 1) {
                pieces.extend(tokens[i + 1..=i + n].iter().map(|t| (t.to_string(), false, false)));
                i += 1 + n;
                continue;
            }
        }
        let at = if forced { i + 1 } else { i };
        let Some((n, sym)) = phrase_at(at) else {
            pieces.push((tokens[i].to_string(), false, false));
            i += 1;
            continue;
        };
        let last = tokens[at + n - 1];
        let trail = &last[last.trim_end_matches(|c: char| !c.is_alphanumeric()).len()..];
        let prev = pieces.last().map(|(p, _, _)| p.as_str());
        let next = tokens.get(at + n).copied();
        // a comma before or after is a pause, so nothing joins across it
        let word_before = prev.is_some_and(|p| p.ends_with(|c: char| c.is_alphanumeric()));
        let left = word_before && !prev.is_some_and(|p| NO_GLUE.contains(&bare(p).as_str()));
        let right = trail.is_empty() && next.is_some();
        let (left, right) = match sym {
            "(" | "[" | "{" => (false, right),
            ")" | "]" | "}" => (word_before, false),
            "%" => (prev.is_some_and(|p| p.ends_with(|c: char| c.is_ascii_digit())), false),
            ":" => (word_before, right && next.is_some_and(|t| t.starts_with(|c: char| c.is_ascii_digit()))),
            _ => (left, right),
        };
        let article = prev.is_some_and(|p| matches!(bare(p).as_str(), "a" | "an" | "the"));
        let allowed = match profile {
            _ if forced => true,
            _ if article => false,
            Profile::Email => sym == "@" || (sym == "." && left && right),
            // a joiner needs something after it with no pause: "the hundred yard dash." stays
            Profile::Plain => match sym {
                "(" | "[" | "{" | ")" | "]" | "}" => true,
                "%" | ":" => left,
                _ => right,
            },
        };
        if allowed {
            pieces.push((format!("{sym}{trail}"), left, right));
        } else {
            pieces.extend(tokens[at..at + n].iter().map(|t| (t.to_string(), false, false)));
        }
        i = at + n;
    }
    let mut out = String::with_capacity(text.len());
    for (k, (piece, left, _)) in pieces.iter().enumerate() {
        if k > 0 && !left && !pieces[k - 1].2 {
            out.push(' ');
        }
        out.push_str(piece);
    }
    out
}

/// The model writes numbers as digits but a spoken decimal as a word: "5 point 2" → "5.2".
/// Only between two numbers, so "the point 2 of us made" keeps its word.
fn join_decimals(text: &str) -> String {
    let number = |t: &str| t.starts_with(|c: char| c.is_ascii_digit()) && t.chars().all(|c| c.is_ascii_digit() || c == '.');
    let tokens: Vec<&str> = text.split(' ').collect();
    let mut out: Vec<String> = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        let joins = out.last().is_some_and(|p| number(p))
            && tokens[i].eq_ignore_ascii_case("point")
            && tokens.get(i + 1).is_some_and(|t| t.starts_with(|c: char| c.is_ascii_digit()));
        if joins {
            let last = out.last_mut().expect("checked above");
            last.push('.');
            last.push_str(tokens[i + 1]);
            i += 2;
        } else {
            out.push(tokens[i].to_string());
            i += 1;
        }
    }
    out.join(" ")
}

/// Spoken punctuation. No "period": it is too common a word ("the trial period ends").
const PUNCTUATION: &[(&[&str], &str)] = &[
    (&["comma"], ","),
    (&["semicolon"], ";"),
    (&["full", "stop"], "."),
    (&["question", "mark"], "?"),
    (&["exclamation", "point"], "!"),
    (&["exclamation", "mark"], "!"),
];

/// "Hi comma Sarah" → "Hi, Sarah". The model often punctuates the word as well ("Hi, comma,
/// Sarah"), so the mark replaces whatever ends the word before and drops what ends its own.
/// After "a", "an" or "the" it stays a word ("add a comma"), as it does after "literal".
fn apply_punctuation(text: &str) -> String {
    let tokens: Vec<&str> = text.split(' ').collect();
    // the phrase at `i`, with its length; only its last word may carry punctuation
    let phrase_at = |i: usize| {
        PUNCTUATION.iter().find_map(|(words, mark)| {
            let toks = tokens.get(i..i + words.len())?;
            let fits = words
                .iter()
                .zip(toks)
                .enumerate()
                .all(|(k, (w, t))| bare(t) == *w && (k + 1 == words.len() || t.chars().all(char::is_alphanumeric)));
            fits.then_some((words.len(), *mark))
        })
    };
    let mut out: Vec<String> = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        if bare(tokens[i]) == "literal" {
            if let Some((n, _)) = phrase_at(i + 1) {
                out.extend(tokens[i + 1..=i + n].iter().map(|t| t.to_string()));
                i += 1 + n;
                continue;
            }
        }
        let prev = out.last_mut().filter(|p| !matches!(bare(p).as_str(), "" | "a" | "an" | "the"));
        if let (Some(prev), Some((n, mark))) = (prev, phrase_at(i)) {
            prev.truncate(prev.trim_end_matches([',', '.', ';', ':', '!', '?']).len());
            prev.push_str(mark);
            i += n;
            continue;
        }
        out.push(tokens[i].to_string());
        i += 1;
    }
    out.join(" ")
}

pub fn clean(text: &str, dict: &Dictionary, snippets: &Snippets, cfg: &Config, profile: Profile) -> String {
    let mut s = strip_fillers(text, &cfg.fillers);
    if cfg.spoken_commands {
        s = drop_scratched(&s);
    }
    // after the fillers, so "the, um, the" is seen as a repeat
    if cfg.drop_stutters {
        s = drop_stutters(&s, dict);
    }
    if cfg.self_corrections {
        s = drop_self_corrections(&s);
    }
    // before the dictionary so phonetic matching cannot rewrite trigger words
    let (mut s, expansions) = snippets.mark(&s);
    if cfg.spoken_commands {
        s = apply_punctuation(&s);
    }
    // before the dictionary too, so a sound-alike term cannot take "dot" or "dash"
    s = apply_symbols(&join_decimals(&s), &cfg.symbols, profile);
    s = dict.apply(&s);
    if cfg.spoken_commands {
        s = apply_commands(&s);
    }
    let s = mark_questions(&tidy(&s));
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
    fn question_shaped_sentences_get_a_question_mark() {
        let (d, c) = bare_env();
        for (said, want) in [
            ("Is there a test for this.", "Is there a test for this?"),
            ("Can you check the build.", "Can you check the build?"),
            ("Do we ship today.", "Do we ship today?"),
            ("Should I merge it.", "Should I merge it?"),
            ("Would it be faster.", "Would it be faster?"),
            ("Isn't it done.", "Isn't it done?"),
            ("What do you think.", "What do you think?"),
            ("What\u{2019}s the status.", "What\u{2019}s the status?"),
            ("So, can you look at it.", "So, can you look at it?"),
            ("It's ready, right.", "It's ready, right?"),
            ("The build passed, didn't it.", "The build passed, didn't it?"),
            ("It's done. Can you test it. Thanks.", "It's done. Can you test it? Thanks."),
            ("Is it version 3.5 now.", "Is it version 3.5 now?"),
        ] {
            assert_eq!(clean(said, &d, &Snippets::default(), &c), want);
        }
    }

    #[test]
    fn statements_keep_their_period() {
        let (d, c) = bare_env();
        for keep in [
            "Do it now.",
            "Have a look at this.",
            "Do the dishes.",
            "It is there.",
            "What I mean is it works.",
            "Turn right.",
            "We can, if you want.",
            "Can you believe it!",
        ] {
            assert_eq!(clean(keep, &d, &Snippets::default(), &c), keep);
        }
    }

    #[test]
    fn spoken_symbols_become_symbols() {
        let (d, c) = bare_env();
        for (said, want) in [
            ("Send it to jeff at sign showah dot co.", "Send it to jeff@showah.co."),
            ("The path is slash users slash jeff slash code.", "The path is /users/jeff/code."),
            ("Open paren that one close paren.", "(That one)."),
            ("Name it my underscore file dash two.", "Name it my_file-two."),
            ("Note colon ship it.", "Note: ship it."),
            ("Port localhost colon 8080.", "Port localhost:8080."),
            ("It grew 20 percent.", "It grew 20%."),
            ("Type symbol dash v.", "Type -v."),
            ("Say the literal dot com.", "Say the dot com."),
        ] {
            assert_eq!(clean(said, &d, &Snippets::default(), &c), want);
        }
    }

    #[test]
    fn symbol_words_kept_as_words() {
        let (d, c) = bare_env();
        for keep in [
            "Use an underscore between the words, and a dash in the name.",
            "Add a dash of salt.",
            "He won the hundred yard dash.",
            "A high percent of them.",
            "Put a dot here.",
        ] {
            assert_eq!(clean(keep, &d, &Snippets::default(), &c), keep);
        }
    }

    #[test]
    fn scratch_that_drops_what_came_before() {
        let (d, c) = bare_env();
        assert_eq!(clean("Send it Monday. Scratch that. Send it Friday.", &d, &Snippets::default(), &c), "Send it Friday.");
        assert_eq!(clean("One. Delete that! Two. Scratch that. Three.", &d, &Snippets::default(), &c), "Three.");
        assert_eq!(clean("Send it Monday. Um, scratch that. Friday.", &d, &Snippets::default(), &c), "Friday.");
        // not its own sentence, so it is just words
        assert_eq!(clean("I had to scratch that idea.", &d, &Snippets::default(), &c), "I had to scratch that idea.");
        let mut off = c.clone();
        off.spoken_commands = false;
        assert_eq!(clean("A. Scratch that. B.", &d, &Snippets::default(), &off), "A. Scratch that. B.");
    }

    #[test]
    fn a_marker_between_like_items_keeps_the_second() {
        let (d, c) = bare_env();
        let cl = |s: &str| clean(s, &d, &Snippets::default(), &c);
        assert_eq!(cl("Let's meet Tuesday, no, Wednesday."), "Let's meet Wednesday.");
        assert_eq!(cl("Call me at 3, I mean 4."), "Call me at 4.");
        assert_eq!(cl("It's due in March, actually, April."), "It's due in April.");
        assert_eq!(cl("Send it to Jim, sorry, Jen."), "Send it to Jen.");
        assert_eq!(cl("Book two, no, three rooms."), "Book three rooms.");
        assert_eq!(cl("At 3pm, no, 4:30pm."), "At 4:30pm.");
        assert_eq!(cl("Monday, no, Tuesday, no, Friday works."), "Friday works.");
        // an answer, not a correction
        assert_eq!(cl("Is it Tuesday? No, Wednesday."), "Is it Tuesday? No, Wednesday.");
        // items of different kinds
        assert_eq!(cl("Thanks Mike, sorry, Monday doesn't work."), "Thanks Mike, sorry, Monday doesn't work.");
        assert_eq!(cl("I saw Jim, sorry, I was late."), "I saw Jim, sorry, I was late.");
        assert_eq!(cl("I have 2 kids, no, 3 dogs."), "I have 2 kids, no, 3 dogs.");
        // a capitalised word that opens the sentence isn't a name
        assert_eq!(cl("Okay, sorry, Jim is here."), "Okay, sorry, Jim is here.");
        let mut off = c.clone();
        off.self_corrections = false;
        assert_eq!(clean("Tuesday, no, Wednesday.", &d, &Snippets::default(), &off), "Tuesday, no, Wednesday.");
    }

    #[test]
    fn a_scratch_on_its_own_is_a_command() {
        let (_, c) = bare_env();
        assert!(super::is_scratch("Scratch that.", &c));
        assert!(super::is_scratch("Um, delete that", &c));
        assert!(!super::is_scratch("Scratch that idea.", &c));
        assert!(!super::is_scratch("A. Scratch that.", &c));
        assert!(super::scratches_itself("A. Scratch that.", &c));
        assert!(!super::scratches_itself("Um.", &c));
        assert!(!super::scratches_itself("Scratch that. B.", &c));
        assert!(super::opens_with_scratch("Um, scratch that. B.", &c));
        assert!(!super::opens_with_scratch("Scratch that.", &c));
        assert!(!super::opens_with_scratch("A. Scratch that. B.", &c));
        assert!(!super::opens_with_scratch("Scratch that idea. B.", &c));
        let mut off = c.clone();
        off.spoken_commands = false;
        assert!(!super::is_scratch("Scratch that.", &off));
        assert!(!super::opens_with_scratch("Scratch that. B.", &off));
    }

    #[test]
    fn a_blank_symbol_key_is_ignored() {
        let (d, mut c) = bare_env();
        c.symbols.insert(" ".into(), "x".into());
        assert_eq!(clean("Go dot com.", &d, &Snippets::default(), &c), "Go.com.");
    }

    #[test]
    fn email_takes_only_addresses() {
        let (d, c) = bare_env();
        let got = super::clean("Mail jeff at sign showah dot co slash later.", &d, &Snippets::default(), &c, Profile::Email);
        assert_eq!(got, "Mail jeff@showah.co slash later.");
    }

    #[test]
    fn spoken_decimal_points_become_dots() {
        let (d, c) = bare_env();
        for (said, want) in [
            ("Update the 5 Point 2 release.", "Update the 5.2 release."),
            ("Ship 0 point 5 point 3 today.", "Ship 0.5.3 today."),
            ("It weighs 2 point 75kg.", "It weighs 2.75kg."),
            ("That's 3 point.", "That's 3 point."),
            ("Make a point 2 times.", "Make a point 2 times."),
        ] {
            assert_eq!(clean(said, &d, &Snippets::default(), &c), want, "{said}");
        }
    }

    #[test]
    fn spoken_punctuation_becomes_marks() {
        let (d, c) = bare_env();
        for (said, want) in [
            ("Hi comma Sarah.", "Hi, Sarah."),
            ("Hi, comma, Sarah.", "Hi, Sarah."),
            ("Done full stop next one.", "Done. Next one."),
            ("Is it ready question mark", "Is it ready?"),
            ("We shipped exclamation point.", "We shipped!"),
            ("First semicolon second.", "First; second."),
            ("Add a comma here.", "Add a comma here."),
            ("Type literal comma here.", "Type comma here."),
            ("The trial period ends.", "The trial period ends."),
            ("Comma is a word.", "Comma is a word."),
        ] {
            assert_eq!(clean(said, &d, &Snippets::default(), &c), want, "{said}");
        }
    }

    #[test]
    fn spoken_punctuation_off() {
        let (d, mut c) = bare_env();
        c.spoken_commands = false;
        assert_eq!(clean("Hi comma Sarah.", &d, &Snippets::default(), &c), "Hi comma Sarah.");
    }

    #[test]
    fn dots_inside_words_and_currency() {
        let (d, c) = bare_env();
        assert_eq!(clean("Open showah.co now.", &d, &Snippets::default(), &c), "Open showah.co now.");
        assert_eq!(clean("It costs$125.", &d, &Snippets::default(), &c), "It costs $125.");
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
