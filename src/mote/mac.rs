//! The mote until its macOS window lands (phase 3): it never flies and says nothing, so the
//! app loop treats every dictation as landing without it.

use super::{Message, Pt, Target};
use anyhow::Result;

pub(crate) struct Mote;

impl Mote {
    pub(crate) fn create() -> Result<Mote> {
        Ok(Mote)
    }

    pub(crate) fn launch(&mut self, _from: Pt, _to: Pt) {}

    pub(crate) fn dissolve(&mut self) {}

    pub(crate) fn fade(&mut self) {}

    pub(crate) fn is_active(&self) -> bool {
        false
    }

    pub(crate) fn say(&mut self, m: Message, _from: Pt, _to: Target) {
        log::info!("mote would say: {}", text(&m));
    }

    pub(crate) fn say_until_dismissed(&mut self, m: Message, from: Pt, to: Target) {
        self.say(m, from, to);
    }

    pub(crate) fn dismiss(&mut self) {}

    pub(crate) fn is_speaking(&self) -> bool {
        false
    }

    pub(crate) fn is_saying(&self, _m: &Message) -> bool {
        false
    }

    pub(crate) fn animate(&mut self) {}
}

fn text(m: &Message) -> String {
    m.0.iter().map(|(s, _)| s.as_str()).collect()
}
