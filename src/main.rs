#![windows_subsystem = "windows"]

use murmur_lib::{audio, cleanup, config, dictionary, history, model_fetch, snippets, stt, vad};
mod audio_out;
mod autostart;
mod caret;
mod correction;
mod correction_ui;
mod dictionary_editor;
mod dictionary_panel;
mod history_ui;
mod hotkey;
mod inject;
#[allow(dead_code)] // generated tokens; not all are used yet
mod motion;
mod overlay;
mod pipeline;
mod setup_ui;
mod tray;

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
use std::time::{Duration, Instant};
use tray::{Tray, TrayEvent};

const MAX_LOG_BYTES: u64 = 1024 * 1024;

/// `may_truncate` is false for the editor process: the running app may be writing to this log.
fn open_log(path: &std::path::Path, may_truncate: bool) -> std::io::Result<std::fs::File> {
    let truncate = may_truncate && std::fs::metadata(path).map(|m| m.len() > MAX_LOG_BYTES).unwrap_or(false);
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true);
    if truncate {
        opts.write(true).truncate(true);
    } else {
        opts.append(true);
    }
    opts.open(path)
}

fn init_logging(may_truncate: bool) {
    let dir = config::config_dir();
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(f) = open_log(&dir.join("murmur.log"), may_truncate) {
        // the logger accepts DEBUG; the global max level decides, raised once config says debug_log
        let _ = simplelog::WriteLogger::init(log::LevelFilter::Debug, simplelog::Config::default(), f);
    }
    log::set_max_level(log::LevelFilter::Info);
}

fn open_path(p: &std::path::Path) {
    let _ = std::process::Command::new("explorer.exe").arg(p).spawn();
}

const EDITOR_ARG: &str = "--dictionary";

fn wants_editor(mut args: impl Iterator<Item = String>) -> bool {
    args.nth(1).as_deref() == Some(EDITOR_ARG)
}

fn main() -> Result<()> {
    let editor = wants_editor(std::env::args());
    init_logging(!editor);
    // the editor is its own process, so it must not take the app's single-instance mutex
    if editor {
        return dictionary_editor::run();
    }
    log::info!("murmur {} starting, cwd {:?}", env!("CARGO_PKG_VERSION"), std::env::current_dir().ok());
    // second instance would capture the same hotkey and paste every dictation twice
    let _instance = unsafe {
        use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
        use windows::Win32::System::Threading::CreateMutexW;
        let m = CreateMutexW(None, false, windows::core::w!("Local\\Murmur.SingleInstance"))?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            log::warn!("another instance is running; exiting");
            return Ok(());
        }
        m
    };
    let mut startup_errors: Vec<String> = Vec::new();
    let cfg = match Config::load_or_create() {
        Ok(c) => c,
        Err(e) => {
            log::error!("{e:#}");
            startup_errors.push(format!("config: {e}"));
            Config::default()
        }
    };
    if cfg.debug_log {
        log::set_max_level(log::LevelFilter::Debug);
    }
    let dict = Arc::new(Mutex::new(match Dictionary::load_or_seed() {
        Ok(d) => d,
        Err(e) => {
            log::error!("{e:#}");
            startup_errors.push(format!("dictionary: {e}"));
            Dictionary::empty_unloaded()
        }
    }));
    if let Err(e) = snippets::ensure_file() {
        log::error!("{e:#}");
        startup_errors.push(format!("snippets: {e}"));
    }
    // Before anything else starts: the pipeline loads the model as soon as it's spawned.
    let model_dir = cfg.model_dir_path();
    let mut model_missing = !model_fetch::is_installed(&model_dir);
    let default_dir = cfg.model_dir == Config::default().model_dir;
    let plan = setup_ui::plan(model_missing, default_dir, setup_ui::welcome_marker().exists());
    if plan != setup_ui::Plan::Skip {
        let models = model_dir.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| model_dir.clone());
        match setup_ui::run(models, plan, cfg.ptt_key_label()) {
            // re-checked: the unpacked folder must be the one model_dir names
            setup_ui::SetupOutcome::Installed => model_missing = !model_fetch::is_installed(&model_dir),
            setup_ui::SetupOutcome::Quit => {
                log::info!("model setup not finished; exiting");
                return Ok(());
            }
        }
    }
    let mut history = History::new(25);

    let (hk_tx, hk_rx) = unbounded::<HotkeyEvent>();
    let (cmd_tx, cmd_rx) = unbounded::<PipelineCmd>();
    let (msg_tx, msg_rx) = unbounded::<PipelineMsg>();
    let (audio_tx, audio_rx) = unbounded::<Vec<f32>>();

    hotkey::spawn(cfg.ptt_vk(), cfg.hands_free_max(), hk_tx);
    pipeline::spawn(cfg.clone(), dict.clone(), cmd_rx, msg_tx);

    let mut overlay = Overlay::create()?;
    let tray = Tray::create()?;
    // The mic stays open while not paused: a rolling buffer of the last PRE_ROLL_SAMPLES is
    // fed to the pipeline ahead of the live audio on key-down, so the first consonant is not
    // lost to device start-up latency.
    let mut capture = if cfg.mic_always_on { open_mic(&audio_tx, &tray) } else { None };
    let mut mic_used = Instant::now();
    let mut ring: VecDeque<f32> = VecDeque::with_capacity(PRE_ROLL_SAMPLES * 2);
    let mut forwarding = false;
    let mut paused = false;
    let mut listening = false;
    // hands-free: recording continues after the key is let go
    let mut locked = false;
    let mut resting_tick: u32 = 0;
    let mut output_mute = audio_out::OutputMute::new();
    overlay.set(OverlayState::Idle);

    // only reachable with a custom model_dir: the default one is set up above
    if model_missing {
        log::error!("model missing at {} (LOCALAPPDATA={:?})", model_dir.display(), std::env::var("LOCALAPPDATA"));
        tray.notify("Model missing", &format!("Model missing at {}", model_dir.display()));
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
                // level is metered here, not in the pipeline, so it keeps moving while a chunk decodes
                if listening {
                    let l = (audio::rms(&chunk) * 6.0).min(1.0);
                    overlay.set(if locked { OverlayState::Locked(l) } else { OverlayState::Listening(l) });
                }
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
                    } else if cfg.mic_always_on {
                        capture = open_mic(&audio_tx, &tray);
                        mic_used = Instant::now();
                    }
                }
                TrayEvent::FixLast => {
                    correction::fix_last(&mut history, &dict, &tray);
                    while hk_rx.try_recv().is_ok() {}
                }
                TrayEvent::History => {
                    if history.last().is_none() {
                        tray.notify("History", "nothing dictated yet");
                    } else {
                        history_ui::show(history.newest_first().map(|e| (e.cleaned.clone(), e.at)).collect());
                        while hk_rx.try_recv().is_ok() {}
                    }
                }
                TrayEvent::EditDictionary => {
                    // a separate process, so dictation keeps working while it's open
                    let spawned = std::env::current_exe().and_then(|exe| std::process::Command::new(exe).arg(EDITOR_ARG).spawn());
                    if let Err(e) = spawned {
                        log::error!("dictionary editor: {e}");
                        tray.notify("Murmur", &format!("Couldn't open the dictionary editor: {e}"));
                    }
                }
                TrayEvent::OpenSnippets => open_path(&snippets::path()),
                TrayEvent::OpenConfigDir => open_path(&config::config_dir()),
                TrayEvent::ToggleAutostart => {
                    // the registry, not the menu's own check state, says what's on
                    if let Err(e) = autostart::set(!autostart::is_enabled()) {
                        log::error!("autostart: {e:#}");
                        tray.notify("Murmur", &format!("Couldn't change start with Windows: {e}"));
                    }
                    tray.set_autostart_checked(autostart::is_enabled());
                }
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
                    mic_used = Instant::now();
                    if cfg.mute_output {
                        output_mute.mute();
                    }
                    let _ = cmd_tx.send(PipelineCmd::Start);
                    let _ = cmd_tx.send(PipelineCmd::Audio(ring.drain(..).collect()));
                    forwarding = true;
                }
                HotkeyEvent::Press if !paused => {
                    listening = true;
                    overlay.set(OverlayState::Listening(0.0));
                }
                HotkeyEvent::Latch if forwarding => {
                    listening = true;
                    locked = true;
                    overlay.set(OverlayState::Locked(0.0));
                }
                HotkeyEvent::Cancel => {
                    output_mute.restore();
                    forwarding = false;
                    while audio_rx.try_recv().is_ok() {}
                    listening = false;
                    locked = false;
                    let _ = cmd_tx.send(PipelineCmd::Abort);
                    if !cfg.mic_always_on {
                        capture.take();
                    }
                }
                HotkeyEvent::Release => {
                    while let Ok(chunk) = audio_rx.try_recv() {
                        let _ = cmd_tx.send(PipelineCmd::Audio(chunk));
                    }
                    output_mute.restore();
                    forwarding = false;
                    listening = false;
                    locked = false;
                    let _ = cmd_tx.send(PipelineCmd::Stop);
                    if !cfg.mic_always_on {
                        capture.take();
                    }
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
                PipelineMsg::Processing => overlay.set(OverlayState::Processing),
                PipelineMsg::Done(e) => {
                    overlay.set(resting(paused));
                    if !e.cleaned.is_empty() {
                        history.push(e);
                    }
                }
                PipelineMsg::Error(s) => {
                    overlay.set(resting(paused));
                    forwarding = false;
                    listening = false;
                    locked = false;
                    while audio_rx.try_recv().is_ok() {}
                    tray.notify("Murmur", &s);
                }
            }
        }
        resting_tick = resting_tick.wrapping_add(1);
        if resting_tick % 64 == 0 {
            overlay.refresh_resting();
            let mins = cfg.idle_unload_minutes;
            if mins > 0 && capture.is_some() && !forwarding && mic_used.elapsed() > Duration::from_secs(mins * 60) {
                log::info!("closing mic after idle");
                capture.take();
                ring.clear();
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> impl Iterator<Item = String> {
        a.iter().map(|s| s.to_string()).collect::<Vec<_>>().into_iter()
    }

    #[test]
    fn dictionary_flag_selects_the_editor() {
        assert!(wants_editor(args(&["murmur.exe", "--dictionary"])));
        assert!(!wants_editor(args(&["murmur.exe"])));
        assert!(!wants_editor(args(&["murmur.exe", "--other"])));
    }

    fn big_log(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("murmur-log-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("murmur.log");
        std::fs::write(&p, vec![b'x'; MAX_LOG_BYTES as usize + 1]).unwrap();
        p
    }

    #[test]
    fn oversized_log_is_truncated_when_allowed() {
        let p = big_log("truncate");
        drop(open_log(&p, true).unwrap());
        assert_eq!(std::fs::metadata(&p).unwrap().len(), 0);
    }

    #[test]
    fn oversized_log_is_kept_when_truncation_is_not_allowed() {
        let p = big_log("keep");
        drop(open_log(&p, false).unwrap());
        assert_eq!(std::fs::metadata(&p).unwrap().len(), MAX_LOG_BYTES + 1);
    }
}
