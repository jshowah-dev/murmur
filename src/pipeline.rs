use crate::cleanup;
use crate::config::Config;
use crate::dictionary::{self, Dictionary, DictionaryFile};
use crate::history::Entry;
use crate::inject;
use crate::snippets::{self, SnippetFile};
use crate::stt::Recognizer;
use crate::vad::Vad;
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub enum PipelineCmd {
    Start,
    Stop,
    Abort,
    Audio(Vec<f32>),
    Shutdown,
}

pub enum PipelineMsg {
    /// Live text while recording: decoded chunks plus a fresh decode of the speech since.
    Partial(String),
    Processing,
    Done(Entry),
    Error(String),
}

const MIN_SPEECH_SAMPLES: usize = 16_000 * 300 / 1000;
// silence inserted between VAD segments so the model hears the pause but sees one utterance
const SEGMENT_GAP: usize = 16_000 * 200 / 1000;
/// Recordings are decoded in chunks at pauses while they are still being made. A chunk's text
/// is final and is what the live preview shows for that speech, so each preview decode only
/// covers the speech since the last chunk. Shorter than 5 s, the model ends too many chunks with
/// a period where the speaker only paused.
const CHUNK_MIN: usize = 16_000 * 5;
/// The VAD force-splits speech at 15 s (`max_speech_duration`); a segment this long may end
/// mid-word, so a chunk only ends on a segment shorter than this.
const NATURAL_MAX: usize = 16_000 * 14;

/// True when the buffered speech is long enough to decode now and the last segment ended at a pause.
fn chunk_ready(buffered: usize, last_segment: usize) -> bool {
    buffered >= CHUNK_MIN && last_segment < NATURAL_MAX
}

const PREVIEW_EVERY: Duration = Duration::from_millis(700);
/// Speech needed before the model is worth running for a preview.
const PREVIEW_MIN: usize = 16_000 / 2;

struct State {
    cfg: Config,
    dict: Arc<Mutex<Dictionary>>,
    snippets: SnippetFile,
    dict_file: DictionaryFile,
    tx: Sender<PipelineMsg>,
    vad: Option<Vad>,
    rec: Option<Recognizer>,
    speech: Vec<f32>,
    speech_samples: usize,
    /// Text of chunks already decoded during this recording.
    texts: Vec<String>,
    /// Audio of the utterance the VAD hasn't released as a segment yet, from `onset`.
    tail: Vec<f32>,
    /// Index in `tail` where the utterance in progress starts, pre-roll included; None in silence.
    onset: Option<usize>,
    /// New speech or text since the last preview.
    dirty: bool,
    last_preview: Instant,
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

    /// Segments are buffered, not transcribed: the model punctuates whatever it is
    /// given as a full sentence, so per-segment decoding turned every pause into a period.
    /// Returns the speech length of the last segment, 0 if there were none.
    fn collect_segments(&mut self, segs: Vec<crate::vad::Segment>) -> usize {
        let mut last = 0;
        for s in segs {
            last = s.samples.len().saturating_sub(crate::vad::PRE_ROLL);
            self.speech_samples += last;
            if !self.speech.is_empty() {
                self.speech.extend(std::iter::repeat(0.0).take(SEGMENT_GAP));
            }
            self.speech.extend(s.samples);
        }
        last
    }

    /// Decode the buffered speech and keep its text for the final paste.
    fn decode_chunk(&mut self) {
        if let Some(rec) = self.rec.as_ref() {
            let t0 = Instant::now();
            let text = rec.transcribe(&self.speech);
            log::info!("chunk: {:.1}s audio decoded in {}ms", self.speech.len() as f32 / 16_000.0, t0.elapsed().as_millis());
            if !text.is_empty() {
                self.texts.push(text);
            }
        }
        self.speech.clear();
    }

    fn start(&mut self) {
        if let Err(e) = self.ensure_loaded() {
            let _ = self.tx.send(PipelineMsg::Error(e.to_string()));
            return;
        }
        self.vad.as_mut().unwrap().reset();
        self.speech.clear();
        self.speech_samples = 0;
        self.texts.clear();
        self.tail.clear();
        self.onset = None;
        self.dirty = false;
        self.recording = true;
    }

    fn audio(&mut self, chunk: Vec<f32>) {
        if !self.recording {
            return;
        }
        let segs = self.vad.as_mut().map(|v| v.push(&chunk)).unwrap_or_default();
        self.track_tail(&chunk, !segs.is_empty());
        if segs.is_empty() {
            return;
        }
        let last = self.collect_segments(segs);
        if chunk_ready(self.speech.len(), last) {
            self.decode_chunk();
        }
    }

    /// Keep the audio of the utterance in progress for the preview; in silence keep only enough
    /// for the next utterance's pre-roll, since the VAD reports speech a little after it starts.
    fn track_tail(&mut self, chunk: &[f32], released: bool) {
        let speaking = self.vad.as_ref().is_some_and(|v| v.detected());
        self.tail.extend_from_slice(chunk);
        if released {
            self.onset = None;
            self.dirty = true;
        }
        if !speaking {
            self.onset = None;
            let keep = 2 * crate::vad::PRE_ROLL;
            if self.tail.len() > keep {
                self.tail.drain(..self.tail.len() - keep);
            }
            return;
        }
        self.dirty = true;
        let onset = *self.onset.get_or_insert(self.tail.len().saturating_sub(chunk.len() + 2 * crate::vad::PRE_ROLL));
        self.tail.drain(..onset);
        self.onset = Some(0);
    }

    /// Speech not yet in a chunk: buffered segments plus the utterance in progress.
    fn pending_speech(&self) -> Vec<f32> {
        let mut out = self.speech.clone();
        if self.onset.is_some() {
            if !out.is_empty() {
                out.extend(std::iter::repeat(0.0).take(SEGMENT_GAP));
            }
            out.extend_from_slice(&self.tail);
        }
        out
    }

    /// Send the text so far. Runs between commands, so it delays a Stop by at most one decode
    /// of the speech since the last chunk.
    fn preview(&mut self) {
        if !self.recording || !self.dirty || self.last_preview.elapsed() < PREVIEW_EVERY {
            return;
        }
        self.dirty = false;
        self.last_preview = Instant::now();
        let mut raw = self.texts.join(" ");
        let pending = self.pending_speech();
        if let (true, Some(rec)) = (pending.len() >= PREVIEW_MIN, self.rec.as_ref()) {
            let text = rec.transcribe(&pending);
            if !text.is_empty() {
                if !raw.is_empty() {
                    raw.push(' ');
                }
                raw.push_str(&text);
            }
        }
        if raw.is_empty() {
            return;
        }
        let text = {
            let d = self.dict.lock().unwrap_or_else(|e| e.into_inner());
            cleanup::clean(&raw, &d, &self.snippets.current, &self.cfg)
        };
        let _ = self.tx.send(PipelineMsg::Partial(text));
    }

    fn stop(&mut self) {
        if !self.recording {
            return;
        }
        let stop_start = Instant::now();
        self.recording = false;
        let _ = self.tx.send(PipelineMsg::Processing);
        let segs = self.vad.as_mut().map(|v| v.flush()).unwrap_or_default();
        self.collect_segments(segs);
        self.last_used = Instant::now();
        if self.speech_samples >= MIN_SPEECH_SAMPLES && !self.speech.is_empty() {
            self.decode_chunk();
        }
        let raw = std::mem::take(&mut self.texts).join(" ");
        self.speech.clear();
        if raw.is_empty() {
            let _ = self.tx.send(PipelineMsg::Done(Entry { raw: String::new(), cleaned: String::new(), inject: None, at: Instant::now() }));
            return;
        }
        log::debug!("raw: {raw}");
        if let Some(e) = self.snippets.refresh() {
            let _ = self.tx.send(PipelineMsg::Error(e));
        }
        if let Some(e) = self.dict_file.refresh(&self.dict) {
            let _ = self.tx.send(PipelineMsg::Error(e));
        }
        let cleaned = {
            let d = self.dict.lock().unwrap_or_else(|e| e.into_inner());
            cleanup::clean(&raw, &d, &self.snippets.current, &self.cfg)
        };
        let inject = match inject::paste(&cleaned) {
            Ok(r) => Some(r),
            Err(e) => {
                let _ = self.tx.send(PipelineMsg::Error(format!("paste failed: {e}")));
                None
            }
        };
        log::info!("dictated {} chars, release_to_text_ms={}", cleaned.chars().count(), stop_start.elapsed().as_millis());
        let _ = self.tx.send(PipelineMsg::Done(Entry { raw, cleaned, inject, at: Instant::now() }));
    }

    fn abort(&mut self) {
        self.recording = false;
        self.speech.clear();
        self.speech_samples = 0;
        self.texts.clear();
        self.tail.clear();
        self.onset = None;
        if let Some(v) = self.vad.as_mut() {
            v.reset();
        }
        let _ = self.tx.send(PipelineMsg::Done(Entry { raw: String::new(), cleaned: String::new(), inject: None, at: Instant::now() }));
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
            let mut st = State { cfg, dict, snippets: SnippetFile::new(snippets::path()), dict_file: DictionaryFile::new(dictionary::path()), tx, vad: None, rec: None, speech: vec![], speech_samples: 0, texts: vec![], tail: vec![], onset: None, dirty: false, last_preview: Instant::now(), recording: false, last_used: Instant::now() };
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
            Ok(PipelineCmd::Audio(c)) => {
                st.audio(c);
                if rx.is_empty() {
                    st.preview();
                }
            }
            Ok(PipelineCmd::Stop) => st.stop(),
            Ok(PipelineCmd::Abort) => st.abort(),
            Ok(PipelineCmd::Shutdown) | Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => st.maybe_unload(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_recordings_are_never_chunked() {
        assert!(!chunk_ready(16_000 * 4, 16_000 * 2));
    }

    #[test]
    fn long_buffer_chunks_at_a_natural_pause() {
        assert!(chunk_ready(16_000 * 6, 16_000 * 3));
    }

    #[test]
    fn force_split_segment_does_not_end_a_chunk() {
        assert!(!chunk_ready(16_000 * 25, 16_000 * 15));
    }
}
