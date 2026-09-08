#![allow(dead_code)]
use anyhow::{anyhow, Result};
use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig};
use std::path::Path;
use std::time::Instant;

pub struct Recognizer {
    inner: OfflineRecognizer,
}

impl Recognizer {
    pub fn load(dir: &Path, threads: i32) -> Result<Recognizer> {
        for f in ["encoder.int8.onnx", "decoder.int8.onnx", "joiner.int8.onnx", "tokens.txt"] {
            if !dir.join(f).exists() {
                return Err(anyhow!("model file missing: {} (run setup-model.cmd)", dir.join(f).display()));
            }
        }
        let p = |f: &str| Some(dir.join(f).to_string_lossy().into_owned());
        let mut config = OfflineRecognizerConfig::default();
        config.model_config.transducer.encoder = p("encoder.int8.onnx");
        config.model_config.transducer.decoder = p("decoder.int8.onnx");
        config.model_config.transducer.joiner = p("joiner.int8.onnx");
        config.model_config.tokens = p("tokens.txt");
        config.model_config.model_type = Some("nemo_transducer".into());
        config.model_config.num_threads = threads;
        config.model_config.provider = Some("cpu".into());
        let t0 = Instant::now();
        let inner = OfflineRecognizer::create(&config).ok_or_else(|| anyhow!("create recognizer from {}", dir.display()))?;
        log::info!("model loaded in {:.2}s", t0.elapsed().as_secs_f32());
        Ok(Recognizer { inner })
    }

    pub fn transcribe(&self, samples: &[f32]) -> String {
        if samples.is_empty() {
            return String::new();
        }
        let t0 = Instant::now();
        let stream = self.inner.create_stream();
        stream.accept_waveform(16_000, samples);
        self.inner.decode(&stream);
        let text = stream.get_result().map(|r| r.text).unwrap_or_default();
        log::debug!("transcribed {:.2}s audio in {:.3}s: {text}", samples.len() as f32 / 16_000.0, t0.elapsed().as_secs_f32());
        text.trim().to_string()
    }
}
