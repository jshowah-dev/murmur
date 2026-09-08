use crate::audio::rms;
use crate::cleanup;
use crate::config::Config;
use crate::dictionary::Dictionary;
use crate::history::Entry;
use crate::inject;
use crate::stt::Recognizer;
use crate::vad::Vad;
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub enum PipelineCmd {
    Start,
    Stop,
    Audio(Vec<f32>),
    Shutdown,
}

pub enum PipelineMsg {
    Level(f32),
    Processing,
    Done(Entry),
    Error(String),
}

const MIN_SPEECH_SAMPLES: usize = 16_000 * 300 / 1000;

struct State {
    cfg: Config,
    dict: Arc<Mutex<Dictionary>>,
    tx: Sender<PipelineMsg>,
    vad: Option<Vad>,
    rec: Option<Recognizer>,
    parts: Vec<String>,
    raw_parts: Vec<String>,
    speech_samples: usize,
    recording: bool,
    last_used: Instant,
}

impl State {
    fn ensure_loaded(&mut self) -> anyhow::Result<()> {
        if self.vad.is_none() {
            self.vad = Some(Vad::new(&self.cfg.vad_model_path(), self.cfg.min_silence_ms)?);
        }
        if self.rec.is_none() {
            self.rec = Some(Recognizer::load(&self.cfg.model_dir_path(), self.cfg.threads)?);
        }
        self.last_used = Instant::now();
        Ok(())
    }

    fn transcribe_segments(&mut self, segs: Vec<crate::vad::Segment>) {
        let Some(rec) = self.rec.as_ref() else { return };
        for s in segs {
            self.speech_samples += s.samples.len();
            let text = rec.transcribe(&s.samples);
            if !text.is_empty() {
                self.raw_parts.push(text.clone());
                self.parts.push(text);
            }
        }
    }

    fn start(&mut self) {
        if let Err(e) = self.ensure_loaded() {
            let _ = self.tx.send(PipelineMsg::Error(e.to_string()));
            return;
        }
        self.vad.as_mut().unwrap().reset();
        self.parts.clear();
        self.raw_parts.clear();
        self.speech_samples = 0;
        self.recording = true;
    }

    fn audio(&mut self, chunk: Vec<f32>) {
        if !self.recording {
            return;
        }
        let _ = self.tx.send(PipelineMsg::Level((rms(&chunk) * 6.0).min(1.0)));
        let segs = self.vad.as_mut().map(|v| v.push(&chunk)).unwrap_or_default();
        self.transcribe_segments(segs);
    }

    fn stop(&mut self) {
        if !self.recording {
            return;
        }
        self.recording = false;
        let _ = self.tx.send(PipelineMsg::Processing);
        let segs = self.vad.as_mut().map(|v| v.flush()).unwrap_or_default();
        self.transcribe_segments(segs);
        self.last_used = Instant::now();
        if self.speech_samples < MIN_SPEECH_SAMPLES || self.parts.is_empty() {
            let _ = self.tx.send(PipelineMsg::Done(Entry { raw: String::new(), cleaned: String::new(), inject: None }));
            return;
        }
        let raw = cleanup::join_parts(&self.raw_parts);
        let cleaned = {
            let d = self.dict.lock().unwrap();
            cleanup::clean(&cleanup::join_parts(&self.parts), &d, &self.cfg)
        };
        let inject = match inject::paste(&cleaned) {
            Ok(r) => Some(r),
            Err(e) => {
                let _ = self.tx.send(PipelineMsg::Error(format!("paste failed: {e}")));
                None
            }
        };
        log::info!("dictated {} chars", cleaned.chars().count());
        let _ = self.tx.send(PipelineMsg::Done(Entry { raw, cleaned, inject }));
    }

    fn maybe_unload(&mut self) {
        let mins = self.cfg.idle_unload_minutes;
        if mins > 0 && !self.recording && self.rec.is_some() && self.last_used.elapsed() > Duration::from_secs(mins * 60) {
            log::info!("unloading model after idle");
            self.rec = None;
        }
    }
}

pub fn spawn(cfg: Config, dict: Arc<Mutex<Dictionary>>, rx: Receiver<PipelineCmd>, tx: Sender<PipelineMsg>) -> JoinHandle<()> {
    thread::Builder::new()
        .name("pipeline".into())
        .spawn(move || {
            let mut st = State { cfg, dict, tx, vad: None, rec: None, parts: vec![], raw_parts: vec![], speech_samples: 0, recording: false, last_used: Instant::now() };
            // Spec § Error handling: a panic is logged and the loop restarts; the tray survives.
            loop {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&mut st, &rx)));
                match result {
                    Ok(()) => break,
                    Err(_) => {
                        log::error!("pipeline panicked; restarting");
                        let _ = st.tx.send(PipelineMsg::Error("pipeline error, restarted".into()));
                        st.recording = false;
                        st.vad = None;
                        st.rec = None;
                    }
                }
            }
        })
        .expect("spawn pipeline")
}

/// Returns when Shutdown arrives or the channel closes.
fn run(st: &mut State, rx: &Receiver<PipelineCmd>) {
    loop {
        match rx.recv_timeout(Duration::from_secs(30)) {
            Ok(PipelineCmd::Start) => st.start(),
            Ok(PipelineCmd::Audio(c)) => st.audio(c),
            Ok(PipelineCmd::Stop) => st.stop(),
            Ok(PipelineCmd::Shutdown) | Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => st.maybe_unload(),
        }
    }
}
