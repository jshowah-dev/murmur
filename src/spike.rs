//! The macOS binary while the port is under way: the real pipeline, driven from the terminal.
//! Enter starts a dictation, Enter stops it, and the text that would be pasted is printed.

use crate::pipeline::{self, PipelineCmd, PipelineMsg};
use anyhow::{Context, Result};
use crossbeam_channel::unbounded;
use murmur_lib::config::Config;
use murmur_lib::dictionary::Dictionary;
use murmur_lib::model_fetch::{self, Fetcher};
use murmur_lib::{audio, snippets};
use std::io::{BufRead, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub fn main() -> Result<()> {
    simplelog::TermLogger::init(
        log::LevelFilter::Info,
        simplelog::Config::default(),
        simplelog::TerminalMode::Stderr,
        simplelog::ColorChoice::Auto,
    )?;
    let cfg = Config::load_or_create()?;
    let model_dir = cfg.model_dir_path();
    if !model_fetch::is_installed(&model_dir) {
        let models = model_dir.parent().context("model_dir has no parent folder")?;
        install(models)?;
    }
    let dict = Arc::new(Mutex::new(Dictionary::load_or_seed()?));
    snippets::ensure_file()?;
    let mut snippet_file = snippets::SnippetFile::new(snippets::path());
    if let Some(p) = snippet_file.refresh() {
        log::warn!("{}: {}", p.path.display(), p.reason);
    }

    let (cmd_tx, cmd_rx) = unbounded::<PipelineCmd>();
    let (msg_tx, msg_rx) = unbounded::<PipelineMsg>();
    pipeline::spawn(cfg, dict, snippet_file, cmd_rx, msg_tx);

    let (audio_tx, audio_rx) = unbounded::<Vec<f32>>();
    let _capture = audio::Capture::start(audio_tx, crossbeam_channel::bounded(1).0)?;
    let forwarding = Arc::new(AtomicBool::new(false));
    {
        let (forwarding, cmd_tx) = (forwarding.clone(), cmd_tx.clone());
        std::thread::spawn(move || {
            for chunk in audio_rx {
                if forwarding.load(Ordering::SeqCst) {
                    let _ = cmd_tx.send(PipelineCmd::Audio(chunk));
                }
            }
        });
    }

    let mut lines = std::io::stdin().lock().lines();
    loop {
        prompt("Enter to dictate, q to quit: ");
        let Some(line) = lines.next().transpose()? else { break };
        if line.trim().eq_ignore_ascii_case("q") {
            break;
        }
        // Start goes on the channel before any audio does: the forwarder only sends once it sees the flag
        cmd_tx.send(PipelineCmd::Start)?;
        forwarding.store(true, Ordering::SeqCst);
        prompt("listening… Enter to stop ");
        lines.next().transpose()?;
        forwarding.store(false, Ordering::SeqCst);
        cmd_tx.send(PipelineCmd::Stop)?;
        loop {
            match msg_rx.recv()? {
                PipelineMsg::Processing => {}
                PipelineMsg::Done(e) if e.cleaned.is_empty() => {
                    println!("(didn't catch that)");
                    break;
                }
                PipelineMsg::Done(e) => {
                    println!("raw:     {}\ncleaned: {}", e.raw, e.cleaned);
                    break;
                }
                // a failed Start sends Error and no Done
                PipelineMsg::Error(s) => {
                    println!("error: {s}");
                    break;
                }
                PipelineMsg::FileProblem(p) => log::warn!("{}: {}", p.path.display(), p.reason),
            }
        }
    }
    let _ = cmd_tx.send(PipelineCmd::Shutdown);
    Ok(())
}

fn prompt(s: &str) {
    print!("{s}");
    let _ = std::io::stdout().flush();
}

/// The first-run download from `setup_ui`, with progress on the terminal.
fn install(models: &Path) -> Result<()> {
    let fetcher = Fetcher::standard();
    let (vad, parakeet) = (model_fetch::vad(), model_fetch::parakeet());
    let total = vad.size + parakeet.size;
    let mut last = u64::MAX;
    let mut report = |done: u64| {
        let pct = done * 100 / total;
        if pct != last {
            last = pct;
            eprint!("\rdownloading the speech model into {}: {pct}%", models.display());
        }
    };
    let never = AtomicBool::new(false);
    fetcher.download(&vad, models, &mut |n| report(n), &never)?;
    fetcher.download(&parakeet, models, &mut |n| report(vad.size + n), &never)?;
    eprintln!("\nunpacking…");
    let dir = model_fetch::extract(&models.join(&parakeet.file), models, &mut |_| {})?;
    eprintln!("installed {}", dir.display());
    Ok(())
}
