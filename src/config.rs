use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub ptt_key: String,
    pub model_dir: String,
    pub idle_unload_minutes: u64,
    pub threads: i32,
    pub min_silence_ms: u32,
    pub fillers: Vec<String>,
    pub spoken_commands: bool,
    /// Keep the mic open between dictations so the first consonant is not lost to device
    /// start-up. It closes after `idle_unload_minutes` without a dictation. false = open on press.
    pub mic_always_on: bool,
    /// Mute the default speaker while the PTT key is held (Spotify, YouTube ...).
    pub mute_output: bool,
    /// Hands-free (double-tap) recordings stop and paste after this many minutes.
    pub hands_free_max_minutes: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ptt_key: "RControl".into(),
            model_dir: "%LOCALAPPDATA%\\Murmur\\models\\sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8".into(),
            idle_unload_minutes: 15,
            threads: 8,
            min_silence_ms: 500,
            fillers: ["um", "uh", "er", "hmm", "mm"].iter().map(|s| s.to_string()).collect(),
            spoken_commands: true,
            mic_always_on: true,
            mute_output: true,
            hands_free_max_minutes: 5,
        }
    }
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir().expect("APPDATA").join("Murmur")
}

/// Expand `%VAR%` segments using the process environment. Unknown vars are left as-is.
pub fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(v) => out.push_str(&v),
                    Err(_) => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

impl Config {
    pub fn load_or_create() -> Result<Config> {
        let dir = config_dir();
        std::fs::create_dir_all(&dir).context("create config dir")?;
        let path = dir.join("config.toml");
        if path.exists() {
            let text = std::fs::read_to_string(&path).context("read config.toml")?;
            Ok(toml::from_str(&text).context("parse config.toml")?)
        } else {
            let c = Config::default();
            std::fs::write(&path, toml::to_string_pretty(&c)?).context("write config.toml")?;
            Ok(c)
        }
    }

    pub fn model_dir_path(&self) -> PathBuf {
        PathBuf::from(expand_env(&self.model_dir))
    }

    pub fn vad_model_path(&self) -> PathBuf {
        self.model_dir_path()
            .parent()
            .map(|p| p.join("silero_vad.onnx"))
            .unwrap_or_else(|| PathBuf::from("silero_vad.onnx"))
    }

    /// Virtual-key code for the configured PTT key.
    pub fn ptt_vk(&self) -> u16 {
        match self.ptt_key.to_ascii_lowercase().as_str() {
            "rcontrol" | "rctrl" => 0xA3,
            "lcontrol" | "lctrl" => 0xA2,
            "ralt" | "rmenu" => 0xA5,
            "lalt" | "lmenu" => 0xA4,
            "rshift" => 0xA1,
            "capslock" => 0x14,
            "scrolllock" => 0x91,
            "pause" => 0x13,
            k if k.starts_with('f') && k[1..].parse::<u16>().map(|n| (1..=24).contains(&n)).unwrap_or(false) => {
                0x70 + k[1..].parse::<u16>().unwrap() - 1
            }
            _ => 0xA3,
        }
    }

    /// Length cap for a hands-free recording; at least one minute.
    pub fn hands_free_max(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.hands_free_max_minutes.max(1) * 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_spec() {
        let c = Config::default();
        assert_eq!(c.ptt_key, "RControl");
        assert_eq!(c.idle_unload_minutes, 15);
        assert_eq!(c.threads, 8);
        assert_eq!(c.min_silence_ms, 500);
        assert!(c.spoken_commands);
        assert_eq!(c.fillers, vec!["um", "uh", "er", "hmm", "mm"]);
        assert_eq!(c.hands_free_max_minutes, 5);
    }

    #[test]
    fn expands_env_vars() {
        std::env::set_var("MURMUR_TEST_X", "abc");
        assert_eq!(expand_env("%MURMUR_TEST_X%\\y"), "abc\\y");
        assert_eq!(expand_env("plain"), "plain");
    }

    #[test]
    fn ptt_key_names_map_to_vk() {
        let mut c = Config::default();
        assert_eq!(c.ptt_vk(), 0xA3); // VK_RCONTROL
        c.ptt_key = "F13".into();
        assert_eq!(c.ptt_vk(), 0x7C);
        c.ptt_key = "CapsLock".into();
        assert_eq!(c.ptt_vk(), 0x14);
    }

    #[test]
    fn round_trips_through_toml() {
        let c = Config::default();
        let s = toml::to_string(&c).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(back.ptt_key, c.ptt_key);
        assert_eq!(back.model_dir, c.model_dir);
    }
}
