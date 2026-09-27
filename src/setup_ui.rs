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

enum Msg {
    Progress(u64),
    Unpacking,
    Done,
    Failed(String),
}

enum Stage {
    Downloading(u64),
    Unpacking,
    Failed(String),
}

struct SetupApp {
    models: PathBuf,
    total: u64,
    stage: Stage,
    rx: Receiver<Msg>,
    cancel: Arc<AtomicBool>,
    installed: Arc<AtomicBool>,
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
    let _ = tx.send(Msg::Unpacking);
    ctx.request_repaint();
    let dir = model_fetch::extract(&models.join(&parakeet.file), models)?;
    log::info!("model setup: installed {}", dir.display());
    Ok(())
}

/// tar.exe can't be stopped, so a close while it runs would orphan it mid-unpack. Once
/// installed, the close is the window's own and must go through.
fn refuse_close(stage: &Stage, installed: bool) -> bool {
    matches!(stage, Stage::Unpacking) && !installed
}

impl eframe::App for SetupApp {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Progress(n) => self.stage = Stage::Downloading(n),
                Msg::Unpacking => self.stage = Stage::Unpacking,
                Msg::Failed(e) => self.stage = Stage::Failed(e),
                Msg::Done => {
                    self.installed.store(true, Ordering::SeqCst);
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                    return;
                }
            }
        }
        // the title bar's X counts as Cancel; the .part stays for next launch
        if ctx.input(|i| i.viewport().close_requested()) {
            if refuse_close(&self.stage, self.installed.load(Ordering::SeqCst)) {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            } else {
                self.cancel.store(true, Ordering::SeqCst);
            }
        }
        let mut retry = false;
        egui::Frame::new().inner_margin(Margin::same(18)).show(ui, |ui| {
            ui.label(egui::RichText::new("Murmur needs its speech model (about 460 MB) before first use.").size(13.0).color(MUTED));
            ui.add_space(12.0);
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
                Stage::Unpacking => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(egui::RichText::new("Unpacking…").size(15.0).color(TEXT));
                    });
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
            }
        });
        if retry {
            self.start(&ctx);
        }
    }
}

/// First-run model download. Blocks until the model is installed or the user cancels or quits.
pub fn run(models: PathBuf) -> SetupOutcome {
    let installed = Arc::new(AtomicBool::new(false));
    let done = installed.clone();
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Murmur setup")
            .with_resizable(false)
            .with_inner_size([480.0, 170.0]),
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
            };
            app.start(&cc.egui_ctx);
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
        assert!(refuse_close(&Stage::Unpacking, false));
        assert!(!refuse_close(&Stage::Downloading(5), false));
        assert!(!refuse_close(&Stage::Failed("x".into()), false));
    }

    #[test]
    fn own_close_after_install_is_not_refused() {
        // Done arrives while the stage still reads Unpacking; the window's own Close must go through
        assert!(!refuse_close(&Stage::Unpacking, true));
    }
}
