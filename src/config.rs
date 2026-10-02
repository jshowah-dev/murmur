use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

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

/// A file's modified time, `None` when it doesn't exist. Used to notice writes by another process.
pub type Stamp = Option<SystemTime>;

#[derive(Debug, PartialEq)]
pub enum SaveOutcome {
    Saved(Stamp),
    /// The file changed since it was loaded; nothing was written.
    Conflict,
}

pub fn file_stamp(p: &Path) -> Stamp {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// Writes `contents` through a `.tmp` and a rename, keeping the previous version as `.bak`.
pub fn write_with_backup(p: &Path, contents: &str) -> Result<()> {
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if p.exists() {
        if let Err(e) = std::fs::copy(p, p.with_extension("toml.bak")) {
            log::warn!("failed to back up {name}: {e}");
        }
    }
    let tmp = p.with_extension("toml.tmp");
    std::fs::write(&tmp, contents).with_context(|| format!("write {name}.tmp"))?;
    std::fs::rename(&tmp, p).with_context(|| format!("rename {name}.tmp"))
}

/// Writes only if the file is still the version stamped at load (or at the last save).
pub fn write_if_unchanged(p: &Path, stamp: Stamp, contents: &str) -> Result<SaveOutcome> {
    if file_stamp(p) != stamp {
        return Ok(SaveOutcome::Conflict);
    }
    write_with_backup(p, contents)?;
    Ok(SaveOutcome::Saved(file_stamp(p)))
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

const VK_RCONTROL: u16 = 0xA3;
const VK_RSHIFT: u16 = 0xA1;
const VK_LWIN: u16 = 0x5B;
const VK_RWIN: u16 = 0x5C;
const VK_F1: u16 = 0x70;

/// (config name, virtual-key code, label) of the PTT keys that aren't function keys. None of them
/// types anything: the key is watched, not swallowed, so it still reaches the app in front.
/// A chord is named and labelled in this order.
const PTT_KEYS: [(&str, u16, &str); 10] = [
    ("RControl", VK_RCONTROL, "Right Ctrl"),
    ("LControl", 0xA2, "Left Ctrl"),
    ("RAlt", 0xA5, "Right Alt"),
    ("LAlt", 0xA4, "Left Alt"),
    ("RShift", VK_RSHIFT, "Right Shift"),
    ("CapsLock", 0x14, "Caps Lock"),
    ("ScrollLock", 0x91, "Scroll Lock"),
    ("Pause", 0x13, "Pause"),
    ("LWin", VK_LWIN, "Left Win"),
    ("RWin", VK_RWIN, "Right Win"),
];

/// Other spellings config.toml accepts.
const PTT_ALIASES: [(&str, &str); 4] = [("RCtrl", "RControl"), ("LCtrl", "LControl"), ("RMenu", "RAlt"), ("LMenu", "LAlt")];

/// Virtual-key code for one key name from config.toml, any case; F1 to F24 included.
pub fn ptt_vk_of(name: &str) -> Option<u16> {
    let name = PTT_ALIASES.iter().find(|(a, _)| a.eq_ignore_ascii_case(name)).map_or(name, |(_, n)| n);
    if let Some((_, vk, _)) = PTT_KEYS.iter().find(|(n, _, _)| n.eq_ignore_ascii_case(name)) {
        return Some(*vk);
    }
    let n: u16 = name.strip_prefix(['f', 'F'])?.parse().ok()?;
    (1..=24).contains(&n).then(|| VK_F1 + n - 1)
}

/// The keys of a `ptt_key` setting: one key, or a chord like "LControl+LAlt" that is held together.
pub fn ptt_vks_of(name: &str) -> Option<Vec<u16>> {
    let vks = name.split('+').map(|part| ptt_vk_of(part.trim())).collect::<Option<Vec<u16>>>()?;
    usable(&vks)
}

/// The name config.toml stores for a PTT key or chord.
pub fn ptt_name_of(vks: &[u16]) -> Option<String> {
    Some(usable(vks)?.iter().filter_map(|vk| ptt_key_of(*vk)).map(|(name, _)| name).collect::<Vec<_>>().join("+"))
}

/// A PTT key or chord as a person would name it ("Right Ctrl", "Left Ctrl + Left Alt").
pub fn ptt_label_of(vks: &[u16]) -> Option<String> {
    Some(usable(vks)?.iter().filter_map(|vk| ptt_key_of(*vk)).map(|(_, label)| label).collect::<Vec<_>>().join(" + "))
}

/// `vks` without repeats and in the order they're named, if they can be a PTT key: a known key
/// other than Win on its own, or several of Ctrl, Alt and Win.
fn usable(vks: &[u16]) -> Option<Vec<u16>> {
    let mut vks = vks.to_vec();
    if vks.iter().any(|vk| ptt_key_of(*vk).is_none()) {
        return None;
    }
    vks.sort_by_key(|vk| PTT_KEYS.iter().position(|(_, v, _)| v == vk));
    vks.dedup();
    let chord = chord_vks();
    match vks.as_slice() {
        [] => None,
        // Win on its own opens Start when it's let go
        [vk] => (!matches!(*vk, VK_LWIN | VK_RWIN)).then_some(vks),
        many => many.iter().all(|vk| chord.contains(vk)).then_some(vks),
    }
}

fn ptt_key_of(vk: u16) -> Option<(String, String)> {
    if let Some((name, _, label)) = PTT_KEYS.iter().find(|(_, v, _)| *v == vk) {
        return Some((name.to_string(), label.to_string()));
    }
    (VK_F1..VK_F1 + 24).contains(&vk).then(|| {
        let f = format!("F{}", vk - VK_F1 + 1);
        (f.clone(), f)
    })
}

/// The keys the picker offers on their own. Right Shift isn't one: Shift with the PTT key means
/// fix-last, so it could never start a dictation.
pub fn pickable_vks() -> Vec<u16> {
    let alone = |vk: &u16| !matches!(*vk, VK_RSHIFT | VK_LWIN | VK_RWIN);
    PTT_KEYS.iter().map(|(_, vk, _)| *vk).filter(alone).chain(VK_F1..VK_F1 + 24).collect()
}

/// The keys that can be held together as a chord: Ctrl, Alt and Win, either side.
pub fn chord_vks() -> Vec<u16> {
    vec![VK_RCONTROL, 0xA2, 0xA5, 0xA4, VK_LWIN, VK_RWIN]
}

/// `text` (a config.toml) with its `ptt_key` set to `name`. Only that line changes, and it keeps
/// its trailing comment; a file without the line gets it at the top.
pub fn set_ptt_key_line(text: &str, name: &str) -> String {
    let set = format!("ptt_key = \"{name}\"");
    let mut out = String::with_capacity(text.len() + set.len() + 1);
    let mut done = false;
    let mut in_table = false;
    for line in text.split_inclusive('\n') {
        in_table |= line.trim_start().starts_with('[');
        let value = line.trim_start().strip_prefix("ptt_key").map(str::trim_start).and_then(|r| r.strip_prefix('='));
        match value {
            Some(value) if !done && !in_table => {
                let value = value.trim_start();
                // what follows the quoted value: a comment, the line ending
                let tail = value.chars().next().filter(|q| matches!(q, '"' | '\'')).and_then(|q| value[1..].find(q).map(|end| &value[end + 2..]));
                out.push_str(&set);
                out.push_str(tail.unwrap_or(&line[line.trim_end_matches(['\r', '\n']).len()..]));
                done = true;
            }
            _ => out.push_str(line),
        }
    }
    if done { out } else { format!("{set}\n{out}") }
}

/// Stores `name` as the PTT key in config.toml, leaving the rest of the file as it was written.
pub fn save_ptt_key(name: &str) -> Result<()> {
    let path = config_dir().join("config.toml");
    let text = if path.exists() { std::fs::read_to_string(&path).context("read config.toml")? } else { String::new() };
    write_with_backup(&path, &set_ptt_key_line(&text, name))
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

    /// Virtual-key codes for the configured PTT key or chord; an unusable setting means Right Ctrl.
    pub fn ptt_vks(&self) -> Vec<u16> {
        ptt_vks_of(&self.ptt_key).unwrap_or_else(|| vec![VK_RCONTROL])
    }

    /// The PTT key as a person would name it ("Right Ctrl"), for on-screen instructions.
    pub fn ptt_key_label(&self) -> String {
        ptt_label_of(&self.ptt_vks()).unwrap_or_default()
    }

    /// Length cap for a hands-free recording; at least one minute.
    pub fn hands_free_max(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.hands_free_max_minutes.max(1) * 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backup_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("murmur-config-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn write_with_backup_keeps_the_previous_version_and_no_tmp() {
        let p = backup_dir("backup").join("x.toml");
        write_with_backup(&p, "one").unwrap();
        assert!(!p.with_extension("toml.bak").exists(), "nothing to back up on the first write");
        write_with_backup(&p, "two").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "two");
        assert_eq!(std::fs::read_to_string(p.with_extension("toml.bak")).unwrap(), "one");
        assert!(!p.with_extension("toml.tmp").exists());
    }

    #[test]
    fn write_if_unchanged_saves_on_a_matching_stamp_and_conflicts_after_an_outside_write() {
        let p = backup_dir("stamp").join("x.toml");
        assert!(matches!(write_if_unchanged(&p, None, "new").unwrap(), SaveOutcome::Saved(Some(_))), "a missing file saves");
        let stamp = file_stamp(&p);
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&p, "outside").unwrap();
        assert_eq!(write_if_unchanged(&p, stamp, "mine").unwrap(), SaveOutcome::Conflict);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "outside");
    }

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
        assert_eq!(c.ptt_vks(), [0xA3]); // VK_RCONTROL
        c.ptt_key = "F13".into();
        assert_eq!(c.ptt_vks(), [0x7C]);
        c.ptt_key = "CapsLock".into();
        assert_eq!(c.ptt_vks(), [0x14]);
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

    #[test]
    fn key_names_vks_and_labels_agree() {
        for (name, vk, label) in [
            ("RControl", 0xA3, "Right Ctrl"),
            ("LAlt", 0xA4, "Left Alt"),
            ("Pause", 0x13, "Pause"),
            ("F1", 0x70, "F1"),
            ("F13", 0x7C, "F13"),
            ("F24", 0x87, "F24"),
        ] {
            assert_eq!(ptt_vk_of(name), Some(vk), "{name}");
            assert_eq!(ptt_name_of(&[vk]).as_deref(), Some(name));
            assert_eq!(ptt_label_of(&[vk]).as_deref(), Some(label));
        }
        assert_eq!(ptt_vk_of("rctrl"), Some(0xA3));
        assert_eq!(ptt_vk_of("LMENU"), Some(0xA4));
        for unknown in ["nonsense", "f", "F0", "F25", ""] {
            assert_eq!(ptt_vk_of(unknown), None, "{unknown}");
        }
        assert_eq!(ptt_name_of(&[0x41]), None);
        assert_eq!(ptt_name_of(&[]), None);
    }

    #[test]
    fn right_shift_parses_but_cannot_be_picked() {
        assert_eq!(ptt_vk_of("RShift"), Some(0xA1));
        let vks = pickable_vks();
        assert!(!vks.contains(&0xA1));
        assert_eq!(vks.len(), 7 + 24);
        for vk in vks {
            let name = ptt_name_of(&[vk]).expect("a pickable key has a config name");
            assert_eq!(ptt_vk_of(&name), Some(vk));
        }
    }

    #[test]
    fn modifiers_combine_into_a_chord() {
        assert_eq!(ptt_vks_of("LControl+LAlt"), Some(vec![0xA2, 0xA4]));
        assert_eq!(ptt_vks_of("lwin + lctrl"), Some(vec![0xA2, 0x5B]), "any case, spaces, in the stored order");
        assert_eq!(ptt_vks_of("LControl+LControl"), Some(vec![0xA2]));
        assert_eq!(ptt_name_of(&[0x5B, 0xA2]).as_deref(), Some("LControl+LWin"));
        assert_eq!(ptt_label_of(&[0x5B, 0xA2]).as_deref(), Some("Left Ctrl + Left Win"));
        assert_eq!(ptt_label_of(&[0xA5, 0xA3, 0x5C]).as_deref(), Some("Right Ctrl + Right Alt + Right Win"));
    }

    #[test]
    fn only_ctrl_alt_and_win_combine_and_win_is_never_alone() {
        for bad in ["LWin", "RWin", "F13+LControl", "CapsLock+LAlt", "RShift+LControl", "LControl+", "+", "LControl+nonsense"] {
            assert_eq!(ptt_vks_of(bad), None, "{bad}");
        }
        assert_eq!(ptt_name_of(&[0x5B]), None);
        assert_eq!(ptt_name_of(&[0x7C, 0xA2]), None);
        assert!(!pickable_vks().contains(&0x5B));
        assert_eq!(chord_vks(), [0xA3, 0xA2, 0xA5, 0xA4, 0x5B, 0x5C]);
    }

    #[test]
    fn a_chord_in_config_is_used_and_a_bad_one_falls_back_to_right_ctrl() {
        let mut c = Config::default();
        c.ptt_key = "LControl+LAlt".into();
        assert_eq!(c.ptt_vks(), [0xA2, 0xA4]);
        assert_eq!(c.ptt_key_label(), "Left Ctrl + Left Alt");
        c.ptt_key = "LWin".into();
        assert_eq!(c.ptt_vks(), [0xA3]);
        assert_eq!(c.ptt_key_label(), "Right Ctrl");
        let text = set_ptt_key_line("ptt_key = \"RControl\"
", "LControl+LWin");
        assert_eq!(toml::from_str::<Config>(&text).unwrap().ptt_vks(), [0xA2, 0x5B]);
    }

    #[test]
    fn setting_the_key_changes_only_its_line() {
        let before = "# my notes\r\nptt_key = \"RControl\"  # the talk key\r\nthreads = 4\r\n# ptt_key = \"F13\"\r\n";
        let after = "# my notes\r\nptt_key = \"F13\"  # the talk key\r\nthreads = 4\r\n# ptt_key = \"F13\"\r\n";
        assert_eq!(set_ptt_key_line(before, "F13"), after);
        assert_eq!(set_ptt_key_line("ptt_key='CapsLock'", "LAlt"), "ptt_key = \"LAlt\"");
    }

    #[test]
    fn setting_the_key_adds_a_missing_line_at_the_top() {
        assert_eq!(set_ptt_key_line("threads = 4\n", "F13"), "ptt_key = \"F13\"\nthreads = 4\n");
        assert_eq!(set_ptt_key_line("", "F13"), "ptt_key = \"F13\"\n");
        // another key that merely starts the same is not the setting
        assert_eq!(set_ptt_key_line("ptt_key_old = 1\n", "F13"), "ptt_key = \"F13\"\nptt_key_old = 1\n");
    }

    #[test]
    fn the_rewritten_file_still_parses_to_the_new_key() {
        let text = set_ptt_key_line(&toml::to_string_pretty(&Config::default()).unwrap(), "CapsLock");
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.ptt_key, "CapsLock");
        assert_eq!(back.threads, 8);
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
