//! The About window: version (checked against the latest GitHub release while it's open), what
//! Murmur has learned, and the parts that do the hearing, with their licenses.

use crate::correction_ui::{hwnd_of, keycap, load_system_font, BG, BORDER, GREEN, MUTED, TEXT};
use eframe::egui::{self, CornerRadius, Frame, Key, Margin, Modifiers, RichText, Stroke, ViewportCommand};
use std::path::Path;
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE};

const WIDTH: f32 = 420.0;
const VERSION: &str = env!("CARGO_PKG_VERSION");
const REPO: &str = "https://github.com/jshowah-dev/murmur";
const LATEST_API: &str = "https://api.github.com/repos/jshowah-dev/murmur/releases/latest";

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
    Available(String),
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

/// `tag_name` from a GitHub release as JSON, without its leading "v".
fn latest_tag(json: &str) -> Option<String> {
    let rest = &json[json.find("\"tag_name\"")? + "\"tag_name\"".len()..];
    let rest = &rest[rest.find('"')? + 1..];
    Some(rest[..rest.find('"')?].trim_start_matches('v').to_string())
}

fn is_newer(latest: &str, current: &str) -> bool {
    let parts = |v: &str| v.split('.').map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    parts(latest) > parts(current)
}

/// Asks GitHub for the latest release once, off the UI thread, and wakes the window with the answer.
fn check(ctx: egui::Context) -> Receiver<Update> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(8))).build().new_agent();
        let body = agent.get(LATEST_API).header("User-Agent", "murmur").call().and_then(|mut r| r.body_mut().read_to_string());
        let update = match body.map(|b| latest_tag(&b)) {
            Ok(Some(tag)) if is_newer(&tag, VERSION) => Update::Available(tag),
            Ok(Some(_)) => Update::UpToDate,
            Ok(None) => Update::Failed,
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
    hwnd: HWND,
    frame: u32,
    was_focused: bool,
    height: f32,
}

impl eframe::App for AboutApp {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.frame += 1;
        if self.frame == 1 {
            self.rx = Some(check(ctx.clone()));
        }
        if self.frame == 2 {
            // same as History: take focus once shown, then re-pin topmost
            unsafe {
                crate::correction::bring_to_front(self.hwnd);
                let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
        }
        if let Some(u) = self.rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.update = u;
            self.rx = None;
        }
        // Esc or clicking another window closes it
        let focused = ctx.input(|i| i.focused);
        self.was_focused |= focused;
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) || (self.was_focused && !focused) {
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
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
                        Update::Available(v) => {
                            if ui.link(RichText::new(format!("v{v} available →")).color(GREEN)).clicked() {
                                open_url(&format!("{REPO}/releases/latest"));
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

/// Shows the About window, centred on screen. Blocks until closed.
pub fn show(model: String, terms: usize) {
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
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            load_system_font(&cc.egui_ctx);
            let hwnd = hwnd_of(cc).unwrap_or_default();
            Ok(Box::new(AboutApp { model, terms, update: Update::Checking, rx: None, hwnd, frame: 0, was_focused: false, height: 0.0 }))
        }),
    );
    if let Err(e) = r {
        log::error!("about window: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_tag_reads_the_release_json() {
        let json = r#"{"url":"x","tag_name": "v0.4.3","name":"Murmur v0.4.3"}"#;
        assert_eq!(latest_tag(json).as_deref(), Some("0.4.3"));
        assert_eq!(latest_tag(r#"{"message":"Not Found"}"#), None);
    }

    #[test]
    fn versions_compare_by_number() {
        assert!(is_newer("0.4.10", "0.4.3"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("0.4.3", "0.4.3"));
        assert!(!is_newer("0.4.2", "0.4.3"));
    }

    #[test]
    fn model_label_names_parakeet_and_falls_back_to_the_folder() {
        let p = Path::new(r"C:\m\sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8");
        assert_eq!(model_label(p), "Parakeet TDT 0.6B v2 (NVIDIA)");
        assert_eq!(model_label(Path::new(r"C:\m\whisper-small")), "whisper-small");
    }

    #[test]
    fn learned_counts_words() {
        assert_eq!(learned(0), "Hasn't learned any of your words yet");
        assert_eq!(learned(13), "Knows 13 of your words");
    }
}
