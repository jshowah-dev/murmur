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

fn init_logging() {
    let dir = config::config_dir();
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(f) = std::fs::File::create(dir.join("murmur.log")) {
        let _ = simplelog::WriteLogger::init(log::LevelFilter::Info, simplelog::Config::default(), f);
    }
}

fn open_path(p: &std::path::Path) {
    let _ = std::process::Command::new("explorer.exe").arg(p).spawn();
}

fn main() -> Result<()> {
    init_logging();
    let cfg = Config::load_or_create()?;
    let dict = Arc::new(Mutex::new(Dictionary::load_or_seed()?));
    let mut history = History::new(10);

    let (hk_tx, hk_rx) = unbounded::<HotkeyEvent>();
    let (cmd_tx, cmd_rx) = unbounded::<PipelineCmd>();
    let (msg_tx, msg_rx) = unbounded::<PipelineMsg>();
    let (audio_tx, audio_rx) = unbounded::<Vec<f32>>();

    hotkey::spawn(cfg.ptt_vk(), hk_tx);
    pipeline::spawn(cfg.clone(), dict.clone(), cmd_rx, msg_tx);
    // forward audio chunks to the pipeline
    {
        let cmd_tx = cmd_tx.clone();
        std::thread::spawn(move || {
            for chunk in audio_rx {
                let _ = cmd_tx.send(PipelineCmd::Audio(chunk));
            }
        });
    }

    let mut overlay = Overlay::create()?;
    let tray = Tray::create()?;
    let mut capture: Option<audio::Capture> = None;
    let mut paused = false;

    if !cfg.model_dir_path().join("encoder.int8.onnx").exists() {
        tray.notify("Model missing", "run setup-model.cmd");
    }

    loop {
        if !overlay.pump_once() {
            break;
        }
        if let Some(ev) = tray.poll() {
            match ev {
                TrayEvent::TogglePause => {
                    paused = !paused;
                    tray.set_paused(paused);
                }
                TrayEvent::FixLast => correction::fix_last(&mut history, &dict, &tray),
                TrayEvent::OpenDictionary => open_path(&config::config_dir().join("dictionary.toml")),
                TrayEvent::OpenConfigDir => open_path(&config::config_dir()),
                TrayEvent::Quit => break,
            }
        }
        while let Ok(ev) = hk_rx.try_recv() {
            match ev {
                HotkeyEvent::Press if !paused => {
                    let _ = cmd_tx.send(PipelineCmd::Start);
                    match audio::Capture::start(audio_tx.clone()) {
                        Ok(c) => {
                            capture = Some(c);
                            overlay.set(OverlayState::Listening(0.0));
                        }
                        Err(e) => {
                            tray.notify("Microphone", &e.to_string());
                            let _ = cmd_tx.send(PipelineCmd::Stop);
                        }
                    }
                }
                HotkeyEvent::Release => {
                    capture = None;
                    let _ = cmd_tx.send(PipelineCmd::Stop);
                }
                HotkeyEvent::FixLast => correction::fix_last(&mut history, &dict, &tray),
                _ => {}
            }
        }
        while let Ok(m) = msg_rx.try_recv() {
            match m {
                PipelineMsg::Level(l) if capture.is_some() => overlay.set(OverlayState::Listening(l)),
                PipelineMsg::Level(_) => {}
                PipelineMsg::Processing => overlay.set(OverlayState::Processing),
                PipelineMsg::Done(e) => {
                    overlay.set(OverlayState::Hidden);
                    if !e.cleaned.is_empty() {
                        history.push(e);
                    }
                }
                PipelineMsg::Error(s) => {
                    overlay.set(OverlayState::Hidden);
                    capture = None;
                    tray.notify("Murmur", &s);
                }
            }
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    let _ = cmd_tx.send(PipelineCmd::Shutdown);
    Ok(())
}
