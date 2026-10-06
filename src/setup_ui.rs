//! First-run window: the model download as a Murmur card, then "Ready", then the card fades to
//! a dot that the mote carries to the pill (see `main.rs`, where the invitation is said).

use crate::config;
use crate::correction_ui::{load_system_font, BG, BORDER, MUTED, TEXT};
use crate::editor_kit::{ease, reduced_motion};
use crate::motion;
use crate::mote::Pt;
use crossbeam_channel::{Receiver, Sender};
use eframe::egui::{self, Color32, CornerRadius, Frame, Key, Margin, Modifiers, Pos2, RichText, Stroke, ViewportCommand};
use murmur_lib::model_fetch::{self, FetchError, Fetcher};
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use crate::platform::{self, Rect as RECT, Window};

const MB: u64 = 1024 * 1024;
const TICK: Duration = Duration::from_millis(250);
const WIDTH: f32 = 480.0;

pub enum SetupOutcome {
    /// `from`: where the card faded to a dot, physical pixels; None when it didn't (no card,
    /// reduced motion, or no invitation to carry).
    Installed { from: Option<Pt> },
    Quit,
}

/// What the setup window has to do on this launch.
#[derive(Debug, PartialEq)]
pub enum Plan {
    Download { then_ready: bool },
    ReadyOnly,
    Skip,
}

/// The auto-download only runs for the default model_dir; a missing custom one gets the tray
/// notice instead, and no "ready" screen that would be false.
pub fn plan(model_missing: bool, default_dir: bool, welcomed: bool) -> Plan {
    match (model_missing, default_dir, welcomed) {
        (true, true, _) => Plan::Download { then_ready: !welcomed },
        (true, false, _) | (false, _, true) => Plan::Skip,
        (false, _, false) => Plan::ReadyOnly,
    }
}

/// Whether the pill invites a first dictation: never welcomed, and there's a model to dictate with.
pub fn invites(welcomed: bool, model_missing: bool) -> bool {
    !welcomed && !model_missing
}

/// Written on the first dictation with words, so the invitation stops coming back.
pub fn welcome_marker() -> PathBuf {
    config::config_dir().join("welcomed")
}

pub fn mark_welcomed() {
    let path = welcome_marker();
    if let Err(e) = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(&path, "")) {
        log::error!("write {}: {e}", path.display());
    }
}

enum Msg {
    Progress(u64),
    Unpacking(u64),
    Done,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq)]
enum Stage {
    Downloading(u64),
    Unpacking(u64),
    /// what to do, from `FetchError::advice`
    Failed(String),
    /// installed: "Ready" for a beat, since then
    Ready(Instant),
    /// fading to a dot at the card's centre, since then
    Closing(Instant),
}

#[derive(Debug, PartialEq)]
enum Next {
    Hold,
    Fade,
    Close,
}

/// What the Ready beat does `el` after it began.
fn after_ready(el: Duration, reduced: bool) -> Next {
    if el < motion::scaled(motion::duration::LOCATE) {
        Next::Hold
    } else if reduced {
        Next::Close
    } else {
        Next::Fade
    }
}

/// The card's opacity `el` into the fade; None once only the dot is left.
fn fade_alpha(el: Duration) -> Option<f32> {
    let t = el.as_secs_f32() / motion::scaled(motion::duration::EXIT).as_secs_f32();
    (t < 1.0).then(|| 1.0 - ease(motion::easing::EXIT, t))
}

/// The mote's core and halo radii in points: the mote draws them in physical pixels, unscaled.
fn dot_radii(pixels_per_point: f32) -> (f32, f32) {
    (2.5 / pixels_per_point, 12.0 / pixels_per_point)
}

/// Where the mote starts: the card's centre, physical pixels. Not from a minimized window,
/// whose rect is parked far off-screen.
fn hand_back(rect: Option<RECT>, minimized: bool) -> Option<Pt> {
    let r = rect.filter(|_| !minimized)?;
    Some(((r.left + r.right) as f32 / 2.0, (r.top + r.bottom) as f32 / 2.0))
}

/// The mote's dot, drawn where the card was: a pale core in a soft green halo (`mote::render`).
fn dot(painter: &egui::Painter, c: Pos2, alpha: f32) {
    use egui::epaint::{Mesh, Vertex, WHITE_UV};
    // the halo falls off as 0.5 * (1 - d/r)^2, as the mote's canvas draws it: sampled on rings,
    // linear between them
    let (core, r) = dot_radii(painter.ctx().pixels_per_point());
    const SEG: usize = 32;
    const RINGS: [f32; 5] = [0.0, 0.25, 0.5, 0.75, 1.0];
    let green = |f: f32| Color32::from_rgba_unmultiplied(0x60, 0xD0, 0x60, (0.5 * alpha * (1.0 - f).powi(2) * 255.0) as u8);
    let mut mesh = Mesh::default();
    for &f in &RINGS {
        for s in 0..SEG {
            let a = s as f32 / SEG as f32 * std::f32::consts::TAU;
            mesh.vertices.push(Vertex { pos: c + egui::vec2(a.cos(), a.sin()) * r * f, uv: WHITE_UV, color: green(f) });
        }
    }
    for ring in 0..RINGS.len() - 1 {
        for s in 0..SEG {
            let (i0, i1) = ((ring * SEG + s) as u32, (ring * SEG + (s + 1) % SEG) as u32);
            let (o0, o1) = (i0 + SEG as u32, i1 + SEG as u32);
            mesh.add_triangle(i0, o0, o1);
            mesh.add_triangle(i0, o1, i1);
        }
    }
    painter.add(mesh);
    painter.circle_filled(c, core, Color32::from_rgba_unmultiplied(0xE8, 0xFF, 0xE8, (alpha * 255.0) as u8));
}

struct SetupApp {
    models: PathBuf,
    total: u64,
    stage: Stage,
    rx: Receiver<Msg>,
    cancel: Arc<AtomicBool>,
    installed: Arc<AtomicBool>,
    then_ready: bool,
    hwnd: Window,
    reduced: bool,
    /// Close was sent; later frames keep drawing the same picture until the window goes
    closing: bool,
    /// a failure just arrived: Retry takes keyboard focus, so Enter retries
    focus_retry: bool,
    height: f32,
    from: Rc<Cell<Option<Pt>>>,
}

impl SetupApp {
    /// Runs one download + unpack attempt on a worker thread; Retry calls this again and resumes.
    fn start(&mut self, ctx: &egui::Context) {
        let (tx, rx) = crossbeam_channel::unbounded();
        self.rx = rx;
        self.stage = Stage::Downloading(0);
        let models = self.models.clone();
        let cancel = self.cancel.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let msg = match install(&models, &cancel, &tx, &ctx) {
                Ok(()) => Msg::Done,
                Err(FetchError::Cancelled) => return,
                Err(e) => {
                    log::error!("model setup: {e}");
                    Msg::Failed(e.advice())
                }
            };
            let _ = tx.send(msg);
            ctx.request_repaint();
        });
    }

    fn close(&mut self, ctx: &egui::Context) {
        if !self.closing {
            self.closing = true;
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }

    /// The window's centre, physical pixels (the card fills it), if it's on screen.
    fn card_centre(&self) -> Option<Pt> {
        crate::caret::physical(|| hand_back(platform::window_rect(self.hwnd), platform::is_minimized(self.hwnd)))
    }

    /// The whole window, apart from eframe itself, so tests can drive it.
    fn draw(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let now = Instant::now();
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Progress(n) => self.stage = Stage::Downloading(n),
                Msg::Unpacking(n) => self.stage = Stage::Unpacking(n),
                Msg::Failed(e) => {
                    self.stage = Stage::Failed(e);
                    self.focus_retry = true;
                }
                Msg::Done => {
                    self.installed.store(true, Ordering::SeqCst);
                    if self.then_ready {
                        self.stage = Stage::Ready(now);
                    } else {
                        self.close(&ctx);
                    }
                }
            }
        }
        if let Stage::Ready(since) = self.stage {
            let el = now.saturating_duration_since(since);
            match after_ready(el, self.reduced) {
                Next::Hold => ctx.request_repaint_after(motion::scaled(motion::duration::LOCATE).saturating_sub(el)),
                Next::Fade => self.stage = Stage::Closing(now),
                Next::Close => self.close(&ctx),
            }
        }
        // Alt+F4 counts as Cancel; the .part stays for next launch
        if ctx.input(|i| i.viewport().close_requested()) {
            if refuse_close(&self.stage, self.installed.load(Ordering::SeqCst)) {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            } else if !self.installed.load(Ordering::SeqCst) {
                self.cancel.store(true, Ordering::SeqCst);
            }
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            match self.stage {
                Stage::Downloading(_) => {
                    self.cancel.store(true, Ordering::SeqCst);
                    self.close(&ctx);
                }
                Stage::Failed(_) => self.close(&ctx),
                _ => {}
            }
        }

        let alpha = match self.stage {
            Stage::Closing(start) => fade_alpha(now.saturating_duration_since(start)),
            _ => Some(1.0),
        };
        let mut retry = false;
        let card = ui
            .scope(|ui| {
                ui.set_opacity(alpha.unwrap_or(0.0));
                Frame::new()
                    .fill(BG)
                    .stroke(Stroke::new(1.0, BORDER))
                    .corner_radius(CornerRadius::same(14))
                    .inner_margin(Margin::symmetric(16, 14))
                    .show(ui, |ui| {
                        ui.set_width(WIDTH - 34.0);
                        self.body(ui, &ctx, &mut retry);
                    })
                    .response
                    .rect
            })
            .inner;

        if let Stage::Closing(_) = self.stage {
            dot(ui.painter(), card.center(), 1.0 - alpha.unwrap_or(0.0));
            match alpha {
                Some(_) => ctx.request_repaint(),
                None => {
                    if !self.closing {
                        self.from.set(self.card_centre());
                    }
                    self.close(&ctx);
                }
            }
        } else {
            // Grow or shrink the window to the card so no invisible margin swallows clicks.
            let h = card.max.y.ceil() + 1.0;
            if (h - self.height).abs() > 0.5 && !matches!(self.stage, Stage::Ready(_)) {
                self.height = h;
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(egui::vec2(WIDTH, h)));
            }
        }
        if retry {
            self.start(&ctx);
        }
    }

    /// The card's contents. Each stage but Failed keeps the same rows, so the card holds its height from download to Ready.
    fn body(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, retry: &mut bool) {
        let installed = matches!(self.stage, Stage::Ready(_) | Stage::Closing(_));
        let intro = if installed {
            "Speech model installed.".to_string()
        } else {
            format!("Murmur needs its speech model (about {} MB) before first use.", self.total / MB)
        };
        ui.label(RichText::new(intro).size(13.0).color(MUTED));
        ui.add_space(12.0);
        match self.stage.clone() {
            Stage::Downloading(n) => {
                ui.label(RichText::new(format!("Downloading speech model: {} / {} MB", n / MB, self.total / MB)).size(15.0).color(TEXT));
                ui.add_space(6.0);
                ui.add(egui::ProgressBar::new(n as f32 / self.total as f32));
                ui.add_space(12.0);
                if ui.button("Cancel").clicked() {
                    self.cancel.store(true, Ordering::SeqCst);
                    self.close(ctx);
                }
            }
            Stage::Unpacking(n) => {
                // held under 100% until tar exits: the last files land after the size estimate
                let frac = (n as f32 / model_fetch::PARAKEET_UNPACKED as f32).min(0.99);
                ui.label(RichText::new(format!("Unpacking speech model: {:.0}%", frac * 100.0)).size(15.0).color(TEXT));
                ui.add_space(6.0);
                ui.add(egui::ProgressBar::new(frac));
                ui.add_space(12.0);
                // tar can't be stopped: no Cancel, but its room stays
                ui.add_visible(false, egui::Button::new("Cancel"));
            }
            Stage::Failed(e) => {
                ui.label(RichText::new(e).size(14.0).color(TEXT));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let r = ui.button("Retry");
                    if std::mem::take(&mut self.focus_retry) {
                        r.request_focus();
                    }
                    *retry = r.clicked();
                    if ui.button("Quit").clicked() {
                        self.close(ctx);
                    }
                });
            }
            Stage::Ready(_) | Stage::Closing(_) => {
                ui.label(RichText::new("Ready").size(15.0).color(TEXT));
                ui.add_space(6.0);
                ui.add(egui::ProgressBar::new(1.0));
                ui.add_space(12.0);
                ui.add_visible(false, egui::Button::new("Cancel"));
            }
        }
    }
}

fn install(models: &Path, cancel: &AtomicBool, tx: &Sender<Msg>, ctx: &egui::Context) -> Result<(), FetchError> {
    let fetcher = Fetcher::standard();
    let (vad, parakeet) = (model_fetch::vad(), model_fetch::parakeet());
    let mut last = Instant::now() - TICK;
    let mut report = |done: u64| {
        if last.elapsed() >= TICK {
            last = Instant::now();
            let _ = tx.send(Msg::Progress(done));
            ctx.request_repaint();
        }
    };
    log::info!("model setup: downloading into {}", models.display());
    fetcher.download(&vad, models, &mut |n| report(n), cancel)?;
    fetcher.download(&parakeet, models, &mut |n| report(vad.size + n), cancel)?;
    let _ = tx.send(Msg::Unpacking(0));
    ctx.request_repaint();
    let dir = model_fetch::extract(&models.join(&parakeet.file), models, &mut |n| {
        let _ = tx.send(Msg::Unpacking(n));
        ctx.request_repaint();
    })?;
    log::info!("model setup: installed {}", dir.display());
    Ok(())
}

/// tar.exe can't be stopped, so a close while it runs would orphan it mid-unpack. Once
/// installed, the close is the window's own and must go through.
fn refuse_close(stage: &Stage, installed: bool) -> bool {
    matches!(stage, Stage::Unpacking(_)) && !installed
}

impl eframe::App for SetupApp {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        self.draw(ui);
    }
}

/// First-run window, per `plan`: the model download, then (first run only) "Ready" and the fade
/// to a dot. Blocks until the model is installed and the window is closed, or the user quits.
/// `ReadyOnly` and `Skip` show no window.
pub fn run(models: PathBuf, plan: Plan) -> SetupOutcome {
    let then_ready = match plan {
        Plan::Download { then_ready } => then_ready,
        Plan::ReadyOnly | Plan::Skip => return SetupOutcome::Installed { from: None },
    };
    let installed = Arc::new(AtomicBool::new(false));
    let done = installed.clone();
    let from = Rc::new(Cell::new(None));
    let app_from = from.clone();
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Murmur setup")
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_inner_size([WIDTH, 170.0]),
        centered: true,
        ..Default::default()
    };
    let r = eframe::run_native(
        "murmur-setup",
        opts,
        Box::new(move |cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            load_system_font(&cc.egui_ctx);
            let (_, rx) = crossbeam_channel::unbounded();
            let mut app = SetupApp {
                models,
                total: model_fetch::vad().size + model_fetch::parakeet().size,
                stage: Stage::Downloading(0),
                rx,
                cancel: Arc::new(AtomicBool::new(false)),
                installed: done,
                then_ready,
                hwnd: platform::window_of(cc),
                reduced: reduced_motion(),
                closing: false,
                focus_retry: false,
                height: 0.0,
                from: app_from,
            };
            app.start(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    );
    if let Err(e) = r {
        log::error!("setup window: {e}");
    }
    if installed.load(Ordering::SeqCst) {
        SetupOutcome::Installed { from: from.get() }
    } else {
        SetupOutcome::Quit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(stage: Stage) -> SetupApp {
        let (_, rx) = crossbeam_channel::unbounded();
        SetupApp {
            models: PathBuf::new(),
            total: 483 * MB,
            stage,
            rx,
            cancel: Arc::new(AtomicBool::new(false)),
            installed: Arc::new(AtomicBool::new(false)),
            then_ready: true,
            hwnd: Window::default(),
            reduced: false,
            closing: false,
            focus_retry: false,
            height: 0.0,
            from: Rc::new(Cell::new(None)),
        }
    }

    /// Runs one frame, returning the text drawn and whether the window asked to close.
    fn frame(ctx: &egui::Context, app: &mut SetupApp, events: Vec<egui::Event>) -> (Vec<String>, bool) {
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
    fn close_refused_only_while_unpacking() {
        assert!(refuse_close(&Stage::Unpacking(0), false));
        assert!(!refuse_close(&Stage::Downloading(5), false));
        assert!(!refuse_close(&Stage::Failed("x".into()), false));
        assert!(!refuse_close(&Stage::Closing(Instant::now()), true));
    }

    #[test]
    fn own_close_after_install_is_not_refused() {
        // Done arrives while the stage still reads Unpacking; the window's own Close must go through
        assert!(!refuse_close(&Stage::Unpacking(0), true));
    }

    #[test]
    fn plan_covers_first_run_cases() {
        // (model_missing, default_dir, welcomed)
        assert_eq!(plan(true, true, false), Plan::Download { then_ready: true });
        assert_eq!(plan(true, true, true), Plan::Download { then_ready: false });
        assert_eq!(plan(false, true, false), Plan::ReadyOnly);
        assert_eq!(plan(false, false, false), Plan::ReadyOnly);
        assert_eq!(plan(false, true, true), Plan::Skip);
        // a custom model_dir that's missing gets the tray notice, and no Ready claim
        assert_eq!(plan(true, false, false), Plan::Skip);
        assert_eq!(plan(true, false, true), Plan::Skip);
    }

    #[test]
    fn invites_only_with_a_model_and_no_marker() {
        assert!(invites(false, false));
        assert!(!invites(true, false), "welcomed before");
        assert!(!invites(false, true), "a missing custom model_dir: nothing to dictate with");
    }

    #[test]
    fn ready_holds_for_a_beat_then_fades_or_closes() {
        let beat = motion::scaled(motion::duration::LOCATE);
        assert_eq!(after_ready(beat / 2, false), Next::Hold);
        assert_eq!(after_ready(beat, false), Next::Fade);
        assert_eq!(after_ready(beat, true), Next::Close, "reduced motion: no fade");
    }

    #[test]
    fn the_fade_runs_down_to_nothing() {
        let exit = motion::scaled(motion::duration::EXIT);
        assert_eq!(fade_alpha(Duration::ZERO), Some(1.0));
        let mid = fade_alpha(exit / 2).unwrap();
        assert!(mid > 0.0 && mid < 1.0, "{mid}");
        assert_eq!(fade_alpha(exit), None, "only the dot is left");
    }

    #[test]
    fn a_failed_download_says_what_to_do_and_esc_quits() {
        let ctx = egui::Context::default();
        let mut a = app(Stage::Failed(FetchError::Interrupted("os error 10054".into()).advice()));
        let (text, close) = frame(&ctx, &mut a, vec![]);
        assert!(text.iter().any(|t| t.starts_with("The download stopped partway")), "{text:?}");
        assert!(text.iter().any(|t| t == "Retry") && text.iter().any(|t| t == "Quit"), "{text:?}");
        assert!(!text.iter().any(|t| t.contains("10054")), "no raw detail on the card");
        assert!(!close);
        let (_, close) = frame(&ctx, &mut a, vec![esc()]);
        assert!(close);
        assert!(!a.cancel.load(Ordering::SeqCst), "nothing to cancel after a failure");
    }

    #[test]
    fn esc_cancels_the_download() {
        let ctx = egui::Context::default();
        let mut a = app(Stage::Downloading(10 * MB));
        let (text, _) = frame(&ctx, &mut a, vec![]);
        assert!(text.iter().any(|t| t == "Downloading speech model: 10 / 483 MB"), "{text:?}");
        assert!(text.iter().any(|t| t == "Murmur needs its speech model (about 483 MB) before first use."), "{text:?}");
        let (_, close) = frame(&ctx, &mut a, vec![esc()]);
        assert!(close && a.cancel.load(Ordering::SeqCst));
    }

    #[test]
    fn esc_does_nothing_while_unpacking() {
        let ctx = egui::Context::default();
        let mut a = app(Stage::Unpacking(0));
        let (_, close) = frame(&ctx, &mut a, vec![esc()]);
        assert!(!close && !a.cancel.load(Ordering::SeqCst));
    }

    #[test]
    fn ready_runs_without_input() {
        // past the beat, with no input at all: it moves on to the fade by itself
        let ctx = egui::Context::default();
        let mut a = app(Stage::Ready(Instant::now() - motion::scaled(motion::duration::LOCATE) * 2));
        a.installed.store(true, Ordering::SeqCst);
        let (text, close) = frame(&ctx, &mut a, vec![]);
        assert!(matches!(a.stage, Stage::Closing(_)), "fading");
        assert!(!close, "the fade comes first");
        assert!(text.iter().any(|t| t == "Ready"), "same card on the frame the fade starts: {text:?}");
    }

    #[test]
    fn the_faded_card_closes_once() {
        let ctx = egui::Context::default();
        let mut a = app(Stage::Closing(Instant::now() - motion::scaled(motion::duration::EXIT) * 2));
        a.installed.store(true, Ordering::SeqCst);
        let (_, close) = frame(&ctx, &mut a, vec![]);
        assert!(close);
        let (_, close) = frame(&ctx, &mut a, vec![]);
        assert!(!close, "asks to close once");
    }

    #[test]
    fn the_card_hands_back_its_centre_only_when_on_screen() {
        let r = RECT { left: 100, top: 200, right: 580, bottom: 360 };
        assert_eq!(hand_back(Some(r), false), Some((340.0, 280.0)));
        assert_eq!(hand_back(Some(r), true), None, "minimized: its rect is parked off-screen");
        assert_eq!(hand_back(None, false), None, "no window rect");
    }

    #[test]
    fn the_dot_is_the_motes_size_in_physical_pixels() {
        assert_eq!(dot_radii(1.0), (2.5, 12.0));
        let (core, halo) = dot_radii(1.5);
        assert!((core - 2.5 / 1.5).abs() < 1e-6 && (halo - 8.0).abs() < 1e-6, "{core} {halo}");
    }

    #[test]
    fn reduced_motion_closes_after_the_beat_with_no_dot() {
        let ctx = egui::Context::default();
        let mut a = app(Stage::Ready(Instant::now() - motion::scaled(motion::duration::LOCATE) * 2));
        a.reduced = true;
        a.installed.store(true, Ordering::SeqCst);
        let (_, close) = frame(&ctx, &mut a, vec![]);
        assert!(close);
        assert!(a.from.get().is_none(), "no flight: the invitation appears above the pill");
    }
}
