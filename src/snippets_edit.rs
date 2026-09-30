//! Editing rules for the snippets editor, kept free of egui so they can be unit-tested.

use crate::phonetic;
use crate::snippets::{self, Snippet, Snippets};

pub use crate::dictionary_edit::changes;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Field {
    Trigger,
    Text,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    pub snippet: usize,
    pub field: Field,
    /// Errors block Save; warnings don't.
    pub error: bool,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Deleted {
    pub index: usize,
    pub snippet: Snippet,
}

fn fold(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

pub fn validate(list: &[Snippet]) -> Vec<Issue> {
    let mut out = Vec::new();
    for (i, s) in list.iter().enumerate() {
        let w = snippets::words(&s.trigger);
        let trigger = |error, message| Issue { snippet: i, field: Field::Trigger, error, message };
        if w.is_empty() {
            out.push(trigger(true, "Trigger needs at least one word".into()));
        } else if (0..list.len()).any(|j| j != i && snippets::words(&list[j].trigger) == w) {
            out.push(trigger(true, format!("'{}' is already a trigger", w.join(" "))));
        } else if let [word] = w.as_slice() {
            if phonetic::is_common(word) {
                out.push(trigger(false, format!("'{word}' is a common word: this will paste every time you say '{word}'")));
            }
        }
        if s.text.trim().is_empty() {
            out.push(Issue { snippet: i, field: Field::Text, error: true, message: "Text can't be empty".into() });
        }
    }
    out
}

/// What Save writes: the trigger trimmed, the text exactly as typed.
pub fn normalize(list: Vec<Snippet>) -> Vec<Snippet> {
    list.into_iter().map(|s| Snippet { trigger: s.trigger.trim().to_string(), text: s.text }).collect()
}

/// Indices of the snippets to list: matching `query` (trigger or text, ignoring case), A–Z by
/// trigger words, so case and punctuation don't move a row.
pub fn visible(list: &[Snippet], query: &str) -> Vec<usize> {
    let q = fold(query);
    let mut v: Vec<usize> =
        (0..list.len()).filter(|&i| q.is_empty() || fold(&list[i].trigger).contains(&q) || fold(&list[i].text).contains(&q)).collect();
    v.sort_by_key(|&i| (snippets::words(&list[i].trigger).join(" "), i));
    v
}

pub fn delete(list: &mut Vec<Snippet>, index: usize) -> Deleted {
    Deleted { index, snippet: list.remove(index) }
}

pub fn undo(list: &mut Vec<Snippet>, d: Deleted) -> usize {
    let at = d.index.min(list.len());
    list.insert(at, d.snippet);
    at
}

/// `text` with the (unsaved) snippets expanded. Snippets with no trigger words or no text are
/// skipped: one never matches, the other would delete the phrase.
pub fn preview(list: &[Snippet], text: &str) -> String {
    let usable = normalize(list.to_vec())
        .into_iter()
        .filter(|s| !snippets::words(&s.trigger).is_empty() && !s.text.trim().is_empty())
        .collect();
    let (marked, expansions) = Snippets { snippets: usable }.mark(text);
    snippets::restore(&marked, &expansions)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(trigger: &str, text: &str) -> Snippet {
        Snippet { trigger: trigger.into(), text: text.into() }
    }

    fn errors(list: &[Snippet]) -> Vec<(usize, Field)> {
        validate(list).into_iter().filter(|i| i.error).map(|i| (i.snippet, i.field)).collect()
    }

    #[test]
    fn clean_list_has_no_issues() {
        assert!(validate(&[s("my signature", "Best,\nJeff"), s("my email", "me@example.com")]).is_empty());
    }

    #[test]
    fn trigger_without_words_is_an_error() {
        assert_eq!(errors(&[s("  ", "x"), s(" ?! ", "y")]), vec![(0, Field::Trigger), (1, Field::Trigger)]);
    }

    #[test]
    fn empty_or_blank_text_is_an_error() {
        assert_eq!(errors(&[s("sig", ""), s("sig two", " \n\t ")]), vec![(0, Field::Text), (1, Field::Text)]);
    }

    #[test]
    fn duplicate_triggers_ignore_case_and_punctuation() {
        let list = [s("My email.", "a"), s("my  email", "b"), s("my emails", "c")];
        assert_eq!(errors(&list), vec![(0, Field::Trigger), (1, Field::Trigger)]);
    }

    #[test]
    fn common_one_word_trigger_is_a_warning_only() {
        let issues = validate(&[s("All.", "x")]);
        assert_eq!(issues.len(), 1);
        assert!(!issues[0].error);
        assert_eq!(issues[0].field, Field::Trigger);
        assert!(issues[0].message.contains("'all'"), "{}", issues[0].message);
        assert!(validate(&[s("all done", "x")]).is_empty(), "a common word inside a longer trigger is fine");
    }

    #[test]
    fn normalize_trims_the_trigger_and_leaves_text_alone() {
        assert_eq!(normalize(vec![s("  my sig ", "\n  Best, \n")]), vec![s("my sig", "\n  Best, \n")]);
    }

    #[test]
    fn visible_sorts_by_trigger_and_filters_trigger_and_text() {
        let list = [s("zeta", "Best, Jeff"), s("Alpha", "me@example.com"), s("", ""), s("(beta)", "x")];
        assert_eq!(visible(&list, ""), vec![2, 1, 3, 0], "A–Z ignoring case and punctuation, an empty trigger first");
        assert_eq!(visible(&list, "BEST"), vec![0]);
        assert_eq!(visible(&list, " alp "), vec![1]);
        assert!(visible(&list, "zzz").is_empty());
    }

    #[test]
    fn delete_then_undo_restores_position_and_clamps() {
        let mut list = vec![s("a", "1"), s("b", "2"), s("c", "3")];
        let before = list.clone();
        let d = delete(&mut list, 1);
        assert_eq!(undo(&mut list, d), 1);
        assert_eq!(list, before);
        let d = delete(&mut list, 2);
        list.clear();
        assert_eq!(undo(&mut list, d), 0);
    }

    #[test]
    fn changes_counts_added_removed_and_edited() {
        let loaded = vec![s("a", "1"), s("b", "2")];
        let mut working = loaded.clone();
        working[0].text = "9".into();
        working.push(s("c", "3"));
        assert_eq!(changes(&loaded, &working), 2);
    }

    #[test]
    fn preview_expands_unsaved_snippets_and_skips_invalid_ones() {
        let list = [s(" sig ", "Best,\nJeff"), s("", "never"), s("oops", "")];
        assert_eq!(preview(&list, "thanks sig."), "thanks Best,\nJeff");
        assert_eq!(preview(&list, "say oops now"), "say oops now");
    }
}
