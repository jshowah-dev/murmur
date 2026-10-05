#![windows_subsystem = "windows"]

use murmur_lib::{cleanup, config, context, dictionary, history, notice, snippets, stt, vad};
#[cfg(windows)]
use murmur_lib::{audio, model_fetch, update};

#[cfg(windows)]
mod about_ui;
#[cfg(windows)]
mod app;
#[cfg(windows)]
mod audio_out;
#[cfg(windows)]
mod autostart;
#[cfg(windows)]
mod caret;
#[cfg(windows)]
mod canvas;
#[cfg(windows)]
mod correction;
#[cfg(windows)]
mod correction_ui;
#[cfg(windows)]
mod dictionary_panel;
#[cfg(windows)]
mod editor;
#[cfg(windows)]
mod editor_kit;
#[cfg(windows)]
mod history_ui;
#[cfg(windows)]
mod hotkey;
#[cfg(windows)]
mod hotkey_ui;
mod inject;
#[cfg(windows)]
#[allow(dead_code)] // generated tokens; not all are used yet
mod motion;
#[cfg(windows)]
mod mote;
#[cfg(windows)]
mod overlay;
mod pipeline;
#[cfg(windows)]
mod setup_ui;
#[cfg(windows)]
mod snippets_panel;
#[cfg(not(windows))]
mod spike;
#[cfg(windows)]
mod tray;

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    app::main()
}

/// The macOS port is under way: until the tray app runs there, the binary is a console spike.
#[cfg(not(windows))]
fn main() -> anyhow::Result<()> {
    spike::main()
}
