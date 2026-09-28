use crate::config;
use crate::correction_ui::{load_system_font, MUTED, TEXT};
use crossbeam_channel::{Receiver, Sender};
use eframe::egui::{self, Margin, ViewportCommand};
use murmur_lib::model_fetch::{self, FetchError, Fetcher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const MB: u64 = 1024 * 1024;
const TICK: Duration = Duration::from_millis(250);

pub enum SetupOutcome {
    Installed,
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

/// Written once the "you're ready" screen has been dismissed, so it shows on the first launch only.
pub fn welcome_marker() -> PathBuf {
    config::config_dir().join("welcomed")
}

fn mark_welcomed() {
    let path = welcome_marker();
    if let Err(e) = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(&path, "")) {
        log::error!("write {}: {e}", path.display());
    }
}

const SIZE_DOWNLOAD: [f32; 2] = [480.0, 170.0];
const SIZE_READY: [f32; 2] = [480.0, 250.0];

enum Msg {
    Progress(u64),
    Unpacking(u64),
    Done,
    Failed(String),
}

enum Stage {
    Downloading(u64),
    Unpacking(u64),
    Failed(String),
    Ready,
}

struct SetupApp {
    models: PathBuf,
    total: u64,
    stage: Stage,
    rx: Receiver<Msg>,
    cancel: Arc<AtomicBool>,
    installed: Arc<AtomicBool>,
    then_ready: bool,
    key: String,
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
                    Msg::Failed(e.to_string())
                }
            };
            let _ = tx.send(msg);
            ctx.request_repaint();
        });
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
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Progress(n) => self.stage = Stage::Downloading(n),
                Msg::Unpacking(n) => self.stage = Stage::Unpacking(n),
                Msg::Failed(e) => self.stage = Stage::Failed(e),
                Msg::Done => {
                    self.installed.store(true, Ordering::SeqCst);
                    if self.then_ready {
                        self.stage = Stage::Ready;
                        ctx.send_viewport_cmd(ViewportCommand::InnerSize(SIZE_READY.into()));
                    } else {
                        ctx.send_viewport_cmd(ViewportCommand::Close);
                        return;
                    }
                }
            }
        }
        // the title bar's X counts as Cancel; the .part stays for next launch
        if ctx.input(|i| i.viewport().close_requested()) {
            if refuse_close(&self.stage, self.installed.load(Ordering::SeqCst)) {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            } else if matches!(self.stage, Stage::Ready) {
                mark_welcomed();
            } else {
                self.cancel.store(true, Ordering::SeqCst);
            }
        }
        let mut retry = false;
        egui::Frame::new().inner_margin(Margin::same(18)).show(ui, |ui| {
            if !matches!(self.stage, Stage::Ready) {
                ui.label(egui::RichText::new("Murmur needs its speech model (about 460 MB) before first use.").size(13.0).color(MUTED));
                ui.add_space(12.0);
            }
            match &self.stage {
                Stage::Downloading(n) => {
                    ui.label(
                        egui::RichText::new(format!("Downloading speech model: {} / {} MB", n / MB, self.total / MB))
                            .size(15.0)
                            .color(TEXT),
                    );
                    ui.add_space(6.0);
                    ui.add(egui::ProgressBar::new(*n as f32 / self.total as f32));
                    ui.add_space(12.0);
                    if ui.button("Cancel").clicked() {
                        self.cancel.store(true, Ordering::SeqCst);
                        ctx.send_viewport_cmd(ViewportCommand::Close);
                    }
                }
                Stage::Unpacking(n) => {
                    // held under 100% until tar exits: the last files land after the size estimate
                    let frac = (*n as f32 / model_fetch::PARAKEET_UNPACKED as f32).min(0.99);
                    ui.label(
                        egui::RichText::new(format!("Unpacking speech model: {:.0}%", frac * 100.0))
                            .size(15.0)
                            .color(TEXT),
                    );
                    ui.add_space(6.0);
                    ui.add(egui::ProgressBar::new(frac));
                }
                Stage::Failed(e) => {
                    ui.label(egui::RichText::new(e).size(15.0).color(TEXT));
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        retry = ui.button("Retry").clicked();
                        if ui.button("Quit").clicked() {
                            ctx.send_viewport_cmd(ViewportCommand::Close);
                        }
                    });
                }
                Stage::Ready => {
                    ui.label(egui::RichText::new("You're ready").size(18.0).color(TEXT));
                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new(format!(
                            "Hold {}, speak, then release. Murmur pastes the text where you're typing.",
                            self.key
                        ))
                        .size(14.0)
                        .color(TEXT),
                    );
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(
                            "Murmur lives in the system tray (bottom-right; click ^ if you don't see it). \
                             Right-click its icon to pause, see history, or edit your dictionary.",
                        )
                        .size(13.0)
                        .color(MUTED),
                    );
                    ui.add_space(14.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        if ui.button("Got it").clicked() {
                            mark_welcomed();
                            ctx.send_viewport_cmd(ViewportCommand::Close);
                        }
                    });
                }
            }
        });
        if retry {
            self.start(&ctx);
        }
    }
}

/// First-run window: the model download and/or the "you're ready" screen, per `plan`. Blocks
/// until the model is installed (or already was) and the window is closed, or the user quits.
pub fn run(models: PathBuf, plan: Plan, key: String) -> SetupOutcome {
    let (download, then_ready) = match plan {
        Plan::Download { then_ready } => (true, then_ready),
        Plan::ReadyOnly => (false, true),
        Plan::Skip => return SetupOutcome::Installed,
    };
    let installed = Arc::new(AtomicBool::new(!download));
    let done = installed.clone();
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Murmur setup")
            .with_resizable(false)
            .with_inner_size(if download { SIZE_DOWNLOAD } else { SIZE_READY }),
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
                stage: if download { Stage::Downloading(0) } else { Stage::Ready },
                rx,
                cancel: Arc::new(AtomicBool::new(false)),
                installed: done,
                then_ready,
                key,
            };
            if download {
                app.start(&cc.egui_ctx);
            }
            Ok(Box::new(app))
        }),
    );
    if let Err(e) = r {
        log::error!("setup window: {e}");
    }
    if installed.load(Ordering::SeqCst) {
        SetupOutcome::Installed
    } else {
        SetupOutcome::Quit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_refused_only_while_unpacking() {
        assert!(refuse_close(&Stage::Unpacking(0), false));
        assert!(!refuse_close(&Stage::Downloading(5), false));
        assert!(!refuse_close(&Stage::Failed("x".into()), false));
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
    fn own_close_after_install_is_not_refused() {
        // Done arrives while the stage still reads Unpacking; the window's own Close must go through
        assert!(!refuse_close(&Stage::Unpacking(0), true));
    }
}
