//! Editing rules for the dictionary editor, kept free of egui so they can be unit-tested.

use crate::dictionary::{Dictionary, Term};
use crate::phonetic;

#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    pub term: usize,
    /// Which spoken form, when the issue is about one.
    pub spoken: Option<usize>,
    /// Errors block Save; warnings don't.
    pub error: bool,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Deleted {
    pub index: usize,
    pub term: Term,
}

fn fold(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

pub fn validate(terms: &[Term]) -> Vec<Issue> {
    let mut out = Vec::new();
    for (i, t) in terms.iter().enumerate() {
        let w = fold(&t.written);
        if w.is_empty() {
            out.push(Issue { term: i, spoken: None, error: true, message: "Written form can't be empty".into() });
        } else if (0..terms.len()).any(|j| j != i && fold(&terms[j].written) == w) {
            out.push(Issue { term: i, spoken: None, error: true, message: format!("'{}' is already a term", t.written.trim()) });
        }
        for (k, s) in t.spoken.iter().enumerate() {
            let s = fold(s);
            if s.is_empty() {
                continue;
            }
            if let Some(j) = (0..terms.len()).find(|&j| j != i && terms[j].spoken.iter().any(|x| fold(x) == s)) {
                out.push(Issue {
                    term: i,
                    spoken: Some(k),
                    error: true,
                    message: format!("'{s}' is also a spoken form of '{}'", terms[j].written.trim()),
                });
            }
            if !s.contains(' ') && phonetic::is_common(&s) {
                out.push(Issue {
                    term: i,
                    spoken: Some(k),
                    error: false,
                    message: format!("'{s}' is a common word: this will rewrite '{s}' everywhere"),
                });
            }
        }
    }
    out
}

/// What Save writes: trimmed written form; spoken forms trimmed, single-spaced, lower-cased,
/// empties dropped and duplicates removed.
pub fn normalize(terms: Vec<Term>) -> Vec<Term> {
    terms
        .into_iter()
        .map(|t| {
            let mut spoken: Vec<String> = Vec::new();
            for s in t.spoken {
                let s = fold(&s);
                if !s.is_empty() && !spoken.contains(&s) {
                    spoken.push(s);
                }
            }
            Term { written: t.written.trim().to_string(), spoken, phonetic: t.phonetic }
        })
        .collect()
}

/// Indices of the terms to list: matching `query` (written or spoken, ignoring case), A–Z by written form.
pub fn visible(terms: &[Term], query: &str) -> Vec<usize> {
    let q = fold(query);
    let mut v: Vec<usize> = (0..terms.len())
        .filter(|&i| q.is_empty() || fold(&terms[i].written).contains(&q) || terms[i].spoken.iter().any(|s| fold(s).contains(&q)))
        .collect();
    v.sort_by_key(|&i| (fold(&terms[i].written), i));
    v
}

pub fn delete(terms: &mut Vec<Term>, index: usize) -> Deleted {
    Deleted { index, term: terms.remove(index) }
}

pub fn undo(terms: &mut Vec<Term>, d: Deleted) -> usize {
    let at = d.index.min(terms.len());
    terms.insert(at, d.term);
    at
}

/// Terms added, removed or edited; an edit counts once (it's one term missing on each side).
pub fn changes(loaded: &[Term], working: &[Term]) -> usize {
    fn missing(a: &[Term], b: &[Term]) -> usize {
        let mut pool: Vec<&Term> = b.iter().collect();
        a.iter()
            .filter(|t| match pool.iter().position(|x| x == t) {
                Some(p) => {
                    pool.swap_remove(p);
                    false
                }
                None => true,
            })
            .count()
    }
    missing(working, loaded).max(missing(loaded, working))
}

/// `text` as the (unsaved) terms would rewrite it. Terms with no written form are skipped:
/// an empty phrase would match every punctuation-only token.
pub fn preview(terms: &[Term], text: &str) -> String {
    let usable = normalize(terms.to_vec()).into_iter().filter(|t| !t.written.is_empty()).collect();
    Dictionary::from_terms(usable).apply(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(written: &str, spoken: &[&str]) -> Term {
        Term { written: written.into(), spoken: spoken.iter().map(|s| s.to_string()).collect(), phonetic: true }
    }

    fn errors(terms: &[Term]) -> Vec<Issue> {
        validate(terms).into_iter().filter(|i| i.error).collect()
    }

    #[test]
    fn clean_list_has_no_issues() {
        assert!(validate(&[t("HAWB", &["hob"]), t("BOL", &["bee oh el"])]).is_empty());
    }

    #[test]
    fn empty_written_is_an_error() {
        let e = errors(&[t("  ", &[])]);
        assert_eq!(e.len(), 1);
        assert_eq!((e[0].term, e[0].spoken), (0, None));
    }

    #[test]
    fn duplicate_written_ignoring_case_flags_both() {
        let e = errors(&[t("HAWB", &[]), t("hawb ", &[])]);
        assert_eq!(e.iter().map(|i| i.term).collect::<Vec<_>>(), vec![0, 1]);
    }

    #[test]
    fn shared_spoken_ignores_case_and_spaces() {
        let e = errors(&[t("HAWB", &["Hob "]), t("Hobart", &["hob"])]);
        assert_eq!(e.iter().map(|i| (i.term, i.spoken)).collect::<Vec<_>>(), vec![(0, Some(0)), (1, Some(0))]);
    }

    #[test]
    fn empty_spoken_fields_are_ignored() {
        assert!(validate(&[t("HAWB", &[""]), t("BOL", &["  "])]).is_empty());
    }

    #[test]
    fn common_word_spoken_is_a_warning_only() {
        let issues = validate(&[t("Call", &["all"])]);
        assert_eq!(issues.len(), 1);
        assert!(!issues[0].error);
        assert_eq!(issues[0].spoken, Some(0));
        assert!(issues[0].message.contains("'all'"));
    }

    #[test]
    fn normalize_trims_dedupes_and_lowercases_spoken() {
        let n = normalize(vec![t(" HAWB ", &["Hob", " hob", "", "  bee   oh el "])]);
        assert_eq!(n, vec![t("HAWB", &["hob", "bee oh el"])]);
    }

    #[test]
    fn normalize_keeps_phonetic_flag() {
        let mut x = t("BOL", &[]);
        x.phonetic = false;
        assert!(!normalize(vec![x])[0].phonetic);
    }

    #[test]
    fn visible_sorts_ignoring_case_and_filters_written_and_spoken() {
        let terms = [t("jira", &["jerry"]), t("BOL", &["bee oh el"]), t("HAWB", &["hob"])];
        assert_eq!(visible(&terms, ""), vec![1, 2, 0]);
        assert_eq!(visible(&terms, "JER"), vec![0]);
        assert_eq!(visible(&terms, " hawb "), vec![2]);
        assert!(visible(&terms, "zzz").is_empty());
    }

    #[test]
    fn new_empty_term_sorts_first() {
        assert_eq!(visible(&[t("BOL", &[]), t("", &[])], ""), vec![1, 0]);
    }

    #[test]
    fn delete_then_undo_restores_position() {
        let mut terms = vec![t("A1", &[]), t("B2", &[]), t("C3", &[])];
        let before = terms.clone();
        let d = delete(&mut terms, 1);
        assert_eq!(terms.len(), 2);
        assert_eq!(undo(&mut terms, d), 1);
        assert_eq!(terms, before);
    }

    #[test]
    fn undo_clamps_index_when_list_shrank() {
        let mut terms = vec![t("A1", &[]), t("B2", &[])];
        let d = delete(&mut terms, 1);
        terms.clear();
        assert_eq!(undo(&mut terms, d), 0);
    }

    #[test]
    fn changes_counts_added_removed_and_edited_terms() {
        let loaded = vec![t("A1", &[]), t("B2", &[]), t("C3", &[])];
        assert_eq!(changes(&loaded, &loaded), 0);
        let mut edited = loaded.clone();
        edited[0].written = "A9".into();
        assert_eq!(changes(&loaded, &edited), 1);
        let mut added = loaded.clone();
        added.push(t("D4", &[]));
        assert_eq!(changes(&loaded, &added), 1);
        let mut edit_and_delete = edited.clone();
        edit_and_delete.remove(1);
        assert_eq!(changes(&loaded, &edit_and_delete), 2);
    }

    #[test]
    fn preview_applies_unsaved_terms() {
        assert_eq!(preview(&[t("HAWB", &[" Hob "])], "send the hob"), "send the HAWB");
    }

    #[test]
    fn preview_ignores_terms_with_empty_written_form() {
        assert_eq!(preview(&[t("", &["hob"]), t("  ", &[])], "send the hob — now"), "send the hob — now");
    }
}
