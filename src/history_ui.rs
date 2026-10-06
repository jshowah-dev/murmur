use crate::correction_ui::{dark_theme, keycap, load_system_font, BG, BORDER, GREEN, MUTED, TEXT};
use crate::platform::{self, Window};
use eframe::egui::{
    self, text::LayoutJob, text::TextFormat, Color32, CornerRadius, FontId, Frame, Key, Margin, Modifiers, Stroke,
    ViewportCommand,
};
use std::time::{Duration, Instant};

const WIDTH: f32 = 540.0;
const MAX_LIST: f32 = 460.0;
const HOVER: Color32 = Color32::from_rgb(0x2A, 0x2A, 0x2A);

/// How long "Copied" shows on a row after a copy.
fn flash() -> Duration {
    crate::motion::scaled(crate::motion::duration::CONFIRM)
}

fn ago(secs: u64) -> String {
    match secs {
        0..5 => "just now".into(),
        5..60 => format!("{secs} s ago"),
        60..3600 => format!("{} min ago", secs / 60),
        _ => format!("{} h ago", secs / 3600),
    }
}

struct HistoryApp {
    items: Vec<(String, Instant)>,
    hwnd: Window,
    frame: u32,
    was_focused: bool,
    copied: Option<(usize, Instant)>,
    /// Set once the window has been asked to close. It's drawn a few more times before it goes:
    /// those frames must show the same card, or it blinks out and back.
    closing: bool,
    height: f32,
    /// the talk key's label, for the empty state
    key: String,
}

impl eframe::App for HistoryApp {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        self.draw(ui);
    }
}

impl HistoryApp {
    /// The whole window, apart from eframe itself, so tests can drive it.
    fn draw(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.frame += 1;
        if self.frame == 2 {
            // same as the fix dialog: take focus once shown, then re-pin topmost
            platform::raise(self.hwnd);
            platform::keep_on_top(self.hwnd);
        }
        // Esc or clicking another window closes it
        let focused = ctx.input(|i| i.focused);
        self.was_focused |= focused;
        if !self.closing && (ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) || (self.was_focused && !focused)) {
            self.closing = true;
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }

        let flashing = self.copied.filter(|(_, at)| at.elapsed() < flash());
        if let Some((_, at)) = flashing {
            // one wake-up to clear the flash; never a repaint loop (it starves the overlay and tray)
            ctx.request_repaint_after(flash().saturating_sub(at.elapsed()));
        }

        let card = Frame::new()
            .fill(BG)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(CornerRadius::same(14))
            .inner_margin(Margin::symmetric(10, 12))
            .show(ui, |ui| {
                ui.set_width(WIDTH - 22.0);
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    let (dot, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                    ui.painter().circle_filled(dot.center(), 4.0, GREEN);
                    let head = if self.items.is_empty() { "History" } else { "History · click to copy" };
                    ui.label(egui::RichText::new(head).size(12.0).color(MUTED));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(6.0);
                        keycap(ui, "Esc");
                        ui.label(egui::RichText::new("Close").size(12.0).color(MUTED));
                    });
                });
                ui.add_space(6.0);
                if self.items.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        ui.label(egui::RichText::new(format!("Nothing dictated yet. Hold {} and speak.", self.key)).size(15.0).color(TEXT));
                    });
                    ui.add_space(6.0);
                }
                egui::ScrollArea::vertical().max_height(MAX_LIST).auto_shrink([false, true]).show(ui, |ui| {
                    for (i, (text, at)) in self.items.iter().enumerate() {
                        let id = ui.id().with(("row", i));
                        let hovered = ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
                        let row = Frame::new()
                            .fill(if hovered { HOVER } else { Color32::TRANSPARENT })
                            .corner_radius(CornerRadius::same(8))
                            .inner_margin(Margin::symmetric(8, 6))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(ago(at.elapsed().as_secs())).size(11.0).color(MUTED));
                                    if flashing.is_some_and(|(k, _)| k == i) {
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            ui.label(egui::RichText::new("Copied").size(11.0).color(GREEN));
                                        });
                                    }
                                });
                                let mut job = LayoutJob::single_section(
                                    text.clone(),
                                    TextFormat { font_id: FontId::proportional(15.0), color: TEXT, ..Default::default() },
                                );
                                job.wrap.max_width = ui.available_width();
                                job.wrap.max_rows = 3;
                                job.wrap.overflow_character = Some('…');
                                ui.add(egui::Label::new(job).selectable(false));
                            })
                            .response
                            .interact(egui::Sense::click());
                        if row.hovered() != hovered {
                            ctx.data_mut(|d| d.insert_temp(id, row.hovered()));
                            ctx.request_repaint();
                        }
                        if row.clicked() {
                            match crate::inject::set_clipboard_text(text) {
                                Ok(()) => self.copied = Some((i, Instant::now())),
                                Err(e) => log::error!("history copy: {e}"),
                            }
                            ctx.request_repaint();
                        }
                    }
                });
            });

        // Grow or shrink the window to the card so no invisible margin swallows clicks.
        let h = card.response.rect.max.y.ceil() + 1.0;
        if (h - self.height).abs() > 0.5 {
            self.height = h;
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(egui::vec2(WIDTH, h)));
        }
    }
}

/// Shows recent dictations, newest first, centred on screen. Blocks until closed.
pub fn show(items: Vec<(String, Instant)>, key: String) {
    #[cfg(windows)]
    window(items, key);
    #[cfg(target_os = "macos")]
    {
        let items = items.into_iter().map(|(t, at)| (t, at.elapsed().as_millis() as u64)).collect();
        let _: Option<bool> = crate::child::ask(FLAG, &Request { items, key });
    }
}

/// The argument that starts Murmur as the history window.
#[cfg(target_os = "macos")]
pub const FLAG: &str = "--history";

/// Each dictation with how many milliseconds ago it was heard.
#[cfg(target_os = "macos")]
#[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq)]
struct Request {
    items: Vec<(String, u64)>,
    key: String,
}

/// The history window's process.
#[cfg(target_os = "macos")]
pub fn run_child() -> anyhow::Result<()> {
    crate::child::serve(|r: Request| {
        let now = Instant::now();
        let items = r.items.into_iter().map(|(t, ms)| (t, now.checked_sub(Duration::from_millis(ms)).unwrap_or(now))).collect();
        window(items, r.key);
        None::<bool>
    })
}

fn window(items: Vec<(String, Instant)>, key: String) {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Murmur — history")
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_taskbar(false)
            .with_resizable(false)
            .with_inner_size([WIDTH, 200.0]),
        centered: true,
        ..Default::default()
    };
    let r = eframe::run_native(
        "murmur-history",
        opts,
        Box::new(move |cc| {
            dark_theme(&cc.egui_ctx);
            load_system_font(&cc.egui_ctx);
            let hwnd = platform::window_of(cc);
            Ok(Box::new(HistoryApp { items, hwnd, frame: 0, was_focused: false, copied: None, closing: false, height: 0.0, key }))
        }),
    );
    if let Err(e) = r {
        log::error!("history window: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn a_request_crosses_to_the_child() {
        let r = Request { items: vec![("first\nline \"quoted\"".into(), 1500), ("".into(), 0)], key: "Right Option".into() };
        assert_eq!(crate::child::round_trip(&r), Some(r));
    }

    #[test]
    fn copied_flash_lasts_the_confirm_token() {
        assert_eq!(flash(), crate::motion::scaled(crate::motion::duration::CONFIRM));
    }

    /// Past the frame that takes focus, so a test doesn't.
    fn app() -> HistoryApp {
        HistoryApp { items: vec![("hello there".into(), Instant::now())], hwnd: Window::default(), frame: 5, was_focused: false, copied: None, closing: false, height: 0.0, key: "Right Ctrl".into() }
    }

    /// Runs one frame, returning the text drawn and whether the window asked to close.
    fn frame(ctx: &egui::Context, app: &mut HistoryApp, events: Vec<egui::Event>) -> (Vec<String>, bool) {
        fn walk(shape: &egui::epaint::Shape, out: &mut Vec<String>) {
            match shape {
                egui::epaint::Shape::Text(t) => out.push(t.galley.text().to_string()),
                egui::epaint::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let input = egui::RawInput { events, ..Default::default() };
        let out = ctx.run_ui(input, |ui| app.draw(ui));
        let mut text = Vec::new();
        out.shapes.iter().for_each(|c| walk(&c.shape, &mut text));
        let close = out.viewport_output.values().any(|v| v.commands.contains(&ViewportCommand::Close));
        (text, close)
    }

    #[test]
    fn the_card_stays_drawn_while_it_closes() {
        let ctx = egui::Context::default();
        let mut app = app();
        frame(&ctx, &mut app, vec![]);
        let esc = egui::Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
        let (text, close) = frame(&ctx, &mut app, vec![esc]);
        assert!(close);
        assert!(text.iter().any(|t| t == "hello there"), "closing frame drew {text:?}");
        // a redraw already queued, before the window goes
        let (text, close) = frame(&ctx, &mut app, vec![]);
        assert!(!close, "asks to close once");
        assert!(text.iter().any(|t| t == "hello there"), "next frame drew {text:?}");
    }

    #[test]
    fn ago_buckets() {
        assert_eq!(ago(0), "just now");
        assert_eq!(ago(4), "just now");
        assert_eq!(ago(42), "42 s ago");
        assert_eq!(ago(60), "1 min ago");
        assert_eq!(ago(3599), "59 min ago");
        assert_eq!(ago(7200), "2 h ago");
    }

    #[test]
    fn nothing_dictated_says_how_to_start() {
        let ctx = egui::Context::default();
        let mut app = HistoryApp { items: Vec::new(), key: "Right Ctrl".into(), ..app() };
        let (text, _) = frame(&ctx, &mut app, vec![]);
        assert!(text.iter().any(|t| t == "Nothing dictated yet. Hold Right Ctrl and speak."), "{text:?}");
        assert!(!text.iter().any(|t| t.contains("click to copy")), "nothing to copy: {text:?}");
    }
}
