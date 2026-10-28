//! The live caption: the words you're saying, shown just below the caret while you dictate
//! (above the pill when there's no caret to find). It's a click-through window of its own and
//! never touches the app you're typing in; the paste on release is what lands there.

use crate::canvas::{self, Canvas};
use crate::caret::{self, Anchor};
use crate::mote::{Pt, Surface};
use crate::platform::Rect as RECT;
use anyhow::Result;
use murmur_lib::preview::tail_of;

/// Characters kept in view: the end of what's being said.
const MAX_CHARS: usize = 64;
/// The caption's widest, in 96-dpi pixels (points on a Mac).
const MAX_W: f32 = 560.0;
const HEIGHT: f32 = 30.0;
const PAD: f32 = 12.0;
const FONT: f32 = 15.0;
/// Space between the pill's top and the caption above it.
const ABOVE_PILL: f32 = 10.0;

/// Where the caption goes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Place {
    /// Below this caret line, or above it near the bottom of the screen.
    Caret(RECT),
    /// Centred above this point: the pill's top centre.
    Pill(Pt),
}

/// Top-left for a `w`×`h` caption at `place`, kept inside `work`.
pub(crate) fn position(place: Place, w: i32, h: i32, work: RECT) -> (i32, i32) {
    match place {
        Place::Caret(r) => caret::place(&Anchor::Caret(r), w, h, work),
        Place::Pill((x, y)) => {
            let (x, y) = ((x - w as f32 / 2.0).round() as i32, (y - ABOVE_PILL - h as f32).round() as i32);
            (x.clamp(work.left, (work.right - w).max(work.left)), y.clamp(work.top, (work.bottom - h).max(work.top)))
        }
    }
}

pub(crate) struct Caption {
    surface: Surface,
    /// physical pixels per 96-dpi pixel (1 on a Mac, which works in points)
    scale: f32,
    /// pixels drawn per unit: 1 on Windows, the screen's pixels per point on a Mac
    px_scale: f32,
    place: Option<Place>,
    text: String,
    shown: bool,
}

impl Caption {
    pub(crate) fn create() -> Result<Caption> {
        let surface = Surface::create()?;
        let (scale, _, px_scale, _) = surface.scales();
        Ok(Caption { surface, scale, px_scale, place: None, text: String::new(), shown: false })
    }

    /// A dictation started: nothing to show until words arrive, and the place is still unknown.
    pub(crate) fn begin(&mut self, pill: Pt) {
        self.hide();
        self.place = Some(Place::Pill(pill));
    }

    /// The caret lookup answered; without a caret the caption stays above the pill.
    pub(crate) fn set_caret(&mut self, caret: Option<RECT>) {
        if let Some(r) = caret {
            self.place = Some(Place::Caret(r));
            if self.shown {
                self.draw();
            }
        }
    }

    pub(crate) fn show(&mut self, text: String) {
        if text.is_empty() {
            self.hide();
            return;
        }
        if text != self.text || !self.shown {
            self.text = text;
            self.draw();
        }
    }

    pub(crate) fn hide(&mut self) {
        if self.shown {
            self.surface.hide();
            self.shown = false;
        }
        self.text.clear();
    }

    fn draw(&mut self) {
        let Some(place) = self.place else { return };
        let font = (FONT * self.scale).round() as i32;
        let max_w = (MAX_W * self.scale).round() as i32;
        let pad = PAD * self.scale;
        let colour = crate::correction_ui::rgb(crate::correction_ui::TEXT);
        // the end of the text that fits, measured: fonts differ in width
        let mut chars = MAX_CHARS;
        let (spans, size) = loop {
            let spans = vec![(tail_of(&self.text, chars), colour)];
            let size = canvas::measure(&spans, font);
            if size.0 + 2 * pad as i32 <= max_w || chars <= 8 {
                break (spans, size);
            }
            chars -= 8;
        };
        let (w, h) = (size.0 + (2.0 * pad).ceil() as i32, ((HEIGHT * self.scale).round() as i32).max(size.1));
        let anchor = match place {
            Place::Caret(r) => Anchor::Caret(r),
            Place::Pill((x, y)) => Anchor::Area(RECT { left: x as i32, top: y as i32, right: x as i32 + 1, bottom: y as i32 + 1 }),
        };
        let work = caret::work_area(&anchor);
        let (x, y) = position(place, w, h, work);
        let mut c = Canvas::scaled(w, h, self.px_scale);
        c.capsule(0.0, 0.0, w as f32, h as f32, 0x202020, 0.9);
        c.text(pad, (h - size.1) as f32 / 2.0, &spans, font, size, 1.0);
        self.surface.push(x, y, w, h, &c.into_bgra());
        if !self.shown {
            self.surface.show();
            self.shown = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: RECT = RECT { left: 0, top: 0, right: 1920, bottom: 1040 };

    #[test]
    fn caption_sits_below_the_caret_line() {
        let caret = RECT { left: 500, top: 300, right: 501, bottom: 320 };
        assert_eq!(position(Place::Caret(caret), 300, 30, WORK), caret::place(&Anchor::Caret(caret), 300, 30, WORK));
        assert!(position(Place::Caret(caret), 300, 30, WORK).1 > caret.bottom);
    }

    #[test]
    fn without_a_caret_it_is_centred_above_the_pill() {
        assert_eq!(position(Place::Pill((960.0, 1000.0)), 300, 30, WORK), (810, 960));
    }

    #[test]
    fn above_the_pill_it_stays_on_screen() {
        assert_eq!(position(Place::Pill((10.0, 1000.0)), 300, 30, WORK).0, 0);
    }
}
