//! Live preview: a small streaming model transcribes while you speak, so the words can be shown
//! at the caret before you let go. It never changes what is pasted: Parakeet still decodes the
//! dictation, and each chunk Parakeet decodes at a pause replaces the preview's words for that
//! stretch, so the caption firms up as you go.

use anyhow::{anyhow, Result};
use crossbeam_channel::{Receiver, RecvTimeoutError};
use sherpa_onnx::{OnlineRecognizer, OnlineRecognizerConfig, OnlineStream};
use std::path::{Path, PathBuf};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub enum PreviewCmd {
    /// A dictation started: forget the last one.
    Start,
    /// Mic audio, 16 kHz mono, as the pipeline receives it.
    Audio(Vec<f32>),
    /// Parakeet decoded the speech up to here (at a pause); its text stands for that stretch.
    Commit(String),
    /// The key was let go or the dictation cancelled: no more text until the next Start.
    Stop,
}

/// The files the streaming model needs. Kroko ships fp32 ONNX files without an `int8` suffix.
const FILES: [&str; 4] = ["encoder.onnx", "decoder.onnx", "joiner.onnx", "tokens.txt"];

/// The encoder only appears after the final rename in `model_fetch::extract`.
pub fn is_installed(dir: &Path) -> bool {
    dir.join(FILES[0]).is_file()
}

/// The streaming recognizer and the stream for the speech since the last commit.
pub struct Engine {
    rec: OnlineRecognizer,
    stream: OnlineStream,
}

impl Engine {
    pub fn load(dir: &Path) -> Result<Engine> {
        for f in FILES {
            if !dir.join(f).exists() {
                return Err(anyhow!("preview model file missing: {}", dir.join(f).display()));
            }
        }
        let p = |f: &str| Some(dir.join(f).to_string_lossy().into_owned());
        let mut config = OnlineRecognizerConfig::default();
        config.model_config.transducer.encoder = p(FILES[0]);
        config.model_config.transducer.decoder = p(FILES[1]);
        config.model_config.transducer.joiner = p(FILES[2]);
        config.model_config.tokens = p(FILES[3]);
        // one thread: it keeps up at about a twentieth of real time, and leaves the cores to Parakeet
        config.model_config.num_threads = 1;
        config.model_config.provider = Some("cpu".into());
        config.decoding_method = Some("greedy_search".into());
        // the pipeline's VAD decides where speech ends; the preview only follows
        config.enable_endpoint = false;
        let t0 = Instant::now();
        let rec = OnlineRecognizer::create(&config).ok_or_else(|| anyhow!("create preview recognizer from {}", dir.display()))?;
        log::info!("preview model loaded in {:.2}s", t0.elapsed().as_secs_f32());
        let stream = rec.create_stream();
        Ok(Engine { rec, stream })
    }

    /// Starts over on fresh speech.
    fn restart(&mut self) {
        self.stream = self.rec.create_stream();
    }

    /// Feeds `samples` and returns the text heard since the last restart.
    fn accept(&mut self, samples: &[f32]) -> String {
        self.stream.accept_waveform(16_000, samples);
        while self.rec.is_ready(&self.stream) {
            self.rec.decode(&self.stream);
        }
        self.rec.get_result(&self.stream).map(|r| r.text.trim().to_string()).unwrap_or_default()
    }
}

/// Parakeet chunks kept in the caption's text; each is at least 3 s of speech.
const RECENT_CHUNKS: usize = 2;

/// The preview's raw text: Parakeet's chunks so far, then the streaming model's words since.
#[derive(Debug, Default)]
pub struct Composer {
    committed: Vec<String>,
    tail: String,
}

impl Composer {
    pub fn reset(&mut self) {
        self.committed.clear();
        self.tail.clear();
    }

    /// Parakeet's text for the speech up to the last pause; the live words for it are dropped.
    pub fn commit(&mut self, text: String) {
        if !text.is_empty() {
            self.committed.push(text);
        }
        self.tail.clear();
    }

    pub fn tail(&mut self, text: String) {
        self.tail = text;
    }

    /// The text worth showing: the caption only has room for the end, and a hands-free
    /// dictation can run for minutes, so only the last chunks are cleaned and shown.
    pub fn raw(&self) -> String {
        let recent = &self.committed[self.committed.len().saturating_sub(RECENT_CHUNKS)..];
        recent.iter().chain(std::iter::once(&self.tail)).filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join(" ")
    }
}

/// Runs the preview on its own thread, so it keeps up while Parakeet decodes a chunk. `clean`
/// shapes the raw text as a paste would be; `out` gets each new caption. The model loads on the
/// first Start, and again after `idle_unload` without one (zero: never unloaded).
pub fn spawn<C, O>(dir: PathBuf, idle_unload: Duration, clean: C, rx: Receiver<PreviewCmd>, out: O) -> JoinHandle<()>
where
    C: Fn(&str) -> String + Send + 'static,
    O: Fn(String) + Send + 'static,
{
    thread::Builder::new()
        .name("preview".into())
        .spawn(move || {
            let mut engine: Option<Engine> = None;
            // a load that failed isn't retried until Murmur restarts; it would fail every dictation
            let mut broken = false;
            let mut active = false;
            let mut text = Composer::default();
            let mut sent = String::new();
            let mut last_used = Instant::now();
            let mut next: Option<PreviewCmd> = None;
            loop {
                let cmd = match next.take() {
                    Some(c) => c,
                    None => match rx.recv_timeout(Duration::from_secs(30)) {
                        Ok(c) => c,
                        Err(RecvTimeoutError::Disconnected) => return,
                        Err(RecvTimeoutError::Timeout) => {
                            if !idle_unload.is_zero() && !active && engine.is_some() && last_used.elapsed() > idle_unload {
                                log::info!("unloading preview model after idle");
                                engine = None;
                            }
                            continue;
                        }
                    },
                };
                match cmd {
                    PreviewCmd::Start => {
                        last_used = Instant::now();
                        text.reset();
                        sent.clear();
                        if engine.is_none() && !broken && is_installed(&dir) {
                            match Engine::load(&dir) {
                                Ok(e) => engine = Some(e),
                                Err(e) => {
                                    log::error!("preview: {e:#}");
                                    broken = true;
                                }
                            }
                        }
                        if let Some(e) = engine.as_mut() {
                            e.restart();
                        }
                        active = engine.is_some();
                    }
                    PreviewCmd::Audio(mut samples) => {
                        // a slow machine falls behind: take everything queued in one decode
                        loop {
                            match rx.try_recv() {
                                Ok(PreviewCmd::Audio(more)) => samples.extend(more),
                                Ok(other) => {
                                    next = Some(other);
                                    break;
                                }
                                Err(_) => break,
                            }
                        }
                        if let (true, Some(e)) = (active, engine.as_mut()) {
                            text.tail(e.accept(&samples));
                            emit(&text, &clean, &mut sent, &out);
                        }
                    }
                    PreviewCmd::Commit(t) => {
                        if let (true, Some(e)) = (active, engine.as_mut()) {
                            text.commit(t);
                            e.restart();
                            emit(&text, &clean, &mut sent, &out);
                        }
                    }
                    PreviewCmd::Stop => {
                        active = false;
                        last_used = Instant::now();
                    }
                }
            }
        })
        .expect("spawn preview")
}

/// Sends the caption if it changed.
fn emit(text: &Composer, clean: &impl Fn(&str) -> String, sent: &mut String, out: &impl Fn(String)) {
    let raw = text.raw();
    let caption = if raw.is_empty() { String::new() } else { clean(&raw) };
    if caption != *sent {
        *sent = caption.clone();
        out(caption);
    }
}

/// The end of `text` that fits in `max` characters, cut at a word and led by an ellipsis, so the
/// words being spoken stay in view.
pub fn tail_of(text: &str, max: usize) -> String {
    let n = text.chars().count();
    if n <= max {
        return text.to_string();
    }
    let keep: String = text.chars().skip(n - max.saturating_sub(1)).collect();
    // drop a word cut in half, unless the whole tail is one word
    let keep = match keep.find(' ') {
        Some(i) if i + 1 < keep.len() => keep[i + 1..].to_string(),
        _ => keep,
    };
    format!("…{keep}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cleanup, config::Config, context::Profile, dictionary::Dictionary, snippets::Snippets};
    use sherpa_onnx::Wave;

    #[test]
    fn committed_chunks_come_before_the_live_words() {
        let mut c = Composer::default();
        c.tail("so I wanted".into());
        assert_eq!(c.raw(), "so I wanted");
        c.commit("So I wanted to set up the meeting.".into());
        assert_eq!(c.raw(), "So I wanted to set up the meeting.");
        c.tail("Can you".into());
        assert_eq!(c.raw(), "So I wanted to set up the meeting. Can you");
    }

    #[test]
    fn only_the_last_chunks_are_kept() {
        let mut c = Composer::default();
        for t in ["One.", "Two.", "Three."] {
            c.commit(t.into());
        }
        c.tail("four".into());
        assert_eq!(c.raw(), "Two. Three. four");
    }

    #[test]
    fn an_empty_commit_still_drops_the_live_words() {
        let mut c = Composer::default();
        c.tail("um".into());
        c.commit(String::new());
        assert_eq!(c.raw(), "");
    }

    #[test]
    fn reset_forgets_the_last_dictation() {
        let mut c = Composer::default();
        c.commit("Hello.".into());
        c.tail("there".into());
        c.reset();
        assert_eq!(c.raw(), "");
    }

    #[test]
    fn short_text_is_shown_whole() {
        assert_eq!(tail_of("Send it Friday.", 40), "Send it Friday.");
    }

    #[test]
    fn long_text_keeps_its_end_from_a_word() {
        let t = tail_of("the quick brown fox jumps over the lazy dog", 20);
        assert_eq!(t, "…over the lazy dog");
        assert!(t.chars().count() <= 20);
    }

    #[test]
    fn one_long_word_is_cut_not_dropped() {
        assert_eq!(tail_of("abcdefghijklmnop", 6), "…lmnop");
    }

    #[test]
    fn emit_skips_unchanged_text() {
        let sent_log = std::cell::RefCell::new(Vec::new());
        let out = |s: String| sent_log.borrow_mut().push(s);
        let clean = |s: &str| s.to_uppercase();
        let mut sent = String::new();
        let mut c = Composer::default();
        c.tail("hi".into());
        emit(&c, &clean, &mut sent, &out);
        emit(&c, &clean, &mut sent, &out);
        c.tail("hi there".into());
        emit(&c, &clean, &mut sent, &out);
        assert_eq!(*sent_log.borrow(), vec!["HI".to_string(), "HI THERE".to_string()]);
    }

    /// The streaming model on the speech WAV it ships with: words arrive before the audio ends.
    #[test]
    fn engine_hears_words_while_audio_streams() {
        let dir = Config::default().preview_model_dir();
        let wav = dir.join("test_wavs").join("0.wav");
        if !is_installed(&dir) || !wav.exists() {
            eprintln!("preview model not installed; skipped");
            return;
        }
        let wave = Wave::read(&wav.to_string_lossy()).expect("read wav");
        let mut e = Engine::load(&dir).unwrap();
        let samples = wave.samples();
        let half = samples.len() / 2;
        let early = e.accept(&samples[..half]);
        let all = e.accept(&samples[half..]);
        eprintln!("early={early:?} all={all:?}");
        assert!(!early.is_empty(), "words by halfway");
        assert!(all.len() >= early.len());
    }

    /// The preview thread on the WAV its model ships with, fed in 100 ms chunks the way the mic
    /// does, with a commit halfway as the pipeline sends at a pause: captions arrive while the audio
    /// streams, the committed text replaces the live words for its stretch, and Stop ends them.
    #[test]
    fn captions_follow_the_speech_and_take_committed_text() {
        let cfg = Config::default();
        let dir = cfg.preview_model_dir();
        let wav = dir.join("test_wavs").join("0.wav");
        if !is_installed(&dir) || !wav.exists() {
            eprintln!("preview model not installed; skipping");
            return;
        }
        let wave = Wave::read(&wav.to_string_lossy()).expect("read wav");
        let dict = Dictionary::seed();
        let clean_cfg = cfg.clone();
        let clean = move |raw: &str| cleanup::clean(raw, &dict, &Snippets::default(), &clean_cfg, Profile::Plain);
        let (tx, rx) = crossbeam_channel::unbounded();
        let (out_tx, out_rx) = crossbeam_channel::unbounded::<String>();
        let handle = spawn(dir, Duration::ZERO, clean, rx, move |t| {
            let _ = out_tx.send(t);
        });
        tx.send(PreviewCmd::Start).unwrap();
        let samples = wave.samples();
        let half = samples.len() / 2;
        for c in samples[..half].chunks(1_600) {
            tx.send(PreviewCmd::Audio(c.to_vec())).unwrap();
        }
        // wait for the first half's words, as a live dictation would show them
        let early = out_rx.recv_timeout(Duration::from_secs(60)).expect("a caption during the first half");
        eprintln!("early: {early}");
        tx.send(PreviewCmd::Commit("Um, committed words here.".into())).unwrap();
        for c in samples[half..].chunks(1_600) {
            tx.send(PreviewCmd::Audio(c.to_vec())).unwrap();
        }
        tx.send(PreviewCmd::Stop).unwrap();
        // a later caption than any sent before Stop can't come: collect until the thread goes quiet
        drop(tx);
        handle.join().unwrap();
        let captions: Vec<String> = out_rx.try_iter().collect();
        eprintln!("{captions:#?}");
        let last = captions.last().expect("captions after the commit");
        assert!(last.starts_with("Committed words here."), "cleaned committed text leads: {last}");
        assert!(last.len() > "Committed words here.".len(), "live words follow it: {last}");
        assert!(!last.contains("Ask not what your country can do for you. Ask not"), "no repeat of the first half");
    }
}
