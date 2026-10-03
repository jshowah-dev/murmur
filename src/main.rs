#![windows_subsystem = "windows"]

use murmur_lib::{audio, cleanup, config, context, dictionary, history, model_fetch, snippets, stt, update, vad};
mod about_ui;
mod audio_out;
mod autostart;
mod caret;
mod canvas;
mod correction;
mod correction_ui;
mod dictionary_panel;
mod editor;
mod editor_kit;
mod history_ui;
mod hotkey;
mod hotkey_ui;
mod inject;
#[allow(dead_code)] // generated tokens; not all are used yet
mod motion;
mod mote;
mod overlay;
mod pipeline;
mod setup_ui;
mod snippets_panel;
mod tray;

use anyhow::Result;
use config::Config;
use crossbeam_channel::unbounded;
use dictionary::Dictionary;
use history::History;
use hotkey::HotkeyEvent;
use mote::{landing_point, on_done, Landing, Message, Mote, Target};
use overlay::{Overlay, OverlayState};
use pipeline::{PipelineCmd, PipelineMsg};
use std::sync::{Arc, Mutex};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tray::{Tray, TrayEvent};
use windows::Win32::Foundation::{HWND, RECT};

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

/// The editor tab asked for on the command line, or `None` for the app itself.
fn wants_editor(mut args: impl Iterator<Item = String>) -> Option<editor::Tab> {
    args.nth(1).as_deref().and_then(editor::Tab::from_flag)
}

/// Opens the editor on `tab` as a separate process, so dictation keeps working while it's open.
fn open_editor(tray: &Tray, tab: editor::Tab) {
    let spawned = std::env::current_exe().and_then(|exe| std::process::Command::new(exe).arg(tab.flag()).spawn());
    if let Err(e) = spawned {
        log::error!("editor: {e}");
        tray.notify("Murmur", &format!("Couldn't open the editor: {e}"));
    }
}

/// Results from the update checker and download threads.
enum UpdateMsg {
    Available(update::Release),
    Downloaded(PathBuf),
    Failed(String),
    ModelProgress(u8),
    ModelUnpacking,
    ModelReady,
    ModelFailed(String),
}

/// Hands the hotkey thread the keys to listen for; none stops it listening.
fn set_ptt(shared: &Mutex<Vec<u16>>, vks: Vec<u16>) {
    if let Ok(mut keys) = shared.lock() {
        *keys = vks;
    }
}

fn update_label(tag: &str, installed: bool) -> String {
    if installed { format!("Update to v{tag}…") } else { format!("Get v{tag}…") }
}

/// What the "update available" balloon tells you to do; clicking it does the first part.
fn update_balloon_body(installed: bool) -> &'static str {
    if installed {
        "Click here to update, or right-click the Murmur icon and choose Update."
    } else {
        "Click here to get it, or right-click the Murmur icon and choose Get."
    }
}

fn model_offer_label() -> String {
    format!("Download new speech model ({} MB)…", model_fetch::parakeet().size / 1_000_000)
}

fn percent(done: u64, total: u64) -> u8 {
    (done.min(total) * 100).checked_div(total).map_or(100, |p| p as u8)
}

/// Downloads and unpacks the current default model next to the old one, reporting on `tx`.
/// A cancelled run (Murmur quit) keeps the `.part`, and the next attempt resumes it.
fn download_model(models: PathBuf, tx: crossbeam_channel::Sender<UpdateMsg>) {
    let asset = model_fetch::parakeet();
    let mut last = u8::MAX;
    let result = model_fetch::Fetcher::standard()
        .download(&asset, &models, &mut |n| {
            let p = percent(n, asset.size);
            if p != last {
                last = p;
                let _ = tx.send(UpdateMsg::ModelProgress(p));
            }
        }, &std::sync::atomic::AtomicBool::new(false))
        .and_then(|archive| {
            let _ = tx.send(UpdateMsg::ModelUnpacking);
            model_fetch::extract(&archive, &models, &mut |_| {})
        });
    let _ = tx.send(match result {
        Ok(dir) => {
            log::info!("model upgrade: installed {}", dir.display());
            UpdateMsg::ModelReady
        }
        Err(e) => {
            log::error!("model upgrade: {e}");
            UpdateMsg::ModelFailed(e.advice())
        }
    });
}

fn main() -> Result<()> {
    let editor_tab = wants_editor(std::env::args());
    init_logging(editor_tab.is_none());
    // the editor is its own process, so it must not take the app's single-instance mutex
    if let Some(tab) = editor_tab {
        return editor::run(tab);
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
    let mut cfg = match Config::load_or_create() {
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
    let model_state = config::resolve_model(&mut cfg, model_fetch::is_installed);
    if let config::ModelState::Switched { old } = &model_state {
        // not loaded yet, so nothing holds its files open
        log::info!("model upgrade: now on {}; removing {}", cfg.model_dir, old.display());
        if let Err(e) = std::fs::remove_dir_all(old) {
            log::warn!("remove old model: {e}");
        }
    }
    let model_dir = cfg.model_dir_path();
    let mut model_missing = !model_fetch::is_installed(&model_dir);
    let default_dir = cfg.model_dir == Config::default().model_dir;
    let welcomed = setup_ui::welcome_marker().exists();
    let plan = setup_ui::plan(model_missing, default_dir, welcomed);
    // where the setup card faded to a dot, for the mote that carries it to the pill
    let mut card_at = None;
    if plan != setup_ui::Plan::Skip {
        let models = model_dir.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| model_dir.clone());
        match setup_ui::run(models, plan) {
            // re-checked: the unpacked folder must be the one model_dir names
            setup_ui::SetupOutcome::Installed { from } => {
                model_missing = !model_fetch::is_installed(&model_dir);
                card_at = from;
            }
            setup_ui::SetupOutcome::Quit => {
                log::info!("model setup not finished; exiting");
                return Ok(());
            }
        }
    }
    // until the first dictation with words: quitting before trying brings it back next launch
    let mut invite_open = setup_ui::invites(welcomed, model_missing);
    let mut history = History::new(25);

    let (hk_tx, hk_rx) = unbounded::<HotkeyEvent>();
    let (cmd_tx, cmd_rx) = unbounded::<PipelineCmd>();
    let (msg_tx, msg_rx) = unbounded::<PipelineMsg>();
    let (audio_tx, audio_rx) = unbounded::<Vec<f32>>();
    // signalled by the capture when its stream dies; one pending signal is enough
    let (lost_tx, lost_rx) = crossbeam_channel::bounded::<()>(1);

    // shared with the hotkey thread, so the picker can change the key while Murmur runs
    let ptt_vks = Arc::new(Mutex::new(cfg.ptt_vks()));
    hotkey::spawn(ptt_vks.clone(), cfg.hands_free_max(), hk_tx);
    pipeline::spawn(cfg.clone(), dict.clone(), cmd_rx, msg_tx);

    let mut overlay = Overlay::create()?;
    let mut mote = Mote::create()?;
    // (window, caret) from the lookup that runs when you let go of the key
    let (caret_tx, caret_rx) = unbounded::<(isize, Option<RECT>)>();
    // the window a finished dictation's words will land in, until they do
    let mut awaiting: Option<isize> = None;
    // a Release sent Stop, so the dictation's Done should carry words
    let mut expect_words = false;
    // the caret lookup for the last Release hasn't come back yet
    let mut caret_pending = false;
    // "Didn't catch that" waiting on that lookup
    let mut unheard_pending = false;
    // the window a caret message is about; leaving it closes the message
    let mut said_in: Option<isize> = None;
    let tray = Tray::create(&cfg.ptt_key_label())?;
    let (up_tx, up_rx) = unbounded::<UpdateMsg>();
    let checker_tx = up_tx.clone();
    update::spawn_checker(move |r| {
        let _ = checker_tx.send(UpdateMsg::Available(r));
    });
    let installed_copy = match (std::env::current_exe(), std::env::var_os("LOCALAPPDATA")) {
        (Ok(exe), Some(lad)) => update::is_installed_copy(&exe, std::path::Path::new(&lad)),
        _ => false,
    };
    let mut offer = update::Offer::default();
    // The mic stays open while not paused: a rolling buffer of the last PRE_ROLL_SAMPLES is
    // fed to the pipeline ahead of the live audio on key-down, so the first consonant is not
    // lost to device start-up latency.
    let mic_start = mic_at_start(cfg.mic_always_on, invite_open && card_at.is_some());
    let mut capture = if mic_start == MicStart::Now { open_mic(&audio_tx, &lost_tx, &tray) } else { None };
    // when the deferred mic opens: once the dot has flown to the pill and opened
    let mut mic_due = (mic_start == MicStart::AfterCarry).then(|| Instant::now() + motion::scaled(mote::FLIGHT + motion::duration::ENTER));
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
    // where the invitation was said, to follow the pill when it moves
    let mut invite_at = overlay.above_physical();
    if invite_open {
        let from = card_at.unwrap_or_else(|| overlay.centre_physical());
        mote.say_until_dismissed(invite(&cfg.ptt_key_label()), from, Target::Pill(invite_at));
    }

    // only reachable with a custom model_dir: the default one is set up above
    if model_missing {
        log::error!("model missing at {} (LOCALAPPDATA={:?})", model_dir.display(), std::env::var("LOCALAPPDATA"));
        tray.notify("Model missing", &format!("Model missing at {}", model_dir.display()));
    }
    for msg in &startup_errors {
        tray.notify("Startup", msg);
    }
    if model_state == config::ModelState::UpgradeAvailable {
        tray.notify(
            &format!("A new speech model is available ({} MB)", model_fetch::parakeet().size / 1_000_000),
            "Download from the tray menu.",
        );
        tray.set_model(Some(&model_offer_label()), true);
    }

    'main: loop {
        if !overlay.pump_once() {
            break;
        }
        overlay.animate();
        // you switched windows mid-flight: the words won't land where the mote is
        if mote.is_active() && awaiting.is_some_and(|t| t != inject::foreground_hwnd()) {
            mote.fade();
            overlay.set_quiet(false);
            awaiting = None;
        }
        mote.animate();
        if said_in.is_some_and(|w| w != inject::foreground_hwnd()) {
            mote.dismiss();
            said_in = None;
        }
        if mic_due.is_some_and(|t| Instant::now() >= t) {
            mic_due = None;
            // a key-down may have opened it already; a pause opens it again on resume
            if capture.is_none() && !paused {
                capture = open_mic(&audio_tx, &lost_tx, &tray);
                mic_used = Instant::now();
            }
        }
        // the stream died (device unplugged, or the default input changed): open the current default
        if lost_rx.try_recv().is_ok() {
            capture.take();
            // the dead stream may have signalled again before it was dropped
            while lost_rx.try_recv().is_ok() {}
            if !forwarding {
                // what the old device left behind would be prepended to the next dictation
                while audio_rx.try_recv().is_ok() {}
                ring.clear();
            }
            if reopen_after_loss(paused, cfg.mic_always_on, forwarding) {
                log::info!("mic lost; reopening on the default input");
                capture = open_mic(&audio_tx, &lost_tx, &tray);
            }
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
        if overlay.take_right_click() {
            tray.show_menu(overlay.hwnd());
        }
        if let Some(ev) = tray.poll() {
            match ev {
                TrayEvent::TogglePause => {
                    paused = !paused;
                    tray.set_paused(paused);
                    overlay.set(resting(paused));
                    if paused {
                        // a paused key does nothing, so nothing should invite it
                        if mote.is_speaking() {
                            mote.dismiss();
                        }
                        capture.take();
                        ring.clear();
                    } else if cfg.mic_always_on {
                        capture = open_mic(&audio_tx, &lost_tx, &tray);
                        mic_used = Instant::now();
                    }
                }
                TrayEvent::FixLast => {
                    let out = correction::fix_last(&mut history, &dict, &tray);
                    said_in = after_fix(out, true, &mut mote, &overlay);
                    while hk_rx.try_recv().is_ok() {}
                }
                TrayEvent::History => {
                    history_ui::show(history.newest_first().map(|e| (e.cleaned.clone(), e.at)).collect(), cfg.ptt_key_label());
                    while hk_rx.try_recv().is_ok() {}
                }
                TrayEvent::EditDictionary => open_editor(&tray, editor::Tab::Dictionary),
                TrayEvent::EditSnippets => open_editor(&tray, editor::Tab::Snippets),
                TrayEvent::PttKey => {
                    // not listening while the window is open: pressing the current key there mustn't dictate
                    set_ptt(&ptt_vks, Vec::new());
                    let picked = hotkey_ui::show(cfg.ptt_vks());
                    if let Some(name) = picked.and_then(|vks| config::ptt_name_of(&vks)) {
                        log::info!("ptt key: {} -> {name}", cfg.ptt_key);
                        if let Err(e) = config::save_ptt_key(&name) {
                            log::error!("save ptt key: {e:#}");
                            tray.notify("Murmur", &format!("Couldn't save the push-to-talk key, so it lasts until Murmur restarts: {e}"));
                        }
                        let inviting = mote.is_saying(&invite(&cfg.ptt_key_label()));
                        cfg.ptt_key = name;
                        tray.set_ptt_label(&cfg.ptt_key_label());
                        // the invitation names the key, so it names the new one
                        if inviting {
                            mote.say_until_dismissed(invite(&cfg.ptt_key_label()), invite_at, Target::Pill(invite_at));
                        }
                    }
                    set_ptt(&ptt_vks, cfg.ptt_vks());
                    while hk_rx.try_recv().is_ok() {}
                }
                TrayEvent::OpenConfigDir => open_path(&config::config_dir()),
                TrayEvent::ToggleAutostart => {
                    // the registry, not the menu's own check state, says what's on
                    if let Err(e) = autostart::set(!autostart::is_enabled()) {
                        log::error!("autostart: {e:#}");
                        tray.notify("Murmur", &format!("Couldn't change start with Windows: {e}"));
                    }
                    tray.set_autostart_checked(autostart::is_enabled());
                }
                TrayEvent::Update | TrayEvent::BalloonUpdate if !installed_copy => {
                    if offer.release().is_some() {
                        open_path(std::path::Path::new(update::RELEASES_PAGE));
                    }
                }
                TrayEvent::Update => {
                    start_update(&mut offer, &tray, &up_tx);
                }
                TrayEvent::BalloonUpdate => {
                    let tag = offer.release().map(|r| r.tag.clone());
                    if let (Some(tag), true) = (tag, start_update(&mut offer, &tray, &up_tx)) {
                        // the balloon is gone and the tray menu is closed: say something's happening
                        tray.notify(&format!("Updating to Murmur v{tag}"), "Murmur restarts when it's installed.");
                    }
                }
                TrayEvent::DownloadModel => {
                    tray.set_model(Some("Downloading speech model… 0%"), false);
                    let models = Config::default().model_dir_path().parent().map(PathBuf::from).unwrap_or_default();
                    let tx = up_tx.clone();
                    std::thread::spawn(move || download_model(models, tx));
                }
                TrayEvent::About => {
                    let terms = dict.lock().map(|d| d.terms.len()).unwrap_or(0);
                    if let Some((r, clicked)) = about_ui::show(about_ui::model_label(&model_dir), terms, installed_copy) {
                        let tag = r.tag.clone();
                        // the hourly check may not have seen it yet: offer it in the tray too, without a balloon
                        if offer.available(r) {
                            log::info!("update available: v{tag} (from About)");
                            tray.set_update(Some(&update_label(&tag, installed_copy)), true);
                        }
                        if clicked && start_update(&mut offer, &tray, &up_tx) {
                            // the window is gone and the tray menu is closed: say something's happening
                            tray.notify(&format!("Updating to Murmur v{tag}"), "Murmur restarts when it's installed.");
                        }
                    }
                    while hk_rx.try_recv().is_ok() {}
                }
                TrayEvent::Quit => break 'main,
            }
        }
        while let Ok(ev) = hk_rx.try_recv() {
            match ev {
                HotkeyEvent::Down if !paused => {
                    // a new dictation closes the last message
                    if mote.is_speaking() {
                        mote.dismiss();
                    }
                    said_in = None;
                    // a lookup still out from the last release must not answer for this dictation
                    unheard_pending = false;
                    if capture.is_none() {
                        capture = open_mic(&audio_tx, &lost_tx, &tray);
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
                    awaiting = None;
                    expect_words = false;
                    unheard_pending = false;
                    mote.fade();
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
                    let dictating = forwarding;
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
                    if dictating {
                        let target = inject::foreground_hwnd();
                        awaiting = Some(target);
                        expect_words = true;
                        caret_pending = true;
                        let tx = caret_tx.clone();
                        // UIA can take tens of ms; the loop mustn't wait for it
                        std::thread::spawn(move || {
                            let t0 = Instant::now();
                            let found = caret::find(HWND(target as *mut _));
                            log::info!("caret lookup: {found:?} in {} ms", t0.elapsed().as_millis());
                            let caret = match found {
                                Some(caret::Anchor::Caret(r)) => Some(r),
                                _ => None,
                            };
                            let _ = tx.send((target, caret));
                        });
                    }
                }
                HotkeyEvent::FixLast => {
                    log::info!("fix-last hotkey");
                    let out = correction::fix_last(&mut history, &dict, &tray);
                    said_in = after_fix(out, false, &mut mote, &overlay);
                    while hk_rx.try_recv().is_ok() {}
                }
                _ => {}
            }
        }
        while let Ok((target, caret)) = caret_rx.try_recv() {
            caret_pending = false;
            let to = landing_point(awaiting, target, inject::foreground_hwnd(), caret);
            if std::mem::take(&mut unheard_pending) {
                awaiting = None;
                said_in = say(&mut mote, &overlay, Message::plain(UNHEARD), overlay.centre_physical(), to);
            } else if let Some(to) = to {
                mote.launch(overlay.centre_physical(), to);
                overlay.set_quiet(true);
            }
        }
        while let Ok(m) = msg_rx.try_recv() {
            match m {
                PipelineMsg::Processing => overlay.set(OverlayState::Processing),
                PipelineMsg::Done(e) => {
                    overlay.set(resting(paused));
                    overlay.set_quiet(false);
                    let heard = !e.cleaned.is_empty();
                    let finished = std::mem::take(&mut expect_words);
                    match unheard(finished && !heard, mote.is_active(), caret_pending) {
                        Some(Unheard::WaitForCaret) => unheard_pending = true,
                        Some(Unheard::Here) => {
                            // the mote is at (or flying to) the caret: it says it there; if it's already fading, say starts a fresh flight to the pill target
                            awaiting = None;
                            mote.say(Message::plain(UNHEARD), overlay.centre_physical(), Target::Pill(overlay.above_physical()));
                            said_in = Some(inject::foreground_hwnd());
                        }
                        Some(Unheard::AbovePill) => {
                            awaiting = None;
                            said_in = say(&mut mote, &overlay, Message::plain(UNHEARD), overlay.centre_physical(), None);
                        }
                        None => {
                            let same = awaiting.take().is_some_and(|t| t == inject::foreground_hwnd());
                            match on_done(mote.is_active(), heard, same) {
                                Landing::Dissolve => mote.dissolve(),
                                Landing::Fade => mote.fade(),
                                Landing::Pulse => overlay.pulse(),
                                Landing::Nothing => {}
                            }
                        }
                    }
                    if heard {
                        history.push(e);
                    }
                    if heard && std::mem::take(&mut invite_open) {
                        setup_ui::mark_welcomed();
                    }
                }
                PipelineMsg::Error(s) => {
                    overlay.set(resting(paused));
                    awaiting = None;
                    expect_words = false;
                    unheard_pending = false;
                    mote.fade();
                    overlay.set_quiet(false);
                    forwarding = false;
                    listening = false;
                    locked = false;
                    while audio_rx.try_recv().is_ok() {}
                    tray.notify("Murmur", &s);
                }
            }
        }
        while let Ok(m) = up_rx.try_recv() {
            match m {
                UpdateMsg::Available(r) => {
                    let tag = r.tag.clone();
                    if offer.available(r) {
                        log::info!("update available: v{tag}");
                        tray.set_update(Some(&update_label(&tag, installed_copy)), true);
                        if update::should_alert(&tag, update::read_alerted().as_deref()) {
                            tray.notify_update(&format!("Murmur v{tag} is available"), update_balloon_body(installed_copy));
                            update::write_alerted(&tag);
                        }
                    }
                }
                // the installer would force-close the editor and lose unsaved edits
                UpdateMsg::Downloaded(_) if editor::is_open() => {
                    offer.failed();
                    restore_update_item(&tray, &offer, installed_copy);
                    tray.notify("Close the Dictionary & Snippets window to update", "Then choose Update again.");
                }
                UpdateMsg::Downloaded(path) => match update::install(&path) {
                    Ok(()) => {
                        log::info!("installing {}; quitting", path.display());
                        break 'main;
                    }
                    Err(e) => {
                        log::error!("start installer: {e}");
                        offer.failed();
                        restore_update_item(&tray, &offer, installed_copy);
                        tray.notify("Update failed", &format!("Couldn't start the installer: {e}"));
                    }
                },
                UpdateMsg::Failed(why) => {
                    log::error!("update download: {why}");
                    offer.failed();
                    restore_update_item(&tray, &offer, installed_copy);
                    tray.notify("Update failed", &why);
                }
                UpdateMsg::ModelProgress(p) => tray.set_model(Some(&format!("Downloading speech model… {p}%")), false),
                UpdateMsg::ModelUnpacking => tray.set_model(Some("Unpacking speech model…"), false),
                UpdateMsg::ModelReady => {
                    tray.set_model(None, false);
                    tray.notify("New speech model ready", "It's used from the next start.");
                }
                UpdateMsg::ModelFailed(why) => {
                    tray.set_model(Some(&model_offer_label()), true);
                    tray.notify("Speech model download failed", &why);
                }
            }
        }
        resting_tick = resting_tick.wrapping_add(1);
        if resting_tick % 64 == 0 {
            overlay.refresh_resting();
            let m = invite(&cfg.ptt_key_label());
            if let Some((from, to)) = follow_pill(invite_at, overlay.above_physical()).filter(|_| mote.is_saying(&m)) {
                // closed here, then carried over: a fresh flight from where it was
                mote.dismiss();
                mote.say_until_dismissed(m, from, Target::Pill(to));
                invite_at = to;
            }
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

const UNHEARD: &str = "Didn't catch that";

/// What the pill says on first run until you first press the key.
fn invite(key: &str) -> Message {
    use correction_ui::{rgb, GREEN, TEXT};
    Message(vec![("Hold ".into(), rgb(TEXT)), (key.to_string(), rgb(GREEN)), (" and talk".into(), rgb(TEXT))])
}

/// The pill moved (it follows the foreground window's monitor): carry the invitation from
/// where it was said to above the pill's new place.
fn follow_pill(said_at: (f32, f32), above: (f32, f32)) -> Option<((f32, f32), (f32, f32))> {
    ((said_at.0 - above.0).abs() > 1.0 || (said_at.1 - above.1).abs() > 1.0).then_some((said_at, above))
}

#[derive(Debug, PartialEq)]
enum MicStart {
    Now,
    /// once the card's dot has reached the pill: a Bluetooth mic can take a second to open,
    /// and the loop that draws the mote waits on it
    AfterCarry,
    Never,
}

fn mic_at_start(always_on: bool, carried: bool) -> MicStart {
    match (always_on, carried) {
        (false, _) => MicStart::Never,
        (true, false) => MicStart::Now,
        (true, true) => MicStart::AfterCarry,
    }
}

/// Where "Didn't catch that" goes when a dictation ends with no words.
#[derive(Debug, PartialEq)]
enum Unheard {
    /// the mote is already out: it says it where it is
    Here,
    /// the caret is still being looked up: say it once it's known
    WaitForCaret,
    AbovePill,
}

/// Only a finished dictation (`expect_words`: a Release sent Stop) is unheard; a cancelled
/// hold ends with no words too, and says nothing.
fn unheard(expect_words: bool, mote_active: bool, caret_pending: bool) -> Option<Unheard> {
    if !expect_words {
        return None;
    }
    Some(if mote_active {
        Unheard::Here
    } else if caret_pending {
        Unheard::WaitForCaret
    } else {
        Unheard::AbovePill
    })
}

/// The mote says `m`, flying from `from` to `to`, or to just above the pill with no caret.
/// Returns the window a caret message is about, for `said_in`.
fn say(mote: &mut Mote, overlay: &Overlay, m: Message, from: (f32, f32), to: Option<(f32, f32)>) -> Option<isize> {
    match to {
        Some(p) => {
            mote.say(m, from, Target::Caret(p));
            Some(inject::foreground_hwnd())
        }
        None => {
            mote.say(m, from, Target::Pill(overlay.above_physical()));
            None
        }
    }
}

/// The caret in the window in front, if it has one.
fn foreground_caret() -> Option<(f32, f32)> {
    match caret::find(HWND(inject::foreground_hwnd() as *mut _)) {
        Some(caret::Anchor::Caret(r)) => Some(mote::caret_point(r)),
        _ => None,
    }
}

/// Answers fix-last at the caret. From the tray with nothing to fix, above the pill instead.
fn after_fix(out: correction::FixOutcome, from_tray: bool, mote: &mut Mote, overlay: &Overlay) -> Option<isize> {
    let to = if out.nothing_to_fix && from_tray { None } else { foreground_caret() };
    let from = out.card.unwrap_or_else(|| overlay.centre_physical());
    correction::fix_message(&out).and_then(|m| say(mote, overlay, m, from, to))
}

/// Whether a mic whose stream died is opened again now, or left for the next key-down.
fn reopen_after_loss(paused: bool, always_on: bool, forwarding: bool) -> bool {
    !paused && (always_on || forwarding)
}

fn open_mic(
    audio_tx: &crossbeam_channel::Sender<Vec<f32>>,
    lost_tx: &crossbeam_channel::Sender<()>,
    tray: &Tray,
) -> Option<audio::Capture> {
    match audio::Capture::start(audio_tx.clone(), lost_tx.clone()) {
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

/// Downloads the release on offer, unless a download already runs; `Downloaded` then installs it.
/// True when a download started.
fn start_update(offer: &mut update::Offer, tray: &Tray, tx: &crossbeam_channel::Sender<UpdateMsg>) -> bool {
    let Some(r) = offer.start() else { return false };
    tray.set_update(Some("Downloading update…"), false);
    let tx = tx.clone();
    std::thread::spawn(move || {
        let msg = match update::download(&r, &update::download_dir()) {
            Ok(path) => UpdateMsg::Downloaded(path),
            Err(e) => UpdateMsg::Failed(update::failure_text(&e)),
        };
        let _ = tx.send(msg);
    });
    true
}

fn restore_update_item(tray: &Tray, offer: &update::Offer, installed: bool) {
    if let Some(r) = offer.release() {
        tray.set_update(Some(&update_label(&r.tag, installed)), true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> impl Iterator<Item = String> {
        a.iter().map(|s| s.to_string()).collect::<Vec<_>>().into_iter()
    }

    #[test]
    fn model_offer_names_the_download_size() {
        assert_eq!(model_offer_label(), "Download new speech model (482 MB)…");
    }

    #[test]
    fn percent_is_whole_and_capped() {
        assert_eq!(percent(0, 482_468_385), 0);
        assert_eq!(percent(241_234_192, 482_468_385), 49);
        assert_eq!(percent(482_468_385, 482_468_385), 100);
        assert_eq!(percent(10, 0), 100);
    }

    #[test]
    fn a_lost_mic_is_reopened_when_it_would_be_open() {
        // (paused, always_on, forwarding)
        assert!(reopen_after_loss(false, true, false));
        assert!(reopen_after_loss(false, false, true));
        assert!(reopen_after_loss(false, true, true));
    }

    #[test]
    fn a_lost_mic_stays_closed_when_paused_or_opened_on_demand() {
        assert!(!reopen_after_loss(false, false, false));
        assert!(!reopen_after_loss(true, true, false));
        assert!(!reopen_after_loss(true, false, true));
    }

    #[test]
    fn only_a_finished_dictation_with_no_words_is_unheard() {
        assert_eq!(unheard(false, true, false), None, "a cancelled hold (tap, Esc, Shift) says nothing");
        assert_eq!(unheard(true, true, false), Some(Unheard::Here), "the mote is out: it says it where it is");
        assert_eq!(unheard(true, false, true), Some(Unheard::WaitForCaret), "the caret lookup is still running");
        assert_eq!(unheard(true, false, false), Some(Unheard::AbovePill), "no caret found");
    }

    #[test]
    fn update_label_depends_on_the_copy() {
        assert_eq!(update_label("0.4.5", true), "Update to v0.4.5…");
        assert_eq!(update_label("0.4.5", false), "Get v0.4.5…");
    }

    #[test]
    fn update_balloon_says_what_to_do() {
        assert!(update_balloon_body(true).starts_with("Click here to update"));
        assert!(update_balloon_body(false).starts_with("Click here to get it"));
        // Windows cuts a balloon's text at 255 UTF-16 units
        assert!(update_balloon_body(true).len() < 256 && update_balloon_body(false).len() < 256);
    }

    #[test]
    fn editor_flags_select_the_tab() {
        assert_eq!(wants_editor(args(&["murmur.exe", "--dictionary"])), Some(editor::Tab::Dictionary));
        assert_eq!(wants_editor(args(&["murmur.exe", "--snippets"])), Some(editor::Tab::Snippets));
        assert_eq!(wants_editor(args(&["murmur.exe"])), None);
        assert_eq!(wants_editor(args(&["murmur.exe", "--other"])), None);
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

    #[test]
    fn the_invitation_follows_the_pill_to_another_monitor() {
        assert_eq!(follow_pill((960.0, 1000.0), (960.4, 1000.0)), None, "same place");
        assert_eq!(follow_pill((960.0, 1000.0), (2880.0, 1000.0)), Some(((960.0, 1000.0), (2880.0, 1000.0))));
    }

    #[test]
    fn the_mic_waits_for_the_carry_on_first_run() {
        assert_eq!(mic_at_start(true, false), MicStart::Now);
        assert_eq!(mic_at_start(true, true), MicStart::AfterCarry, "opening it would stall the card's dot mid-handover");
        assert_eq!(mic_at_start(false, true), MicStart::Never);
        assert_eq!(mic_at_start(false, false), MicStart::Never);
    }

    #[test]
    fn the_invitation_names_the_key_in_green() {
        let m = invite("Right Ctrl");
        let text: String = m.0.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!(text, "Hold Right Ctrl and talk");
        let key = m.0.iter().find(|(s, _)| s == "Right Ctrl").expect("the key is its own run");
        assert_eq!(key.1, correction_ui::rgb(correction_ui::GREEN));
        assert!(m.0.iter().filter(|(s, _)| s != "Right Ctrl").all(|(_, c)| *c == correction_ui::rgb(correction_ui::TEXT)));
    }
}
