//! Records 16 kHz mono 16-bit WAV fixtures for tests/stt_integration.rs.
//! Usage: cargo run --release --bin record -- name_one name_two ...
//! For each name: Enter starts, Enter stops, writes tests/fixtures/<name>.wav.
use murmur_lib::audio;

use anyhow::Result;
use std::io::{stdin, Write};
use std::path::Path;

fn write_wav(path: &Path, samples: &[f32]) -> Result<()> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&audio::TARGET_RATE.to_le_bytes());
    out.extend_from_slice(&(audio::TARGET_RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).to_le_bytes());
    }
    std::fs::write(path, out)?;
    Ok(())
}

fn wait_enter() {
    let mut line = String::new();
    let _ = stdin().read_line(&mut line);
}

fn main() -> Result<()> {
    let names: Vec<String> = std::env::args().skip(1).collect();
    if names.is_empty() {
        eprintln!("usage: record <name> [<name> ...]");
        std::process::exit(2);
    }
    let dir = Path::new("tests/fixtures");
    std::fs::create_dir_all(dir)?;
    let (tx, rx) = crossbeam_channel::unbounded::<Vec<f32>>();
    let _cap = audio::Capture::start(tx, crossbeam_channel::bounded(1).0)?;
    for name in names {
        print!("[{name}] Enter to start... ");
        std::io::stdout().flush()?;
        wait_enter();
        // keep 500 ms of pre-roll so the first word is not clipped, as in main.rs
        let mut samples: Vec<f32> = Vec::new();
        while let Ok(chunk) = rx.try_recv() {
            samples.extend(chunk);
        }
        let pre = (audio::TARGET_RATE / 2) as usize;
        if samples.len() > pre {
            samples.drain(..samples.len() - pre);
        }
        print!("recording - Enter to stop... ");
        std::io::stdout().flush()?;
        wait_enter();
        while let Ok(chunk) = rx.try_recv() {
            samples.extend(chunk);
        }
        let path = dir.join(format!("{name}.wav"));
        write_wav(&path, &samples)?;
        println!(
            "wrote {} ({:.1} s, rms {:.3})",
            path.display(),
            samples.len() as f32 / audio::TARGET_RATE as f32,
            audio::rms(&samples)
        );
    }
    Ok(())
}
