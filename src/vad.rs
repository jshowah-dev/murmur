use anyhow::{anyhow, Result};
use sherpa_onnx::{VadModelConfig, VoiceActivityDetector};
use std::path::Path;

pub const WINDOW: usize = 512;

pub struct Segment {
    pub samples: Vec<f32>,
}

pub struct Vad {
    inner: VoiceActivityDetector,
    pending: Vec<f32>,
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
        Ok(Vad { inner, pending: Vec::new() })
    }

    fn drain(&mut self) -> Vec<Segment> {
        let mut out = Vec::new();
        while let Some(seg) = self.inner.front() {
            out.push(Segment { samples: seg.samples().to_vec() });
            self.inner.pop();
        }
        out
    }

    pub fn push(&mut self, samples: &[f32]) -> Vec<Segment> {
        self.pending.extend_from_slice(samples);
        let mut offset = 0;
        while offset + WINDOW <= self.pending.len() {
            self.inner.accept_waveform(&self.pending[offset..offset + WINDOW]);
            offset += WINDOW;
        }
        self.pending.drain(..offset);
        self.drain()
    }

    pub fn flush(&mut self) -> Vec<Segment> {
        if !self.pending.is_empty() {
            let mut tail = std::mem::take(&mut self.pending);
            tail.resize(WINDOW, 0.0);
            self.inner.accept_waveform(&tail);
        }
        self.inner.flush();
        self.drain()
    }

    pub fn reset(&mut self) {
        self.pending.clear();
        self.inner.reset();
        let _ = self.drain();
    }
}
