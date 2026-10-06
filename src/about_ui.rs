//! The About window: version (checked against the latest GitHub release while it's open, with a
//! link that starts the same update as the tray's; a newer release also goes to the tray menu), what Murmur has learned, and the parts that
//! do the hearing, with their licenses.

use crate::correction_ui::{dark_theme, keycap, load_system_font, BG, BORDER, GREEN, MUTED, TEXT};
use crate::platform::{self, Window};
use eframe::egui::{self, CornerRadius, Frame, Key, Margin, Modifiers, RichText, Stroke, ViewportCommand};
use murmur_lib::update::Release;
use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;
use std::sync::mpsc::{channel, Receiver};

const WIDTH: f32 = 420.0;
const VERSION: &str = env!("CARGO_PKG_VERSION");
const REPO: &str = "https://github.com/jshowah-dev/murmur";

/// (component, license and holder)
const LICENSES: [(&str, &str); 5] = [
    ("Murmur", "MIT, Jeff Showah"),
    ("Parakeet TDT 0.6B v2", "CC BY 4.0, NVIDIA"),
    ("sherpa-onnx", "Apache 2.0, k2-fsa"),
    ("ONNX Runtime", "MIT, Microsoft"),
    ("Silero VAD", "MIT, Silero Team"),
];

#[derive(Debug, PartialEq)]
enum Update {
    Checking,
    UpToDate,
    Available(Release),
    Failed,
}

/// The model's display name, from its folder; an unknown model shows its folder name.
pub fn model_label(dir: &Path) -> String {
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if name.contains("parakeet-tdt-0.6b-v2") {
        "Parakeet TDT 0.6B v2 (NVIDIA)".into()
    } else {
        name
    }
}

fn learned(terms: usize) -> String {
    match terms {
        0 => "Hasn't learned any of your words yet".into(),
        n => format!("Knows {n} of your words"),
    }
}

/// Asks GitHub for the latest release once, off the UI thread, and wakes the window with the answer.
fn check(ctx: egui::Context) -> Receiver<Update> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let update = match murmur_lib::update::check() {
            Ok(Some(r)) => Update::Available(r),
            Ok(None) => Update::UpToDate,
            Err(e) => {
                log::info!("update check: {e}");
                Update::Failed
            }
        };
        let _ = tx.send(update);
        ctx.request_repaint();
    });
    rx
}

fn open_url(url: &str) {
    let _ = std::process::Command::new("explorer.exe").arg(url).spawn();
}

struct AboutApp {
    model: String,
    terms: usize,
    update: Update,
    rx: Option<Receiver<Update>>,
    /// The installed copy updates itself; any other copy links to the release page.
    installed: bool,
    /// The newer release the check found, for the tray to offer once the window is gone.
    found: Rc<Cell<Option<Release>>>,
    clicked: Rc<Cell<bool>>,
    hwnd: Window,
    frame: u32,
    was_focused: bool,
    /// Set once the window has been asked to close. It's drawn a few more times before it goes:
    /// those frames must show the same card, or it blinks out and back.
    closing: bool,
    height: f32,
}

impl eframe::App for AboutApp {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        self.draw(ui);
    }
}

impl AboutApp {
    /// The whole window, apart from eframe itself, so tests can drive it.
    fn draw(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.frame += 1;
        if self.frame == 1 {
            self.rx = Some(check(ctx.clone()));
        }
        if self.frame == 2 {
            // same as History: take focus once shown, then re-pin topmost
            platform::raise(self.hwnd);
            platform::keep_on_top(self.hwnd);
        }
        if let Some(u) = self.rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            if let Update::Available(r) = &u {
                self.found.set(Some(r.clone()));
            }
            self.update = u;
            self.rx = None;
        }
        // Esc or clicking another window closes it
        let focused = ctx.input(|i| i.focused);
        self.was_focused |= focused;
        if !self.closing && (ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) || (self.was_focused && !focused)) {
            self.closing = true;
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }

        let card = Frame::new()
            .fill(BG)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(CornerRadius::same(14))
            .inner_margin(Margin::symmetric(16, 14))
            .show(ui, |ui| {
                ui.set_width(WIDTH - 34.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Murmur").size(20.0).color(TEXT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        keycap(ui, "Esc");
                        ui.label(RichText::new("Close").size(12.0).color(MUTED));
                    });
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("v{VERSION} ·")).color(MUTED));
                    match &self.update {
                        Update::Checking => {
                            ui.label(RichText::new("checking for updates…").color(MUTED));
                        }
                        Update::UpToDate => {
                            ui.label(RichText::new("up to date").color(MUTED));
                        }
                        Update::Available(r) if self.installed => {
                            if ui.link(RichText::new(format!("update to v{} →", r.tag)).color(GREEN)).clicked() {
                                self.clicked.set(true);
                                self.closing = true;
                                ctx.send_viewport_cmd(ViewportCommand::Close);
                            }
                        }
                        Update::Available(r) => {
                            if ui.link(RichText::new(format!("v{} available →", r.tag)).color(GREEN)).clicked() {
                                open_url(murmur_lib::update::RELEASES_PAGE);
                            }
                        }
                        Update::Failed => {
                            ui.label(RichText::new("couldn't check for updates").color(MUTED));
                        }
                    }
                });
                ui.label(RichText::new(learned(self.terms)).color(TEXT));
                ui.add_space(10.0);

                ui.label(RichText::new(format!("Heard by {}", self.model)).color(TEXT));
                ui.label(RichText::new("Understood through sherpa-onnx, on ONNX Runtime").color(TEXT));
                ui.label(RichText::new("Silero VAD listens for when you start and stop speaking").color(TEXT));
                ui.add_space(10.0);

                if ui.link("github.com/jshowah-dev/murmur").clicked() {
                    open_url(REPO);
                }
                egui::CollapsingHeader::new(RichText::new("Licenses").color(MUTED)).show(ui, |ui| {
                    egui::Grid::new("licenses").num_columns(2).spacing([16.0, 2.0]).show(ui, |ui| {
                        for (what, license) in LICENSES {
                            ui.label(RichText::new(what).size(12.0).color(TEXT));
                            ui.label(RichText::new(license).size(12.0).color(MUTED));
                            ui.end_row();
                        }
                    });
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

/// Shows the About window, centred on screen. Blocks until closed, returning the newer release its
/// check found, if any, and whether its update link was clicked.
pub fn show(model: String, terms: usize, installed: bool) -> Option<(Release, bool)> {
    let found = Rc::new(Cell::new(None));
    let clicked = Rc::new(Cell::new(false));
    let (app_found, app_clicked) = (found.clone(), clicked.clone());
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Murmur — about")
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
        "murmur-about",
        opts,
        Box::new(move |cc| {
            dark_theme(&cc.egui_ctx);
            load_system_font(&cc.egui_ctx);
            let hwnd = platform::window_of(cc);
            Ok(Box::new(AboutApp { model, terms, update: Update::Checking, rx: None, installed, found: app_found, clicked: app_clicked, hwnd, frame: 0, was_focused: false, closing: false, height: 0.0 }))
        }),
    );
    if let Err(e) = r {
        log::error!("about window: {e}");
    }
    found.take().map(|r| (r, clicked.get()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_label_names_parakeet_and_falls_back_to_the_folder() {
        let p = Path::new("m").join("sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8");
        assert_eq!(model_label(&p), "Parakeet TDT 0.6B v2 (NVIDIA)");
        assert_eq!(model_label(&Path::new("m").join("whisper-small")), "whisper-small");
    }

    /// Past the frames that start the update check and take focus, so a test touches neither.
    fn app() -> AboutApp {
        AboutApp {
            model: "Parakeet".into(),
            terms: 0,
            update: Update::UpToDate,
            rx: None,
            installed: true,
            found: Rc::new(Cell::new(None)),
            clicked: Rc::new(Cell::new(false)),
            hwnd: Window::default(),
            frame: 5,
            was_focused: false,
            closing: false,
            height: 0.0,
        }
    }

    /// Runs one frame, returning the text drawn and whether the window asked to close.
    fn frame(ctx: &egui::Context, app: &mut AboutApp, events: Vec<egui::Event>) -> (Vec<String>, bool) {
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

    fn esc() -> egui::Event {
        egui::Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }
    }

    #[test]
    fn the_card_stays_drawn_while_it_closes() {
        let ctx = egui::Context::default();
        let mut app = app();
        frame(&ctx, &mut app, vec![]);
        let (text, close) = frame(&ctx, &mut app, vec![esc()]);
        assert!(close);
        assert!(text.iter().any(|t| t == "Murmur"), "closing frame drew {text:?}");
        // a redraw already queued, before the window goes
        let (text, close) = frame(&ctx, &mut app, vec![]);
        assert!(!close, "asks to close once");
        assert!(text.iter().any(|t| t == "Murmur"), "next frame drew {text:?}");
    }

    #[test]
    fn learned_counts_words() {
        assert_eq!(learned(0), "Hasn't learned any of your words yet");
        assert_eq!(learned(13), "Knows 13 of your words");
    }
}
