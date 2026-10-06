#![windows_subsystem = "windows"]

use murmur_lib::{audio, cleanup, config, context, dictionary, history, model_fetch, notice, snippets, stt, update, vad};
mod about_ui;
mod app;
mod audio_out;
mod autostart;
mod caret;
mod canvas;
mod correction;
mod correction_ui;
mod dictionary_panel;
mod editor;
mod editor_kit;
mod history_ui;
mod hotkey;
mod hotkey_ui;
mod inject;
#[allow(dead_code)] // generated tokens; not all are used yet
mod motion;
mod mote;
mod overlay;
mod pipeline;
mod platform;
mod setup_ui;
mod snippets_panel;
mod tray;

fn main() -> anyhow::Result<()> {
    app::main()
}
