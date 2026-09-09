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
use std::collections::VecDeque;
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
    // The mic stays open while not paused: a rolling buffer of the last PRE_ROLL_SAMPLES is
    // fed to the pipeline ahead of the live audio on key-down, so the first consonant is not
    // lost to device start-up latency.
    let mut capture = open_mic(&audio_tx, &tray);
    let mut ring: VecDeque<f32> = VecDeque::with_capacity(PRE_ROLL_SAMPLES * 2);
    let mut forwarding = false;
    let mut paused = false;
    let mut listening = false;
    let mut resting_tick: u32 = 0;
    overlay.set(OverlayState::Idle);

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
            if forwarding {
                let _ = cmd_tx.send(PipelineCmd::Audio(chunk));
            } else {
                ring.extend(chunk);
                while ring.len() > PRE_ROLL_SAMPLES {
                    ring.pop_front();
                }
            }
        }
        if let Some(ev) = tray.poll() {
            match ev {
                TrayEvent::TogglePause => {
                    paused = !paused;
                    tray.set_paused(paused);
                    overlay.set(resting(paused));
                    if paused {
                        capture.take();
                        ring.clear();
                    } else {
                        capture = open_mic(&audio_tx, &tray);
                    }
                }
                TrayEvent::FixLast => {
                    correction::fix_last(&mut history, &dict, &tray);
                    while hk_rx.try_recv().is_ok() {}
                }
                TrayEvent::OpenDictionary => open_path(&config::config_dir().join("dictionary.toml")),
                TrayEvent::OpenConfigDir => open_path(&config::config_dir()),
                TrayEvent::TestNotify => tray.notify("Murmur", "Notifications are working"),
                TrayEvent::Quit => break,
            }
        }
        while let Ok(ev) = hk_rx.try_recv() {
            match ev {
                HotkeyEvent::Down if !paused => {
                    if capture.is_none() {
                        capture = open_mic(&audio_tx, &tray);
                    }
                    if capture.is_none() {
                        continue;
                    }
                    let _ = cmd_tx.send(PipelineCmd::Start);
                    let _ = cmd_tx.send(PipelineCmd::Audio(ring.drain(..).collect()));
                    forwarding = true;
                }
                HotkeyEvent::Press if !paused => {
                    listening = true;
                    overlay.set(OverlayState::Listening(0.0));
                }
                HotkeyEvent::Cancel => {
                    forwarding = false;
                    while audio_rx.try_recv().is_ok() {}
                    listening = false;
                    let _ = cmd_tx.send(PipelineCmd::Abort);
                }
                HotkeyEvent::Release => {
                    while let Ok(chunk) = audio_rx.try_recv() {
                        let _ = cmd_tx.send(PipelineCmd::Audio(chunk));
                    }
                    forwarding = false;
                    listening = false;
                    let _ = cmd_tx.send(PipelineCmd::Stop);
                }
                HotkeyEvent::FixLast => {
                    log::info!("fix-last hotkey");
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
                    overlay.set(resting(paused));
                    if !e.cleaned.is_empty() {
                        log::info!("raw: {}", e.raw);
                        history.push(e);
                    }
                }
                PipelineMsg::Error(s) => {
                    overlay.set(resting(paused));
                    forwarding = false;
                    listening = false;
                    while audio_rx.try_recv().is_ok() {}
                    tray.notify("Murmur", &s);
                }
            }
        }
        resting_tick = resting_tick.wrapping_add(1);
        if resting_tick % 64 == 0 {
            overlay.refresh_resting();
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    let _ = cmd_tx.send(PipelineCmd::Shutdown);
    Ok(())
}

/// Audio kept while idle and prepended on key-down: 500 ms at 16 kHz.
const PRE_ROLL_SAMPLES: usize = 8_000;

fn open_mic(audio_tx: &crossbeam_channel::Sender<Vec<f32>>, tray: &Tray) -> Option<audio::Capture> {
    match audio::Capture::start(audio_tx.clone()) {
        Ok(c) => Some(c),
        Err(e) => {
            log::error!("open mic: {e}");
            tray.notify("Microphone", &e.to_string());
            None
        }
    }
}

fn resting(paused: bool) -> OverlayState {
    if paused { OverlayState::Paused } else { OverlayState::Idle }
}
