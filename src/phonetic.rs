use rphonetic::{DoubleMetaphone, Encoder};
use std::collections::HashSet;
use std::sync::OnceLock;

static COMMON: OnceLock<HashSet<&'static str>> = OnceLock::new();

fn common() -> &'static HashSet<&'static str> {
    COMMON.get_or_init(|| include_str!("common_words.txt").lines().map(str::trim).filter(|l| !l.is_empty()).collect())
}

fn letters_only(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphabetic()).collect()
}

pub fn key(word: &str) -> String {
    let w = letters_only(word);
    if w.is_empty() {
        return String::new();
    }
    DoubleMetaphone::default().encode(&w)
}

pub fn is_common(word: &str) -> bool {
    common().contains(word.to_ascii_lowercase().as_str())
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Spec rule: Metaphone keys equal, or normalised Levenshtein <= 0.5.
pub fn similar(a: &str, b: &str) -> bool {
    let (ka, kb) = (key(a), key(b));
    if !ka.is_empty() && ka == kb {
        return true;
    }
    let la = a.to_ascii_lowercase();
    let lb = b.to_ascii_lowercase();
    let max = la.chars().count().max(lb.chars().count()).max(1);
    (levenshtein(&la, &lb) as f32 / max as f32) <= 0.5
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hob_and_hawb_share_a_key() {
        assert_eq!(key("hob"), key("HAWB"));
        assert!(!key("hob").is_empty());
    }

    #[test]
    fn unrelated_words_differ() {
        assert_ne!(key("shipment"), key("invoice"));
    }

    #[test]
    fn common_guard() {
        assert!(is_common("bowl"));
        assert!(is_common("The"));
        assert!(!is_common("hob"));
    }

    #[test]
    fn similarity_rules() {
        assert!(similar("hob", "HAWB"));        // metaphone equal
        assert!(similar("delgato", "Delgado"));   // edit distance
        assert!(!similar("very good", "excellent"));
    }
}
