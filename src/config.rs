use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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
    /// Also log dictated text (DEBUG level). Off by default for privacy.
    pub debug_log: bool,
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
            debug_log: false,
        }
    }
}

/// `model_dir` defaults of earlier releases, newest first. When a release pins a new model, the
/// default it replaces goes here, so installs still on it are offered the new one.
pub const PREVIOUS_DEFAULTS: &[&str] = &[];

#[derive(Debug, PartialEq)]
pub enum ModelState {
    Current,
    /// The new default model is installed; `old` is the previous default's folder, now unused.
    Switched { old: PathBuf },
    /// Still on a previous default; the current default can be downloaded.
    UpgradeAvailable,
}

/// Moves a config on a previous default model to the current default once that's installed.
/// Only `cfg` in memory changes: config.toml keeps its comments and hand edits.
pub fn resolve_model(cfg: &mut Config, installed: impl Fn(&Path) -> bool) -> ModelState {
    resolve_with(cfg, PREVIOUS_DEFAULTS, installed)
}

fn resolve_with(cfg: &mut Config, previous: &[&str], installed: impl Fn(&Path) -> bool) -> ModelState {
    if !previous.contains(&cfg.model_dir.as_str()) {
        return ModelState::Current;
    }
    let old = cfg.model_dir_path();
    let current = Config::default();
    if installed(&current.model_dir_path()) {
        cfg.model_dir = current.model_dir;
        ModelState::Switched { old }
    } else if installed(&old) {
        ModelState::UpgradeAvailable
    } else {
        cfg.model_dir = current.model_dir;
        ModelState::Current
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

    /// The PTT key as a person would name it ("Right Ctrl"), for on-screen instructions.
    pub fn ptt_key_label(&self) -> String {
        match self.ptt_vk() {
            0xA2 => "Left Ctrl".into(),
            0xA5 => "Right Alt".into(),
            0xA4 => "Left Alt".into(),
            0xA1 => "Right Shift".into(),
            0x14 => "Caps Lock".into(),
            0x91 => "Scroll Lock".into(),
            0x13 => "Pause".into(),
            vk @ 0x70..=0x87 => format!("F{}", vk - 0x70 + 1),
            _ => "Right Ctrl".into(),
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
        assert!(!c.debug_log);
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
    fn ptt_key_labels_read_like_the_keyboard() {
        let mut c = Config::default();
        assert_eq!(c.ptt_key_label(), "Right Ctrl");
        c.ptt_key = "lalt".into();
        assert_eq!(c.ptt_key_label(), "Left Alt");
        c.ptt_key = "F13".into();
        assert_eq!(c.ptt_key_label(), "F13");
        c.ptt_key = "CapsLock".into();
        assert_eq!(c.ptt_key_label(), "Caps Lock");
        // an unknown name falls back to Right Ctrl, the same key ptt_vk falls back to
        c.ptt_key = "nonsense".into();
        assert_eq!(c.ptt_key_label(), "Right Ctrl");
    }

    const OLD: &str = "%LOCALAPPDATA%\\Murmur\\models\\old-model";

    fn cfg_with(dir: &str) -> Config {
        Config { model_dir: dir.into(), ..Config::default() }
    }

    #[test]
    fn a_custom_or_current_model_dir_is_left_alone() {
        for dir in ["D:\\models\\mine", &Config::default().model_dir] {
            let mut c = cfg_with(dir);
            assert_eq!(resolve_with(&mut c, &[OLD], |_| true), ModelState::Current);
            assert_eq!(c.model_dir, dir);
        }
    }

    #[test]
    fn an_old_default_switches_once_the_new_model_is_installed() {
        let current = Config::default().model_dir_path();
        let mut c = cfg_with(OLD);
        let state = resolve_with(&mut c, &[OLD], |p| p == current.as_path());
        assert_eq!(state, ModelState::Switched { old: PathBuf::from(expand_env(OLD)) });
        assert_eq!(c.model_dir, Config::default().model_dir);
    }

    #[test]
    fn an_old_default_is_offered_the_new_model() {
        let old = PathBuf::from(expand_env(OLD));
        let mut c = cfg_with(OLD);
        assert_eq!(resolve_with(&mut c, &[OLD], |p| p == old.as_path()), ModelState::UpgradeAvailable);
        assert_eq!(c.model_dir, OLD);
    }

    #[test]
    fn an_old_default_that_is_gone_gets_the_new_default() {
        let mut c = cfg_with(OLD);
        assert_eq!(resolve_with(&mut c, &[OLD], |_| false), ModelState::Current);
        assert_eq!(c.model_dir, Config::default().model_dir);
    }

    #[test]
    fn no_previous_defaults_ship_yet() {
        assert!(PREVIOUS_DEFAULTS.is_empty());
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
