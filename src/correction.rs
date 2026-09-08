use crate::dictionary::Dictionary;
use crate::history::History;
use crate::tray::Tray;
use std::sync::{Arc, Mutex};

pub fn fix_last(_history: &mut History, _dict: &Arc<Mutex<Dictionary>>, tray: &Tray) {
    tray.notify("Fix last", "not implemented yet");
}
