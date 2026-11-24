use crate::config::Config;
use crate::context::Profile;
use crate::dictionary::Dictionary;
use crate::snippets::Snippets;

fn strip_fillers(text: &str, fillers: &[String]) -> String {
    let mut out: Vec<String> = Vec::new();
    for tok in text.split(' ') {
        let raw: String = tok.chars().filter(|c| c.is_alphanumeric()).collect();
        let core = raw.to_ascii_lowercase();
        // "the ER" and "5 mm" are words, not hesitations
        let acronym = raw.len() > 1 && raw.chars().all(|c| c.is_ascii_uppercase());
        let unit = core == "mm" && out.last().is_some_and(|p| p.ends_with(|c: char| c.is_ascii_digit()));
        if !acronym && !unit && fillers.iter().any(|f| f == &core) {
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
        // "on eleven eleven" is a date
        return !DOUBLED_ON_PURPOSE.contains(&p.as_str()) && !DIGIT_WORDS.contains(&p.as_str()) && !NUMBER_WORDS.contains(&p.as_str());
    }
    if inflects(&p, &n) {
        return true;
    }
    p.len() >= 2
        && n.len() > p.len() + 1
        && n.starts_with(&p)
        && !crate::phonetic::is_common(&p)
        && !JARGON.contains(&p.as_str())
        && !in_dictionary(&p, dict)
}

/// True when `next` is `prev` with an ending added: "guess guessing", "make making", "run running".
/// Short words are left out, since "give it its due" and "give her hers" are said on purpose.
fn inflects(prev: &str, next: &str) -> bool {
    const ENDINGS: &[&str] = &["s", "es", "ed", "ing"];
    if prev.len() < 3 || prev == "her" {
        return false;
    }
    let Some(rest) = next.strip_prefix(prev) else {
        // "make making": the silent e goes before -ing
        return prev.strip_suffix('e').is_some_and(|stem| next.strip_prefix(stem) == Some("ing"));
    };
    if ENDINGS.contains(&rest) {
        return true;
    }
    // "run running", "stop stopped": the last letter doubles before -ing or -ed
    let last = prev.chars().next_back().expect("three or more bytes");
    rest.strip_prefix(last).is_some_and(|e| e == "ing" || e == "ed")
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
    // ends in a digit, so a sentence-ending "5." doesn't take the next sentence's "Point 2"
    let number = |t: &str| t.ends_with(|c: char| c.is_ascii_digit()) && t.chars().all(|c| c.is_ascii_digit() || c == '.');
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

const ONES: &[&str] = &[
    "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve", "thirteen", "fourteen",
    "fifteen", "sixteen", "seventeen", "eighteen", "nineteen",
];
const ORDINALS: &[&str] = &[
    "first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth", "ninth", "tenth", "eleventh", "twelfth",
    "thirteenth", "fourteenth", "fifteenth", "sixteenth", "seventeenth", "eighteenth", "nineteenth",
];
const TENS: &[&str] = &["twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety"];

/// 0 to 99 said as digits or words, and whether it was an ordinal: "29", "eleven", "twenty-nine", "22nd", "twenty-second".
fn number(w: &str) -> Option<(u32, bool)> {
    let unit = |w: &str| {
        let pos = |list: &[&str]| list.iter().position(|o| *o == w).map(|i| i as u32 + 1);
        pos(ONES).map(|n| (n, false)).or_else(|| pos(ORDINALS).map(|n| (n, true)))
    };
    let tens = |w: &str| TENS.iter().position(|t| *t == w).map(|i| i as u32 * 10 + 20);
    let digits = w.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    if !digits.is_empty() && digits.len() <= 2 && digits.chars().all(|c| c.is_ascii_digit()) {
        let suffix = &w[digits.len()..];
        let ordinal = matches!(suffix, "st" | "nd" | "rd" | "th");
        return (suffix.is_empty() || ordinal).then(|| digits.parse().ok().map(|n| (n, ordinal))).flatten();
    }
    match w.split_once('-') {
        Some((t, o)) => unit(o).filter(|(n, _)| *n < 10).and_then(|(n, ord)| Some((tens(t)? + n, ord))),
        None => unit(w).or_else(|| tens(w).map(|n| (n, false))).or(match w {
            "twentieth" => Some((20, true)),
            "thirtieth" => Some((30, true)),
            _ => None,
        }),
    }
}

/// The number starting at token i, with how many tokens it takes: "twenty-two", "twenty two", "twenty 2".
fn number_at(tokens: &[&str], i: usize) -> Option<(u32, bool, usize)> {
    let t = tokens.get(i)?;
    let (n, ord) = number(&bare(t))?;
    let plain = t.chars().all(char::is_alphanumeric);
    let ones = tokens.get(i + 1).filter(|_| plain && !ord && n >= 20 && n % 10 == 0 && !t.contains(|c: char| c.is_ascii_digit()));
    match ones.and_then(|o| number(&bare(o))) {
        Some((o, ord)) if (1..10).contains(&o) => Some((n + o, ord, 2)),
        _ => Some((n, ord, 1)),
    }
}

/// A month and day or an hour and minute the speech model wrote as words come out as digits:
/// "on eleven twenty-nine" → "on 11/29", "at three thirty" → "at 3:30". The words around the pair
/// pick which: "on", "the … date", a year or an ordinal mean a date; "at", "by", "pm" or "o'clock"
/// mean a time. With no such word, each number is written on its own.
fn join_dates(text: &str) -> String {
    let tokens: Vec<&str> = text.split(' ').collect();
    let plain = |t: &str| !t.is_empty() && t.chars().all(char::is_alphanumeric);
    let mut out: Vec<String> = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        if let Some(done) = date_or_time(&tokens, i, &out).filter(|_| plain(tokens[i])) {
            out.push(done.0);
            i += done.1;
            continue;
        }
        out.push(tokens[i].to_string());
        i += 1;
    }
    out.join(" ")
}

/// The pair starting at token i written as digits, with how many tokens it took.
fn date_or_time(tokens: &[&str], i: usize, out: &[String]) -> Option<(String, usize)> {
    let (lead, lead_ord, 1) = number_at(tokens, i)? else { return None };
    if lead_ord || !(1..=12).contains(&lead) {
        return None;
    }
    let oh = tokens.get(i + 1).is_some_and(|t| t.eq_ignore_ascii_case("oh"));
    let (second, ord, n) = number_at(tokens, i + 1 + oh as usize)?;
    // "one second" is a moment, not January 2nd
    if ord && second < 10 {
        return None;
    }
    let n = n + oh as usize;
    // the end of what is taken so far, and any punctuation after it
    let ends = |taken: usize| {
        let last = tokens[i + taken];
        let trail = &last[last.trim_end_matches(|c: char| !c.is_alphanumeric()).len()..];
        (trail, tokens.get(i + 1 + taken).filter(|_| trail.is_empty()).map(|t| bare(t)))
    };
    let (trail, next) = ends(n);
    // "eleven twenty-two twenty twenty-six" or "11 22 2026"
    let year = next.as_deref().and_then(|w| {
        if w.len() == 4 && w.starts_with("20") && w.chars().all(|c| c.is_ascii_digit()) {
            return Some((w.to_string(), 1));
        }
        let (y, yord, yn) = number_at(tokens, i + 1 + n + 1).filter(|_| w == "twenty")?;
        (!yord && y >= 10).then(|| (format!("20{y}"), yn + 1))
    });
    let (dtrail, dnext) = year.as_ref().map_or((trail, next.clone()), |(_, yn)| ends(n + yn));
    let word = |o: &Option<String>, list: &[&str]| o.as_deref().is_some_and(|w| list.contains(&w));
    let prev = out.last().map(|p| bare(p));
    let prev2 = out.len().checked_sub(2).map(|k| bare(&out[k]));
    let before = |list: &[&str]| word(&prev, list) || (prev.as_deref() == Some("the") && word(&prev2, list));
    let after_to_noon =
        word(&next, &["to", "until", "till"]) && tokens.get(i + 2 + n).is_some_and(|t| matches!(bare(t).as_str(), "noon" | "midnight"));

    let days = [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][lead as usize - 1];
    let date_ok = !oh && (1..=days).contains(&second);
    let time_ok = !ord && second <= 59 && (second >= 10 || oh);
    let date = || {
        let y = year.as_ref().map_or(String::new(), |(y, _)| format!("/{y}"));
        (format!("{lead}/{second}{y}{dtrail}"), 1 + n + year.as_ref().map_or(0, |(_, yn)| *yn))
    };
    let time = || (format!("{lead}:{second:02}{trail}"), 1 + n);
    // "on one two three" is a count
    let counting = year.is_none() && dnext.as_deref().is_some_and(|w| number(w).is_some());
    if counting && !word(&next, &["am", "pm", "a.m", "p.m"]) {
        return None;
    }

    if oh || word(&next, &["am", "pm", "a.m", "p.m", "o'clock", "time"]) || after_to_noon {
        return time_ok.then(time);
    }
    if ord || year.is_some() || word(&dnext, &["date", "dates", "dated", "deadline"]) || before(&["on", "due", "dated", "since", "through"])
        || (prev.as_deref() == Some("the") && before(&["from", "until", "till", "by", "before", "after"]))
    {
        return if date_ok { Some(date()) } else { time_ok.then(time) };
    }
    if before(&["at", "around", "by", "until", "till", "before", "after"]) {
        return if time_ok { Some(time()) } else { date_ok.then(date) };
    }
    // no word to go by: words_to_digits writes each number on its own
    None
}

#[derive(Clone, Copy, PartialEq)]
enum Part {
    /// one to nineteen, or a one- or two-digit number
    Unit,
    /// twenty, thirty, ... with no ones yet
    Tens,
    /// twenty-seven, or twenty and then seven
    TensUnit,
    Hundred,
    /// thousand, million, billion
    Scale,
}

/// One cardinal said in words, possibly with digits mixed in: "twenty 7", "two hundred and five".
struct Run {
    /// tokens taken
    len: usize,
    value: u64,
    /// "five million" stays "5 million"
    big: Option<&'static str>,
    ordinal: bool,
    /// a single word under ten, as said
    small_word: bool,
    trail: String,
}

/// The cardinal or ordinal starting at token i, said as words or mixed with digits.
fn number_run(tokens: &[&str], i: usize) -> Option<Run> {
    // "a hundred and fifty" is 150, but "a hundred reasons" keeps its words
    if matches!(tokens[i], "a" | "A") && tokens.get(i + 1).is_some_and(|t| matches!(*t, "hundred" | "thousand")) {
        let run = number_run_from(tokens, i + 1, 1, Some(Part::Unit))?;
        return (run.len > 1).then(|| Run { len: run.len + 1, small_word: false, ..run });
    }
    number_run_from(tokens, i, 0, None)
}

fn number_run_from(tokens: &[&str], i: usize, start: u64, start_part: Option<Part>) -> Option<Run> {
    let mut total = 0u64;
    let mut current = start;
    let mut last: Option<Part> = start_part;
    let mut big = None;
    let mut last_scale = 0u64;
    let mut ordinal = false;
    let mut words = start as usize;
    let mut k = i;
    let mut trail = String::new();
    while let Some(t) = tokens.get(k) {
        if t.starts_with(|c: char| !c.is_alphanumeric()) {
            break;
        }
        let w = bare(t);
        let is_digits = w.chars().all(|c| c.is_ascii_digit());
        let (part, v, ord) = match w.as_str() {
            "hundred" => (Part::Hundred, 100, false),
            "thousand" => (Part::Scale, 1_000, false),
            "million" => (Part::Scale, 1_000_000, false),
            "billion" => (Part::Scale, 1_000_000_000, false),
            "and" if matches!(last, Some(Part::Hundred | Part::Scale))
                && tokens.get(k + 1).is_some_and(|n| !n.starts_with(|c: char| !c.is_alphanumeric()) && number(&bare(n)).is_some_and(|(v, o)| v > 0 && !o))
                && !t.ends_with(|c: char| !c.is_alphanumeric()) =>
            {
                k += 1;
                continue;
            }
            _ if is_digits && !w.is_empty() && w.len() <= 2 => (Part::Unit, w.parse().unwrap_or(0), false),
            "zero" => (Part::Unit, 0, false),
            _ => {
                let Some((v, ord)) = number(&w) else { break };
                let part = match (w.contains('-'), v) {
                    (true, _) => Part::TensUnit,
                    (false, v) if v >= 20 && v % 10 == 0 && !ord => Part::Tens,
                    (false, v) if v >= 20 => Part::TensUnit,
                    _ => Part::Unit,
                };
                (part, v as u64, ord)
            }
        };
        // whether this token carries on the number said so far
        let fits = match (last, part) {
            (None, Part::Hundred | Part::Scale) => false,
            (None, _) => true,
            // "a thirty second clip" is a length of time
            (Some(Part::Tens), Part::Unit) => (1..10).contains(&v) && w != "second",
            (Some(Part::Hundred | Part::Scale), Part::Unit | Part::Tens | Part::TensUnit) => v > 0,
            (Some(Part::Unit | Part::Tens | Part::TensUnit), Part::Hundred) => (1..100).contains(&current),
            // "two million three thousand", never "three thousand two million"
            (Some(Part::Unit | Part::Tens | Part::TensUnit | Part::Hundred), Part::Scale) => current > 0 && (last_scale == 0 || v < last_scale),
            _ => false,
        };
        if !fits {
            break;
        }
        match part {
            Part::Hundred => current *= 100,
            Part::Scale => {
                last_scale = v;
                if v >= 1_000_000 && total == 0 && current < 1000 && tokens.get(k + 1).is_none_or(|n| number(&bare(n)).is_none()) {
                    big = Some(if v == 1_000_000 { "million" } else { "billion" });
                } else {
                    total += current * v;
                    current = 0;
                }
            }
            _ => current += v,
        }
        if !is_digits {
            words += 1;
        }
        last = Some(if part == Part::Unit && last == Some(Part::Tens) { Part::TensUnit } else { part });
        k += 1;
        let tail = &t[t.trim_end_matches(|c: char| !c.is_alphanumeric()).len()..];
        if ord || big.is_some() || !tail.is_empty() {
            ordinal = ord;
            trail = tail.to_string();
            break;
        }
    }
    // "twenty five-year-old" stays words
    if last == Some(Part::Tens)
        && tokens.get(k).is_some_and(|t| t.split_once('-').is_some_and(|(u, _)| number(&u.to_ascii_lowercase()).is_some_and(|(v, _)| v < 10)))
    {
        return None;
    }
    let len = k - i;
    if len == 0 || words == 0 || matches!(last, Some(Part::Hundred | Part::Scale)) && total + current == 0 {
        return None;
    }
    let value = total + current;
    Some(Run { len, value, big, ordinal, small_word: len == 1 && value < 10, trail })
}

fn with_suffix(n: u64) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

/// "Q 3" or "Q three" → "Q3". First, so "Q 3 twenty twenty 7" is not read as March 20th.
fn join_quarters(text: &str) -> String {
    let tokens: Vec<&str> = text.split(' ').collect();
    let mut out: Vec<String> = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        let quarter = matches!(tokens[i], "Q" | "q")
            .then(|| tokens.get(i + 1))
            .flatten()
            .filter(|t| !t.starts_with(|c: char| !c.is_alphanumeric()))
            .and_then(|t| number(&bare(t)).filter(|(n, ord)| (1..=4).contains(n) && !ord).map(|(n, _)| (n, t)));
        if let Some((n, t)) = quarter {
            let trail = &t[t.trim_end_matches(|c: char| !c.is_alphanumeric()).len()..];
            out.push(format!("Q{n}{trail}"));
            i += 2;
            continue;
        }
        out.push(tokens[i].to_string());
        i += 1;
    }
    out.join(" ")
}

/// Numbers the speech model wrote as words come out as digits: "twenty twenty 7" → "2027",
/// "two hundred and fifty" → "250", "twenty-first" → "21st". A single word under
/// ten stays a word ("one thing", "the second time") unless a digit is needed: "five percent",
/// "five point two".
fn words_to_digits(text: &str) -> String {
    let tokens: Vec<&str> = text.split(' ').collect();
    let mut out: Vec<String> = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        let Some(run) = number_run(&tokens, i) else {
            out.push(tokens[i].to_string());
            i += 1;
            continue;
        };
        let next = tokens.get(i + run.len).filter(|_| run.trail.is_empty()).map(|t| bare(t));
        // "twenty twenty-seven", "nineteen ninety nine": a year said in pairs
        let year = (run.trail.is_empty() && run.big.is_none() && !run.ordinal && run.len <= 1 && matches!(run.value, 19 | 20))
            .then(|| number_run(&tokens, i + run.len))
            .flatten()
            .filter(|y| (10..100).contains(&y.value) && !y.ordinal && y.big.is_none() && y.len <= 2);
        if let Some(y) = year {
            out.push(format!("{}{}{}", run.value, y.value, y.trail));
            i += run.len + y.len;
            continue;
        }
        let prev = out.last().map(|p| bare(p));
        let digit = |t: &str| t.len() == 1 && t.chars().all(|c| c.is_ascii_digit()) || ONES[..9].contains(&t) || t == "zero";
        let after_point = prev.as_deref() == Some("point")
            && out.len() >= 2
            && out[out.len() - 2].ends_with(|c: char| c.is_ascii_digit())
            && out[out.len() - 2].chars().all(|c| c.is_ascii_digit() || c == '.');
        // "point two five": each digit after the point, as one number
        if after_point && run.small_word && !run.ordinal {
            let mut frac = String::new();
            let mut k = i;
            while let Some(t) = tokens.get(k).filter(|t| digit(&bare(t)) && !t.starts_with(|c: char| !c.is_alphanumeric())) {
                let w = bare(t);
                frac.push_str(&if w.len() == 1 && w.chars().all(|c| c.is_ascii_digit()) { w.clone() } else { number(&w).map_or("0".into(), |(v, _)| v.to_string()) });
                k += 1;
                if t.ends_with(|c: char| !c.is_alphanumeric()) {
                    break;
                }
            }
            let last = tokens[k - 1];
            out.push(format!("{frac}{}", &last[last.trim_end_matches(|c: char| !c.is_alphanumeric()).len()..]));
            i = k;
            continue;
        }
        // "five percent", "five point two"; not "one point I want to make"
        let needs_digit = next.as_deref() == Some("percent")
            || next.as_deref() == Some("point") && tokens.get(i + run.len + 1).is_some_and(|t| digit(&bare(t)));
        if run.small_word && !needs_digit {
            out.push(tokens[i].to_string());
            i += 1;
            continue;
        }
        let digits = match (run.big, run.ordinal) {
            (Some(b), _) => format!("{} {b}", run.value),
            (None, true) => with_suffix(run.value),
            (None, false) => run.value.to_string(),
        };
        out.push(format!("{digits}{}", run.trail));
        i += run.len;
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
    s = apply_symbols(&join_decimals(&words_to_digits(&join_dates(&join_quarters(&s)))), &cfg.symbols, profile);
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
    fn non_ascii_words_never_panic() {
        let (d, c) = env();
        // each first word reads as a false start of the next; the point is that "ë" and "é" don't panic
        assert_eq!(clean("Zoë Zoë's bag.", &d, &Snippets::default(), &c), "Zoë's bag.");
        assert_eq!(clean("The café cafébar opens.", &d, &Snippets::default(), &c), "The cafébar opens.");
    }

    #[test]
    fn fillers_spare_real_words() {
        let (d, c) = bare_env();
        assert_eq!(clean("I went to the ER.", &d, &Snippets::default(), &c), "I went to the ER.");
        assert_eq!(clean("Use a 5 mm screw.", &d, &Snippets::default(), &c), "Use a 5 mm screw.");
        assert_eq!(clean("Er, send it.", &d, &Snippets::default(), &c), "Send it.");
    }

    #[test]
    fn a_sentence_ending_number_takes_no_decimal() {
        let (d, c) = bare_env();
        assert_eq!(clean("I counted 5. Point 2 is next.", &d, &Snippets::default(), &c), "I counted 5. Point 2 is next.");
    }

    #[test]
    fn word_restarted_with_an_ending_is_dropped() {
        let (d, c) = bare_env();
        for (raw, want) in [
            ("Okay, I'm guess guessing I was mistaken.", "Okay, I'm guessing I was mistaken."),
            ("She send sends it daily.", "She sends it daily."),
            ("We test tested it.", "We tested it."),
            ("I was make making dinner.", "I was making dinner."),
            ("He was run running late.", "He was running late."),
        ] {
            assert_eq!(clean(raw, &d, &Snippets::default(), &c), want);
        }
        for keep in ["Give it its due.", "Give her hers.", "Done. Doing more."] {
            assert_eq!(clean(keep, &d, &Snippets::default(), &c), keep);
        }
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
    fn spoken_dates_and_times_become_digits() {
        let (d, c) = bare_env();
        for (said, want) in [
            (
                "I'm using the App, then this item should be on eleven twenty-nine, right? Not today.",
                "I'm using the App, then this item should be on 11/29, right? Not today.",
            ),
            ("Move it on eleven twenty nine.", "Move it on 11/29."),
            ("It ships on 3 15.", "It ships on 3/15."),
            ("Due on eleven eleven.", "Due on 11/11."),
            ("Due on seven seventeen.", "Due on 7/17."),
            ("On twelve thirty-one we close.", "On 12/31 we close."),
            ("We close on twelve thirty-one.", "We close on 12/31."),
            // the words around it say date
            (
                "So I'm considering everything from the eleven twenty 2 date.",
                "So I'm considering everything from the 11/22 date.",
            ),
            ("From the eleven twenty-two date on.", "From the 11/22 date on."),
            ("Past the eleven twenty two deadline.", "Past the 11/22 deadline."),
            ("Since the 11 22 date.", "Since the 11/22 date."),
            ("Everything from the eleven twenty-two forward.", "Everything from the 11/22 forward."),
            ("It's due eleven twenty-two.", "It's due 11/22."),
            ("Ship it eleven twenty-second.", "Ship it 11/22."),
            ("Ship it eleven 22nd.", "Ship it 11/22."),
            ("Due on eleven twenty two twenty twenty-six.", "Due on 11/22/2026."),
            ("Due eleven twenty-two 2026.", "Due 11/22/2026."),
            // no word to go by: each number on its own
            ("Eleven twenty-two works for me.", "11 22 works for me."),
            ("Give me one second.", "Give me one second."),
            ("Room two twenty.", "Room two 20."),
            // the words around it say time
            ("Call on two thirty.", "Call on 2:30."),
            ("Call on two thirty pm.", "Call on 2:30 pm."),
            ("Done by three thirty.", "Done by 3:30."),
            ("Meet at eleven twenty-two.", "Meet at 11:22."),
            ("Meet at eleven oh five.", "Meet at 11:05."),
            ("From eleven twenty to noon.", "From 11:20 to noon."),
            ("Lunch at twelve forty-five.", "Lunch at 12:45."),
            ("Eleven twenty-two time works.", "11:22 time works."),
            // not a date or a time
            ("Focus on one two three.", "Focus on one two three."),
            ("Focus on one thing.", "Focus on one thing."),
            ("Pick one two.", "Pick one two."),
            ("Ready on eleven, twenty people.", "Ready on 11, 20 people."),
        ] {
            assert_eq!(clean(said, &d, &Snippets::default(), &c), want, "{said}");
        }
    }

    #[test]
    fn spoken_numbers_become_digits() {
        let (d, c) = bare_env();
        for (said, want) in [
            (
                "Well, the Q 3 twenty twenty 7, once again, is for applications in general.",
                "Well, the Q3 2027, once again, is for applications in general.",
            ),
            ("I think that would go well into twenty twenty-eight.", "I think that would go well into 2028."),
            ("Back in nineteen ninety nine.", "Back in 1999."),
            ("By two thousand twenty six.", "By 2026."),
            ("Q three numbers are up.", "Q3 numbers are up."),
            ("Twenty two people came.", "22 people came."),
            ("We have twenty-five open tickets.", "We have 25 open tickets."),
            ("About two hundred and fifty users.", "About 250 users."),
            ("Three thousand five hundred rows.", "3500 rows."),
            ("It cost five million.", "It cost 5 million."),
            ("The twenty-first century.", "The 21st century."),
            ("Ten items, then eleven.", "10 items, then 11."),
            ("It grew five percent.", "It grew 5%."),
            ("Version five point two.", "Version 5.2."),
            ("Rate is five point two five percent.", "Rate is 5.25%."),
            ("There is one point I want to make.", "There is one point I want to make."),
            ("A three point turn.", "A three point turn."),
            ("A thirty second clip.", "A 30 second clip."),
            ("The twenty-second item.", "The 22nd item."),
            ("I paid a hundred and fifty.", "I paid 150."),
            ("A hundred twenty people.", "120 people."),
            ("A twenty five-year-old.", "A twenty five-year-old."),
            ("Twenty zero.", "20 zero."),
            ("It's 20 twenty-five now.", "It's 20 25 now."),
            // a single word under ten stays a word
            ("Focus on one thing.", "Focus on one thing."),
            ("No one came the second time.", "No one came the second time."),
            ("I have three kids.", "I have three kids."),
            ("A hundred reasons.", "A hundred reasons."),
            ("Hundreds of them.", "Hundreds of them."),
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
