use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use crossbeam_channel::Sender;
use sherpa_onnx::LinearResampler;

pub const TARGET_RATE: u32 = 16_000;

pub struct Capture {
    _stream: cpal::Stream,
}

pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

impl Capture {
    /// Opens the default input device and streams 16 kHz mono f32 chunks to `tx` until dropped.
    pub fn start(tx: Sender<Vec<f32>>) -> Result<Capture> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| anyhow!("no default input device"))?;
        let supported = device.default_input_config().context("default input config")?;
        let config = supported.config();
        let channels = config.channels as usize;
        let rate = config.sample_rate;
        let resampler = if rate != TARGET_RATE {
            Some(
                LinearResampler::create(rate as i32, TARGET_RATE as i32)
                    .ok_or_else(|| anyhow!("resampler"))?,
            )
        } else {
            None
        };
        // cpal 0.18 reports capture overruns, which come with CPU load and aren't fatal
        let err_fn = |e: cpal::Error| match e.kind() {
            cpal::ErrorKind::Xrun => log::warn!("audio overrun (samples dropped)"),
            _ => log::error!("audio stream error: {e:?}"),
        };

        macro_rules! build {
            ($t:ty, $conv:expr) => {{
                let tx = tx.clone();
                device.build_input_stream(
                    config,
                    move |data: &[$t], _: &cpal::InputCallbackInfo| {
                        if data.is_empty() {
                            return;
                        }
                        let mono: Vec<f32> = data
                            .chunks(channels)
                            .map(|f| f.iter().map(|&s| $conv(s)).sum::<f32>() / channels as f32)
                            .collect();
                        let out = match &resampler {
                            Some(r) => r.resample(&mono, false),
                            None => mono,
                        };
                        let _ = tx.send(out);
                    },
                    err_fn,
                    None,
                )?
            }};
        }

        let stream = match supported.sample_format() {
            SampleFormat::F32 => build!(f32, |s: f32| s),
            SampleFormat::I16 => build!(i16, |s: i16| s as f32 / i16::MAX as f32),
            SampleFormat::U16 => build!(u16, |s: u16| (s as f32 - 32768.0) / 32768.0),
            other => return Err(anyhow!("unsupported sample format {other:?}")),
        };
        stream.play().context("start input stream")?;
        Ok(Capture { _stream: stream })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rms_of_silence_is_zero_and_of_ones_is_one() {
        assert_eq!(rms(&[0.0; 8]), 0.0);
        assert!((rms(&[1.0, -1.0, 1.0, -1.0]) - 1.0).abs() < 1e-6);
        assert_eq!(rms(&[]), 0.0);
    }
}
