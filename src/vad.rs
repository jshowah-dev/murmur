use anyhow::{anyhow, Result};
use sherpa_onnx::{VadModelConfig, VoiceActivityDetector};
use std::path::Path;

pub const WINDOW: usize = 512;
/// Raw audio prepended to every segment. Silero trims the onset, and the first consonant
/// ("Call" -> "All", "Stevie" -> "evie") was landing in the trimmed part.
pub const PRE_ROLL: usize = 4_000; // 250 ms @ 16 kHz

pub struct Segment {
    pub samples: Vec<f32>,
}

pub struct Vad {
    inner: VoiceActivityDetector,
    pending: Vec<f32>,
    /// Every sample fed to the detector since the last reset; `SpeechSegment::start` indexes it.
    fed: Vec<f32>,
}

impl Vad {
    pub fn new(model: &Path, min_silence_ms: u32) -> Result<Vad> {
        if let Err(e) = std::fs::metadata(model) {
            return Err(anyhow!("VAD model unreadable: {}: {e}", model.display()));
        }
        let mut config = VadModelConfig::default();
        config.silero_vad.model = Some(model.to_string_lossy().into_owned());
        config.silero_vad.threshold = 0.5;
        config.silero_vad.min_silence_duration = min_silence_ms as f32 / 1000.0;
        config.silero_vad.min_speech_duration = 0.25;
        config.silero_vad.max_speech_duration = 15.0;
        config.silero_vad.window_size = WINDOW as i32;
        config.sample_rate = 16_000;
        config.num_threads = 1;
        config.provider = Some("cpu".into());
        let inner = VoiceActivityDetector::create(&config, 120.0).ok_or_else(|| anyhow!("create VAD from {}", model.display()))?;
        Ok(Vad { inner, pending: Vec::new(), fed: Vec::new() })
    }

    fn drain(&mut self) -> Vec<Segment> {
        let mut out = Vec::new();
        while let Some(seg) = self.inner.front() {
            let start = seg.start().max(0) as usize;
            let from = start.saturating_sub(PRE_ROLL);
            let mut samples = Vec::with_capacity(start - from + seg.samples().len());
            samples.extend_from_slice(&self.fed[from..start.min(self.fed.len())]);
            samples.extend_from_slice(seg.samples());
            out.push(Segment { samples });
            self.inner.pop();
        }
        out
    }

    fn feed(&mut self, window: &[f32]) {
        self.fed.extend_from_slice(window);
        self.inner.accept_waveform(window);
    }

    pub fn push(&mut self, samples: &[f32]) -> Vec<Segment> {
        self.pending.extend_from_slice(samples);
        let mut offset = 0;
        while offset + WINDOW <= self.pending.len() {
            let window: Vec<f32> = self.pending[offset..offset + WINDOW].to_vec();
            self.feed(&window);
            offset += WINDOW;
        }
        self.pending.drain(..offset);
        self.drain()
    }

    pub fn flush(&mut self) -> Vec<Segment> {
        if !self.pending.is_empty() {
            let mut tail = std::mem::take(&mut self.pending);
            tail.resize(WINDOW, 0.0);
            self.feed(&tail);
        }
        self.inner.flush();
        self.drain()
    }

    pub fn reset(&mut self) {
        self.pending.clear();
        self.fed.clear();
        self.inner.reset();
        let _ = self.drain();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1 s of silence, then the speech WAV that ships with the model: the first segment must
    /// carry PRE_ROLL samples from before the onset Silero reports.
    #[test]
    fn segment_carries_pre_roll() {
        let cfg = crate::config::Config::default();
        let wav = cfg.model_dir_path().join("test_wavs").join("0.wav");
        if !cfg.vad_model_path().exists() || !wav.exists() {
            eprintln!("model not installed; skipped");
            return;
        }
        let wave = sherpa_onnx::Wave::read(&wav.to_string_lossy()).expect("read wav");
        assert_eq!(wave.sample_rate(), 16_000);
        let mut vad = Vad::new(&cfg.vad_model_path(), 300).unwrap();
        let mut audio = vec![0.0f32; 16_000];
        audio.extend_from_slice(wave.samples());
        let mut segs = vad.push(&audio);
        segs.extend(vad.flush());
        assert!(!segs.is_empty(), "at least one segment expected");
        let seg = &segs[0];
        let head: f32 = seg.samples[..PRE_ROLL].iter().map(|s| s * s).sum::<f32>() / PRE_ROLL as f32;
        let body: f32 = seg.samples[PRE_ROLL..].iter().map(|s| s * s).sum::<f32>() / (seg.samples.len() - PRE_ROLL) as f32;
        eprintln!("segments={} first_len={} head={head:e} body={body:e}", segs.len(), seg.samples.len());
        assert!(head < body * 0.5, "pre-roll should be quieter than the speech: head={head} body={body}");
    }
}
