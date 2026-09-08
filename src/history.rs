#![allow(dead_code)]
use std::collections::VecDeque;
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct InjectRecord {
    pub hwnd: isize,
    pub len: usize,
    pub at: Instant,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub raw: String,
    pub cleaned: String,
    pub inject: Option<InjectRecord>,
}

pub struct History {
    cap: usize,
    items: VecDeque<Entry>,
}

impl History {
    pub fn new(cap: usize) -> Self {
        History { cap, items: VecDeque::with_capacity(cap) }
    }
    pub fn push(&mut self, e: Entry) {
        if self.items.len() == self.cap {
            self.items.pop_front();
        }
        self.items.push_back(e);
    }
    pub fn last(&self) -> Option<&Entry> {
        self.items.back()
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn replace_last_cleaned(&mut self, text: String) {
        if let Some(e) = self.items.back_mut() {
            e.cleaned = text;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(s: &str) -> Entry {
        Entry { raw: s.into(), cleaned: s.into(), inject: None }
    }

    #[test]
    fn keeps_last_n() {
        let mut h = History::new(2);
        h.push(e("a"));
        h.push(e("b"));
        h.push(e("c"));
        assert_eq!(h.len(), 2);
        assert_eq!(h.last().unwrap().cleaned, "c");
    }

    #[test]
    fn empty_last_is_none() {
        assert!(History::new(3).last().is_none());
    }

    #[test]
    fn replace_last() {
        let mut h = History::new(3);
        h.push(e("a"));
        h.replace_last_cleaned("A".into());
        assert_eq!(h.last().unwrap().cleaned, "A");
    }
}
