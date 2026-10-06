use crate::{caret, editor_kit, motion};
use eframe::egui::{
    self, text::CCursor, text::LayoutJob, text::TextFormat, Color32, CornerRadius, FontData, FontFamily, FontId, Frame, Key,
    Margin, Modifiers, Stroke, TextEdit, ViewportCommand,
};
use egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily};
use crate::platform::{self, Window};
use murmur_lib::dictionary::Dictionary;
use similar::{ChangeTag, TextDiff};
use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::time::{Duration, Instant};

const WIDTH: f32 = 540.0;
pub(crate) const BG: Color32 = Color32::from_rgb(0x1F, 0x1F, 0x1F);
pub(crate) const BORDER: Color32 = Color32::from_rgb(0x33, 0x33, 0x33);
pub(crate) const TEXT: Color32 = Color32::from_rgb(0xEE, 0xEE, 0xEE);
pub(crate) const MUTED: Color32 = Color32::from_rgb(0x99, 0x99, 0x99);
pub(crate) const GREEN: Color32 = Color32::from_rgb(0x60, 0xD0, 0x60);
pub(crate) const AMBER: Color32 = Color32::from_rgb(0xF5, 0xC5, 0x4A);
const AMBER_BG: Color32 = Color32::from_rgba_premultiplied(0x36, 0x2B, 0x10, 0x38);
pub(crate) const SPOKEN: Color32 = Color32::from_rgb(0xD9, 0x8C, 0x7A);

/// The dialog's opacity `elapsed` after it opened: the kit's enter motion, or fully there under reduced motion.
fn fade_in(elapsed: Duration, reduced: bool) -> f32 {
    if reduced {
        return 1.0;
    }
    let x = (elapsed.as_secs_f32() / motion::scaled(motion::duration::ENTER).as_secs_f32()).min(1.0);
    if x < 1.0 { editor_kit::ease(motion::easing::ENTER, x) } else { 1.0 }
}

/// A palette colour as 0xRRGGBB, for the GDI-drawn windows.
pub(crate) fn rgb(c: Color32) -> u32 {
    (c.r() as u32) << 16 | (c.g() as u32) << 8 | c.b() as u32
}

/// Byte ranges of `after`, each flagged true when it is a word the user changed relative to `before`.
pub fn changed_spans(before: &str, after: &str) -> Vec<(Range<usize>, bool)> {
    let diff = TextDiff::from_words(before, after);
    let mut spans: Vec<(Range<usize>, bool)> = Vec::new();
    let mut at = 0;
    for change in diff.iter_all_changes() {
        let changed = match change.tag() {
            ChangeTag::Delete => continue,
            ChangeTag::Equal => false,
            ChangeTag::Insert => !change.value().trim().is_empty(),
        };
        let end = at + change.value().len();
        match spans.last_mut() {
            Some((r, c)) if *c == changed => r.end = end,
            _ => spans.push((at..end, changed)),
        }
        at = end;
    }
    spans
}

fn heard_ago(at: Option<Instant>) -> Option<String> {
    let s = at?.elapsed().as_secs();
    Some(match s {
        0..5 => "just now".into(),
        5..60 => format!("heard {s} s ago"),
        _ => format!("heard {} min ago", s / 60),
    })
}

struct FixApp {
    original: String,
    text: String,
    heard: Option<String>,
    dict: Dictionary,
    learn: Vec<(String, String)>,
    learn_for: String,
    hwnd: Window,
    frame: u32,
    opened: Instant,
    reduced_motion: bool,
    height: f32,
    out: Rc<RefCell<Option<String>>>,
    card: Rc<RefCell<Option<(f32, f32)>>>,
}

impl FixApp {
    fn close(&self, ctx: &egui::Context, result: Option<String>) {
        *self.out.borrow_mut() = result;
        // where the card was, for the mote that carries the result to the caret
        *self.card.borrow_mut() = Some(caret::physical(|| {
            let r = platform::window_rect(self.hwnd).unwrap_or_default();
            ((r.left + r.right) as f32 / 2.0, (r.top + r.bottom) as f32 / 2.0)
        }));
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }

    fn refresh_learn(&mut self) {
        if self.learn_for == self.text {
            return;
        }
        self.learn = self
            .dict
            .preview_learn(&self.original, self.text.trim())
            .into_iter()
            .map(|t| (t.spoken.last().cloned().unwrap_or_default(), t.written))
            .collect();
        self.learn_for = self.text.clone();
    }
}

pub(crate) fn keycap(ui: &mut egui::Ui, label: &str) {
    Frame::new()
        .stroke(Stroke::new(1.0, Color32::from_rgb(0x48, 0x48, 0x48)))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(Margin::symmetric(5, 1))
        .show(ui, |ui| ui.label(egui::RichText::new(label).size(11.0).color(MUTED)));
}

impl eframe::App for FixApp {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.frame += 1;
        if self.frame == 2 {
            // eframe shows the window after the first frame; only then can it take focus.
            // bring_to_front ends not-topmost, so pin it again: a click elsewhere must not bury it.
            platform::raise(self.hwnd);
            platform::keep_on_top(self.hwnd);
        }
        let (submit, cancel) =
            ctx.input_mut(|i| (i.consume_key(Modifiers::CTRL, Key::Enter), i.consume_key(Modifiers::NONE, Key::Escape)));
        if submit {
            return self.close(&ctx, Some(self.text.clone()));
        }
        if cancel {
            return self.close(&ctx, None);
        }

        // Short fade-in, then repaint only on input: continuous repainting starves the
        // overlay and tray windows that share this thread.
        let opacity = fade_in(self.opened.elapsed(), self.reduced_motion);
        if opacity < 1.0 {
            ctx.request_repaint();
        }
        ui.set_opacity(opacity);
        ui.visuals_mut().text_cursor.stroke.color = AMBER;
        ui.visuals_mut().selection.bg_fill = Color32::from_rgb(0x1C, 0x6E, 0x73);

        self.refresh_learn();
        let card = Frame::new()
            .fill(BG)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(CornerRadius::same(14))
            .inner_margin(Margin::symmetric(16, 14))
            .show(ui, |ui| {
                ui.set_width(WIDTH - 34.0);
                ui.horizontal(|ui| {
                    let (dot, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                    ui.painter().circle_filled(dot.center(), 4.0, GREEN);
                    let head = match &self.heard {
                        Some(h) => format!("Fix last · {h}"),
                        None => "Fix last".into(),
                    };
                    ui.label(egui::RichText::new(head).size(12.0).color(MUTED));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        keycap(ui, "Esc");
                        ui.label(egui::RichText::new("Cancel").size(12.0).color(MUTED));
                        ui.add_space(8.0);
                        keycap(ui, "Ctrl+Enter");
                        ui.label(egui::RichText::new("Replace").size(12.0).color(MUTED));
                    });
                });
                ui.add_space(6.0);

                let original = self.original.clone();
                let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
                    let text = buf.as_str();
                    let mut job = LayoutJob::default();
                    let fmt = |changed: bool| TextFormat {
                        font_id: FontId::proportional(17.0),
                        color: if changed { AMBER } else { TEXT },
                        background: if changed { AMBER_BG } else { Color32::TRANSPARENT },
                        ..Default::default()
                    };
                    for (r, changed) in changed_spans(&original, text) {
                        job.append(&text[r], 0.0, fmt(changed));
                    }
                    if job.sections.is_empty() {
                        job.append("", 0.0, fmt(false));
                    }
                    job.wrap.max_width = wrap;
                    ui.ctx().fonts_mut(|f| f.layout_job(job))
                };
                let out = TextEdit::multiline(&mut self.text)
                    .frame(Frame::NONE)
                    .desired_width(f32::INFINITY)
                    .desired_rows(1)
                    .layouter(&mut layouter)
                    .show(ui);
                if self.frame == 1 {
                    let resp = &out.response.response;
                    resp.request_focus();
                    let mut state = out.state;
                    state.cursor.set_char_range(Some(egui::text::CCursorRange::one(CCursor::new(self.text.chars().count()))));
                    state.store(ui.ctx(), resp.id);
                }

                if !self.learn.is_empty() {
                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let (dot, _) = ui.allocate_exact_size(egui::vec2(6.0, 12.0), egui::Sense::hover());
                        ui.painter().circle_filled(dot.center(), 3.0, AMBER);
                        ui.label(egui::RichText::new("Will learn").size(12.0).color(MUTED));
                        for (i, (spoken, written)) in self.learn.iter().enumerate() {
                            if i > 0 {
                                ui.label(egui::RichText::new(",").size(12.0).color(MUTED));
                            }
                            ui.label(egui::RichText::new(spoken).size(12.0).color(SPOKEN));
                            ui.label(egui::RichText::new("→").size(12.0).color(MUTED));
                            ui.label(egui::RichText::new(written).size(12.0).color(AMBER));
                        }
                    });
                }
            });

        // Grow or shrink the window to the card so no invisible margin swallows clicks.
        let h = card.response.rect.max.y.ceil() + 1.0;
        if (h - self.height).abs() > 0.5 {
            self.height = h;
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(egui::vec2(WIDTH, h)));
        }
    }
}

/// The system's UI font files, best first.
#[cfg(windows)]
fn system_fonts() -> Vec<String> {
    let dir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
    ["SegUIVar.ttf", "segoeui.ttf"].iter().map(|name| format!(r"{dir}\Fonts\{name}")).collect()
}

#[cfg(target_os = "macos")]
fn system_fonts() -> Vec<String> {
    vec!["/System/Library/Fonts/SFNS.ttf".into()]
}

pub(crate) fn load_system_font(ctx: &egui::Context) {
    for path in system_fonts() {
        if let Ok(bytes) = std::fs::read(path) {
            ctx.add_font(FontInsert::new(
                "system",
                FontData::from_owned(bytes),
                vec![InsertFontFamily { family: FontFamily::Proportional, priority: FontPriority::Highest }],
            ));
            return;
        }
    }
}

/// Shows the fix-last dialog next to where `initial` was dictated. Blocks until the user
/// replaces (Some(edited)) or cancels (None); also returns the card's centre, physical pixels.
pub fn show(initial: &str, heard_at: Option<Instant>, dict: Dictionary, target: isize) -> (Option<String>, Option<(f32, f32)>) {
    // read the caret now, while the target app still has focus
    let anchor = caret::find(target);
    log::info!("fix-last anchor: {anchor:?}");
    let out = Rc::new(RefCell::new(None));
    let card = Rc::new(RefCell::new(None));
    let app_card = card.clone();
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Murmur — fix last")
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_taskbar(false)
            .with_resizable(false)
            .with_inner_size([WIDTH, 120.0]),
        centered: anchor.is_none(),
        ..Default::default()
    };
    let started = Instant::now();
    let app_out = out.clone();
    let initial = initial.to_string();
    let r = eframe::run_native(
        "murmur-fix",
        opts,
        Box::new(move |cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            load_system_font(&cc.egui_ctx);
            let hwnd = platform::window_of(cc);
            if let Some(anchor) = anchor {
                caret::physical(|| {
                    let wr = platform::window_rect(hwnd).unwrap_or_default();
                    let (x, y) = caret::place(&anchor, wr.right - wr.left, wr.bottom - wr.top, caret::work_area(&anchor));
                    platform::move_to(hwnd, x, y);
                });
            }
            log::info!("fix-last dialog created in {:?}", started.elapsed());
            Ok(Box::new(FixApp {
                original: initial.clone(),
                text: initial,
                heard: heard_ago(heard_at),
                dict,
                learn: Vec::new(),
                learn_for: String::new(),
                hwnd,
                frame: 0,
                opened: Instant::now(),
                reduced_motion: editor_kit::reduced_motion(),
                height: 0.0,
                out: app_out,
                card: app_card,
            }))
        }),
    );
    if let Err(e) = r {
        log::error!("fix-last dialog: {e}");
    }
    let result = out.borrow_mut().take();
    let at = card.borrow_mut().take();
    (result, at)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduced_motion_opens_fully_opaque() {
        assert_eq!(fade_in(Duration::ZERO, true), 1.0);
    }

    #[test]
    fn fade_in_runs_over_the_enter_token() {
        let d = motion::scaled(motion::duration::ENTER);
        assert!(fade_in(Duration::ZERO, false) < 0.01);
        let (a, b) = (fade_in(d / 4, false), fade_in(d / 2, false));
        assert!(0.0 < a && a < b && b < 1.0, "{a} {b}");
        assert_eq!(fade_in(d, false), 1.0);
        assert_eq!(fade_in(d * 3, false), 1.0);
    }

    fn marked(before: &str, after: &str) -> Vec<(String, bool)> {
        changed_spans(before, after).into_iter().map(|(r, c)| (after[r].to_string(), c)).collect()
    }

    #[test]
    fn unchanged_text_is_one_plain_span() {
        assert_eq!(marked("the dictionary look", "the dictionary look"), [("the dictionary look".into(), false)]);
    }

    #[test]
    fn replaced_word_is_marked() {
        assert_eq!(
            marked("the dictionary look", "the dictation look"),
            [("the ".into(), false), ("dictation".into(), true), (" look".into(), false)]
        );
    }

    #[test]
    fn spans_cover_the_whole_text() {
        let after = "brand new words, and more";
        let spans = changed_spans("old words", after);
        assert_eq!(spans.first().unwrap().0.start, 0);
        assert_eq!(spans.last().unwrap().0.end, after.len());
        assert!(spans.windows(2).all(|w| w[0].0.end == w[1].0.start));
    }

    #[test]
    fn empty_after_has_no_spans() {
        assert!(changed_spans("something", "").is_empty());
    }
}
