#![windows_subsystem = "windows"]

mod audio;
mod cleanup;
mod config;
mod correction;
mod dictionary;
mod history;
mod hotkey;
mod inject;
mod overlay;
mod phonetic;
mod pipeline;
mod stt;
mod tray;
mod vad;

use anyhow::Result;
use config::Config;
use crossbeam_channel::unbounded;
use dictionary::Dictionary;
use history::History;
use hotkey::HotkeyEvent;
use overlay::{Overlay, OverlayState};
use pipeline::{PipelineCmd, PipelineMsg};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tray::{Tray, TrayEvent};

const MAX_LOG_BYTES: u64 = 1024 * 1024;

fn init_logging() {
    let dir = config::config_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("murmur.log");
    let truncate = std::fs::metadata(&path).map(|m| m.len() > MAX_LOG_BYTES).unwrap_or(false);
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true);
    if truncate {
        opts.write(true).truncate(true);
    } else {
        opts.append(true);
    }
    if let Ok(f) = opts.open(&path) {
        let _ = simplelog::WriteLogger::init(log::LevelFilter::Info, simplelog::Config::default(), f);
    }
}

fn open_path(p: &std::path::Path) {
    let _ = std::process::Command::new("explorer.exe").arg(p).spawn();
}

fn main() -> Result<()> {
    init_logging();
    let mut startup_errors: Vec<String> = Vec::new();
    let cfg = match Config::load_or_create() {
        Ok(c) => c,
        Err(e) => {
            log::error!("{e:#}");
            startup_errors.push(format!("config: {e}"));
            Config::default()
        }
    };
    let dict = Arc::new(Mutex::new(match Dictionary::load_or_seed() {
        Ok(d) => d,
        Err(e) => {
            log::error!("{e:#}");
            startup_errors.push(format!("dictionary: {e}"));
            Dictionary::empty_unloaded()
        }
    }));
    let mut history = History::new(10);

    let (hk_tx, hk_rx) = unbounded::<HotkeyEvent>();
    let (cmd_tx, cmd_rx) = unbounded::<PipelineCmd>();
    let (msg_tx, msg_rx) = unbounded::<PipelineMsg>();
    let (audio_tx, audio_rx) = unbounded::<Vec<f32>>();

    hotkey::spawn(cfg.ptt_vk(), hk_tx);
    pipeline::spawn(cfg.clone(), dict.clone(), cmd_rx, msg_tx);

    let mut overlay = Overlay::create()?;
    let tray = Tray::create()?;
    let mut capture: Option<audio::Capture> = None;
    let mut paused = false;
    let mut listening = false;

    let encoder = cfg.model_dir_path().join("encoder.int8.onnx");
    if let Err(e) = std::fs::metadata(&encoder) {
        log::error!("model check failed for {}: {e} (LOCALAPPDATA={:?})", encoder.display(), std::env::var("LOCALAPPDATA"));
        tray.notify("Model missing", "run setup-model.cmd");
    }
    for msg in &startup_errors {
        tray.notify("Startup", msg);
    }

    loop {
        if !overlay.pump_once() {
            break;
        }
        while let Ok(chunk) = audio_rx.try_recv() {
            let _ = cmd_tx.send(PipelineCmd::Audio(chunk));
        }
        if let Some(ev) = tray.poll() {
            match ev {
                TrayEvent::TogglePause => {
                    paused = !paused;
                    tray.set_paused(paused);
                }
                TrayEvent::FixLast => {
                    correction::fix_last(&mut history, &dict, &tray);
                    while hk_rx.try_recv().is_ok() {}
                }
                TrayEvent::OpenDictionary => open_path(&config::config_dir().join("dictionary.toml")),
                TrayEvent::OpenConfigDir => open_path(&config::config_dir()),
                TrayEvent::Quit => break,
            }
        }
        while let Ok(ev) = hk_rx.try_recv() {
            match ev {
                HotkeyEvent::Down if !paused => {
                    let _ = cmd_tx.send(PipelineCmd::Start);
                    match audio::Capture::start(audio_tx.clone()) {
                        Ok(c) => {
                            capture = Some(c);
                        }
                        Err(e) => {
                            tray.notify("Microphone", &e.to_string());
                            let _ = cmd_tx.send(PipelineCmd::Abort);
                        }
                    }
                }
                HotkeyEvent::Press if !paused => {
                    listening = true;
                    overlay.set(OverlayState::Listening(0.0));
                }
                HotkeyEvent::Cancel => {
                    capture.take();
                    while audio_rx.try_recv().is_ok() {}
                    listening = false;
                    let _ = cmd_tx.send(PipelineCmd::Abort);
                }
                HotkeyEvent::Release => {
                    capture.take();
                    while let Ok(chunk) = audio_rx.try_recv() {
                        let _ = cmd_tx.send(PipelineCmd::Audio(chunk));
                    }
                    listening = false;
                    let _ = cmd_tx.send(PipelineCmd::Stop);
                }
                HotkeyEvent::FixLast => {
                    correction::fix_last(&mut history, &dict, &tray);
                    while hk_rx.try_recv().is_ok() {}
                }
                _ => {}
            }
        }
        while let Ok(m) = msg_rx.try_recv() {
            match m {
                PipelineMsg::Level(l) if listening => overlay.set(OverlayState::Listening(l)),
                PipelineMsg::Level(_) => {}
                PipelineMsg::Processing => overlay.set(OverlayState::Processing),
                PipelineMsg::Done(e) => {
                    overlay.set(OverlayState::Hidden);
                    if !e.cleaned.is_empty() {
                        log::debug!("raw: {}", e.raw);
                        history.push(e);
                    }
                }
                PipelineMsg::Error(s) => {
                    overlay.set(OverlayState::Hidden);
                    capture.take();
                    listening = false;
                    while let Ok(chunk) = audio_rx.try_recv() {
                        let _ = cmd_tx.send(PipelineCmd::Audio(chunk));
                    }
                    tray.notify("Murmur", &s);
                }
            }
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    let _ = cmd_tx.send(PipelineCmd::Shutdown);
    Ok(())
}
