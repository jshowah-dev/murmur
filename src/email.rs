//! Email shaping: a dictated greeting gets its own line, a dictated sign-off its own lines.
//! Runs on tidied text, before snippet placeholders are restored.

const OPENERS: &[&[&str]] = &[
    &["good", "morning"],
    &["good", "afternoon"],
    &["good", "evening"],
    &["hi"],
    &["hello"],
    &["hey"],
    &["dear"],
    &["greetings"],
];

const CLOSERS: &[&[&str]] = &[
    &["thanks", "so", "much"],
    &["thanks", "again"],
    &["many", "thanks"],
    &["thank", "you"],
    &["best", "regards"],
    &["kind", "regards"],
    &["warm", "regards"],
    &["talk", "soon"],
    &["thanks"],
    &["best"],
    &["regards"],
    &["cheers"],
    &["sincerely"],
];

/// Lowercase words that can stand where a name would after a greeting opener.
const GROUP_WORDS: &[&str] = &["there", "all", "everyone", "everybody", "team", "folks", "both", "and"];

/// Their full stop is part of the word, not the end of the greeting.
const TITLES: &[&str] = &["mr", "mrs", "ms", "dr", "prof"];

const MAX_NAME_WORDS: usize = 3;

/// Lowercased token with the recogniser's punctuation stripped.
fn bare(tok: &str) -> String {
    tok.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()
}

fn strip_end(tok: &str) -> &str {
    tok.trim_end_matches([',', '.', '!'])
}

fn is_title(tok: &str) -> bool {
    TITLES.contains(&bare(tok).as_str())
}

/// A capitalised word (or, in a greeting, a group word). Snippet placeholders are not letters,
/// so a snippet is never taken for a name.
fn name_like(tok: &str, group_ok: bool) -> bool {
    let core = strip_end(tok);
    let Some(first) = core.chars().next() else { return false };
    if !core.chars().all(|c| c.is_alphabetic() || matches!(c, '-' | '\'' | '’')) {
        return false;
    }
    first.is_uppercase() || (group_ok && GROUP_WORDS.contains(&core.to_lowercase().as_str()))
}

/// Word count of the longest phrase in `list` that `tokens` starts with.
fn leading_phrase(tokens: &[&str], list: &[&[&str]]) -> Option<usize> {
    list.iter().filter(|p| tokens.len() >= p.len() && p.iter().zip(tokens).all(|(w, t)| bare(t) == *w)).map(|p| p.len()).max()
}

/// Word count of the longest phrase in `list` that `tokens` ends with.
fn trailing_phrase(tokens: &[&str], list: &[&[&str]]) -> Option<usize> {
    list.iter()
        .filter(|p| tokens.len() >= p.len() && p.iter().zip(&tokens[tokens.len() - p.len()..]).all(|(w, t)| bare(t) == *w))
        .map(|p| p.len())
        .max()
}

fn capitalise(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// A greeting at the start of `text`: the greeting line, ending in a comma, and the text after it.
fn split_greeting(text: &str) -> Option<(String, &str)> {
    let line = &text[..text.find('\n').unwrap_or(text.len())];
    let tokens: Vec<&str> = line.split(' ').collect();
    let opener = leading_phrase(&tokens, OPENERS)?;
    let opener_end = tokens[opener - 1];
    // tokens[..end] are the greeting
    let mut end = opener;
    let mut found = opener_end.ends_with(['.', '!']) || opener == tokens.len();
    if !found {
        for j in opener..tokens.len().min(opener + MAX_NAME_WORDS) {
            if !name_like(tokens[j], true) {
                break;
            }
            if j + 1 == tokens.len() || (tokens[j].ends_with([',', '.', '!']) && !is_title(tokens[j])) {
                end = j + 1;
                found = true;
                break;
            }
        }
    }
    // "Hey, can you send it": no name, but the comma still ends the greeting
    if !found && !opener_end.ends_with(',') {
        return None;
    }
    let words: Vec<&str> = tokens[..end].iter().enumerate().map(|(i, t)| if i + 1 < end && is_title(t) { *t } else { strip_end(t) }).collect();
    let consumed = tokens[..end].iter().map(|t| t.len() + 1).sum::<usize>().min(line.len());
    Some((format!("{},", words.join(" ")), text[consumed..].trim_start_matches('\n')))
}

/// `text` with a trailing sign-off moved onto its own lines; `None` when it does not end in one.
fn split_signoff(text: &str) -> Option<String> {
    let line_start = text.rfind('\n').map_or(0, |i| i + 1);
    let tokens: Vec<&str> = text[line_start..].split(' ').collect();
    for names in 0..=MAX_NAME_WORDS.min(tokens.len() - 1) {
        let (head, name) = tokens.split_at(tokens.len() - names);
        let Some(len) = trailing_phrase(head, CLOSERS) else { continue };
        let start = head.len() - len;
        if start > 0 && !head[start - 1].ends_with(['.', '!', '?']) {
            continue;
        }
        let is_name = name.iter().enumerate().all(|(i, t)| name_like(t, false) && (i + 1 == name.len() || !t.ends_with([',', '.', '!'])));
        if !is_name {
            continue;
        }
        let closer = if name.is_empty() {
            head[start..].join(" ")
        } else {
            let words: Vec<&str> = head[start..].iter().map(|t| strip_end(t)).collect();
            format!("{},\n{}", words.join(" "), strip_end(&name.join(" ")))
        };
        let before = format!("{}{}", &text[..line_start], head[..start].join(" "));
        let before = before.trim_end_matches(['\n', ' ']);
        return Some(if before.is_empty() { format!("{}{closer}", &text[..line_start]) } else { format!("{before}\n\n{closer}") });
    }
    None
}

pub fn format(text: &str) -> String {
    match split_greeting(text) {
        Some((greeting, rest)) => {
            let rest = capitalise(rest);
            let rest = split_signoff(&rest).unwrap_or(rest);
            format!("{greeting}\n\n{rest}")
        }
        None => split_signoff(text).unwrap_or_else(|| text.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::format;

    fn unchanged(text: &str) {
        assert_eq!(format(text), text);
    }

    #[test]
    fn greeting_and_signoff_together() {
        assert_eq!(
            format("Hi Sarah, thanks for the update. I'll review it tomorrow and send notes by Friday. Thanks, Jeff."),
            "Hi Sarah,\n\nThanks for the update. I'll review it tomorrow and send notes by Friday.\n\nThanks,\nJeff"
        );
    }

    #[test]
    fn greeting_rows() {
        assert_eq!(format("Hi Sarah, thanks for the update."), "Hi Sarah,\n\nThanks for the update.");
        assert_eq!(format("Hi, Sarah. Thanks for the update."), "Hi Sarah,\n\nThanks for the update.");
        assert_eq!(format("Good morning, team. The build is ready."), "Good morning team,\n\nThe build is ready.");
        assert_eq!(format("Hello. Quick question."), "Hello,\n\nQuick question.");
        assert_eq!(format("Hello there, I wanted to ask."), "Hello there,\n\nI wanted to ask.");
        assert_eq!(format("Hi Sarah and Tom, the build is ready."), "Hi Sarah and Tom,\n\nThe build is ready.");
    }

    #[test]
    fn greeting_only_ends_with_a_blank_line() {
        assert_eq!(format("Hi Sarah."), "Hi Sarah,\n\n");
        assert_eq!(format("Hi Sarah"), "Hi Sarah,\n\n");
        assert_eq!(format("Good morning."), "Good morning,\n\n");
    }

    #[test]
    fn greeting_does_not_fire() {
        unchanged("Hi Sarah and Tom and Priya and Dev, the build is ready.");
        unchanged("The build is ready. Hi Sarah, thanks.");
        unchanged("Hey Jeff can you send the file");
        unchanged("History is the topic today.");
        unchanged("\nHi Sarah, thanks for the update.");
    }

    #[test]
    fn opener_without_a_name() {
        assert_eq!(format("Hi, just checking in. Are you free?"), "Hi,\n\nJust checking in. Are you free?");
        assert_eq!(format("Hey, can you send the file?"), "Hey,\n\nCan you send the file?");
    }

    #[test]
    fn titles_and_non_ascii_names() {
        assert_eq!(format("Dear Mr. Smith, the report is attached."), "Dear Mr. Smith,\n\nThe report is attached.");
        assert_eq!(format("Hi José, ça va?"), "Hi José,\n\nÇa va?");
        assert_eq!(format("See you then. Thanks, Zoë."), "See you then.\n\nThanks,\nZoë");
    }

    #[test]
    fn signoff_rows() {
        assert_eq!(format("I'll send notes by Friday. Thanks, Jeff."), "I'll send notes by Friday.\n\nThanks,\nJeff");
        assert_eq!(format("I'll send notes by Friday. Best regards. Jeff Showah."), "I'll send notes by Friday.\n\nBest regards,\nJeff Showah");
        assert_eq!(format("I'll send notes by Friday. Thanks."), "I'll send notes by Friday.\n\nThanks.");
        assert_eq!(format("Thanks, Jeff."), "Thanks,\nJeff");
        assert_eq!(format("Is Friday okay? Thank you, Jeff"), "Is Friday okay?\n\nThank you,\nJeff");
    }

    #[test]
    fn signoff_does_not_fire() {
        unchanged("Thanks for the update, I'll look tomorrow.");
        unchanged("I said thanks, Jeff.");
        unchanged("I think that works best.");
        unchanged("Thanks.");
        unchanged("Send it over. Thanks, Jeff Robert Alan Showah.");
    }

    #[test]
    fn closing_sentences_are_not_signoffs() {
        unchanged("I'll send notes by Friday. Thanks a lot.");
        unchanged("I'll send notes by Friday. Thank you very much.");
        unchanged("I'll send notes by Friday. Thanks, I appreciate it.");
    }

    #[test]
    fn existing_line_breaks_are_not_doubled() {
        assert_eq!(format("Hi Sarah,\nThanks for the update."), "Hi Sarah,\n\nThanks for the update.");
        assert_eq!(format("Hi Sarah,\n\nThanks for the update."), "Hi Sarah,\n\nThanks for the update.");
        assert_eq!(format("Send it Friday.\n\nThanks, Jeff."), "Send it Friday.\n\nThanks,\nJeff");
        assert_eq!(format("Send it Friday.\nThanks, Jeff."), "Send it Friday.\n\nThanks,\nJeff");
        unchanged("Send it Friday. Thanks.\n");
        unchanged("Thanks.\nJeff.");
    }

    #[test]
    fn placeholders_are_never_names() {
        unchanged("\u{E000}");
        unchanged("Thanks. \u{E000}");
        unchanged("Send it Friday. Thanks, \u{E000}.");
        unchanged("Hi \u{E000}, the build is ready.");
        assert_eq!(format("Hi Sarah, \u{E000}"), "Hi Sarah,\n\n\u{E000}");
    }

    #[test]
    fn middle_of_an_email_is_untouched() {
        unchanged("");
        unchanged("The numbers look right. I'll confirm with finance on Monday.");
    }
}
