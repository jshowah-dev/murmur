use crate::cleanup;
use crate::config::Config;
use crate::context::{self, Profile};
use crate::dictionary::{self, Dictionary, DictionaryFile};
use crate::history::Entry;
use crate::inject;
use crate::notice::{self, FileProblem};
use crate::snippets::SnippetFile;
use crate::stt::Recognizer;
use crate::vad::Vad;
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub enum PipelineCmd {
    Start,
    /// Carries the moment the hotkey was released, for the release-to-text timing.
    Stop(Instant),
    Abort,
    Audio(Vec<f32>),
    Shutdown,
}

pub enum PipelineMsg {
    Processing,
    Done(Entry),
    /// "Scratch that": nothing was pasted. `undo` when it was said on its own, so the
    /// dictation before it should come back out.
    Scratch { undo: bool },
    /// Said above the pill: the dictation couldn't go ahead.
    Error(String),
    /// A settings file changed and is broken; the previous version stays in use.
    FileProblem(FileProblem),
}

const MIN_SPEECH_SAMPLES: usize = 16_000 * 300 / 1000;
// silence inserted between VAD segments so the model hears the pause but sees one utterance
const SEGMENT_GAP: usize = 16_000 * 200 / 1000;
/// Long recordings are decoded in chunks while they are still being made: whole-utterance
/// decode time and memory grow faster than the audio (283 s took 56 s and 3 GB).
const CHUNK_MIN: usize = 16_000 * 20;
/// The VAD force-splits speech at 15 s (`max_speech_duration`); a segment this long may end
/// mid-word, so a chunk only ends on a segment shorter than this.
const NATURAL_MAX: usize = 16_000 * 14;

/// A pause this long usually ends a sentence, so decoding there doesn't put a period mid-sentence
/// the way decoding at every 500 ms VAD pause did.
const LONG_PAUSE: usize = 16_000;
/// Less speech than this waits for the next pause: a short fragment decodes poorly on its own.
const PAUSE_MIN: usize = 16_000 * 3;

/// True when the buffered speech is long enough to decode now and the last segment ended at a pause.
fn chunk_ready(buffered: usize, last_segment: usize) -> bool {
    buffered >= CHUNK_MIN && last_segment < NATURAL_MAX
}

/// True when the speaker has paused long enough to decode what's buffered while they're quiet,
/// so a release only waits for the speech after the last long pause.
fn pause_ready(buffered: usize, quiet: usize) -> bool {
    buffered >= PAUSE_MIN && quiet >= LONG_PAUSE
}

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
    recording: bool,
    last_used: Instant,
}

impl State {
    fn ensure_loaded(&mut self) -> anyhow::Result<()> {
        if self.vad.is_none() {
            self.vad = Some(Vad::new(&self.cfg.vad_model_path(), self.cfg.min_silence_ms)?);
        }
        if self.rec.is_none() {
            self.rec = Some(Recognizer::load(&self.cfg.model_dir_path(), self.cfg.recognition_threads())?);
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
            log::error!("load model: {e:#}");
            let _ = self.tx.send(PipelineMsg::Error(notice::MODEL_LOAD.into()));
            return;
        }
        self.vad.as_mut().unwrap().reset();
        self.speech.clear();
        self.speech_samples = 0;
        self.texts.clear();
        self.recording = true;
    }

    fn audio(&mut self, chunk: Vec<f32>) {
        if !self.recording {
            return;
        }
        let Some(vad) = self.vad.as_mut() else { return };
        let segs = vad.push(&chunk);
        let quiet = vad.quiet_samples();
        let last = self.collect_segments(segs);
        // chunk_ready judges the segment that just ended, so it waits for one to arrive
        if pause_ready(self.speech.len(), quiet) || (last > 0 && chunk_ready(self.speech.len(), last)) {
            self.decode_chunk();
        }
    }

    fn stop(&mut self, released: Instant) {
        if !self.recording {
            return;
        }
        let picked_up = Instant::now();
        self.recording = false;
        let _ = self.tx.send(PipelineMsg::Processing);
        let segs = self.vad.as_mut().map(|v| v.flush()).unwrap_or_default();
        self.collect_segments(segs);
        self.last_used = Instant::now();
        let tail_secs = self.speech.len() as f32 / 16_000.0;
        if self.speech_samples >= MIN_SPEECH_SAMPLES && !self.speech.is_empty() {
            self.decode_chunk();
        }
        let decoded = Instant::now();
        let raw = std::mem::take(&mut self.texts).join(" ");
        self.speech.clear();
        if raw.is_empty() {
            let _ = self.tx.send(PipelineMsg::Done(Entry { raw: String::new(), cleaned: String::new(), inject: None, at: Instant::now() }));
            return;
        }
        log::debug!("raw: {raw}");
        if cleanup::scratches_itself(&raw, &self.cfg) {
            let undo = cleanup::is_scratch(&raw, &self.cfg);
            log::info!("scratch that (undo {undo})");
            let _ = self.tx.send(PipelineMsg::Scratch { undo });
            return;
        }
        if let Some(p) = self.snippets.refresh() {
            let _ = self.tx.send(PipelineMsg::FileProblem(p));
        }
        if let Some(p) = self.dict_file.refresh(&self.dict) {
            let _ = self.tx.send(PipelineMsg::FileProblem(p));
        }
        // read now, as the text is about to be pasted: the profile belongs to the window that receives it
        let profile = if self.cfg.format_by_context { context::detect(&self.cfg) } else { Profile::Plain };
        log::info!("profile: {}", profile.name());
        let cleaned = {
            let d = self.dict.lock().unwrap_or_else(|e| e.into_inner());
            cleanup::clean(&raw, &d, &self.snippets.current, &self.cfg, profile)
        };
        let cleaned_at = Instant::now();
        // paste falls back to typing the text in, so an error here is only logged
        let pasted = cleanup::spaced_after(crate::caret::char_before(), &cleaned);
        let inject = inject::paste(&pasted).map_err(|e| log::error!("paste: {e:#}")).ok();
        let ms = |a: Instant, b: Instant| b.duration_since(a).as_millis();
        log::info!(
            "dictated {} chars, release_to_text_ms={} (queue {}, decode {} on {tail_secs:.1}s tail, clean {}, paste {})",
            cleaned.chars().count(),
            released.elapsed().as_millis(),
            ms(released, picked_up),
            ms(picked_up, decoded),
            ms(decoded, cleaned_at),
            cleaned_at.elapsed().as_millis()
        );
        let _ = self.tx.send(PipelineMsg::Done(Entry { raw, cleaned, inject, at: Instant::now() }));
    }

    fn abort(&mut self) {
        self.recording = false;
        self.speech.clear();
        self.speech_samples = 0;
        self.texts.clear();
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

pub fn spawn(cfg: Config, dict: Arc<Mutex<Dictionary>>, snippets: SnippetFile, rx: Receiver<PipelineCmd>, tx: Sender<PipelineMsg>) -> JoinHandle<()> {
    thread::Builder::new()
        .name("pipeline".into())
        .spawn(move || {
            let mut st = State { cfg, dict, snippets, dict_file: DictionaryFile::new(dictionary::path()), tx, vad: None, rec: None, speech: vec![], speech_samples: 0, texts: vec![], recording: false, last_used: Instant::now() };
            // Spec § Error handling: a panic is logged and the loop restarts; the tray survives.
            loop {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&mut st, &rx)));
                match result {
                    Ok(()) => break,
                    Err(_) => {
                        log::error!("pipeline panicked; restarting");
                        let _ = st.tx.send(PipelineMsg::Error(notice::RESTARTED.into()));
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
            Ok(PipelineCmd::Stop(at)) => st.stop(at),
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
        assert!(!chunk_ready(16_000 * 19, 16_000 * 2));
    }

    #[test]
    fn long_buffer_chunks_at_a_natural_pause() {
        assert!(chunk_ready(16_000 * 25, 16_000 * 6));
    }

    #[test]
    fn force_split_segment_does_not_end_a_chunk() {
        assert!(!chunk_ready(16_000 * 25, 16_000 * 15));
    }

    #[test]
    fn long_pause_decodes_what_is_buffered() {
        assert!(pause_ready(16_000 * 5, 16_000));
    }

    #[test]
    fn short_pause_keeps_buffering() {
        assert!(!pause_ready(16_000 * 5, 16_000 * 6 / 10));
    }

    #[test]
    fn short_fragment_waits_for_more_speech() {
        assert!(!pause_ready(16_000 * 2, 16_000 * 2));
    }
}
