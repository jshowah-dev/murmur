//! The push-to-talk key picker: press the key you want to hold to talk, Enter to keep it.

use crate::correction_ui::{hwnd_of, keycap, load_system_font, AMBER, BG, BORDER, MUTED, TEXT};
use crate::hotkey::down;
use eframe::egui::{self, CornerRadius, Frame, Key, Margin, Modifiers, RichText, Stroke, ViewportCommand};
use murmur_lib::config::{chord_vks, pickable_vks, ptt_label_of, ptt_name_of, ptt_vk_of};
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::VK_SHIFT;
use windows::Win32::UI::WindowsAndMessaging::{SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE};

const WIDTH: f32 = 380.0;
const POLL: Duration = Duration::from_millis(30);

const SHIFT_TAKEN: &str = "Shift is taken: Shift with the talk key fixes your last dictation.";
const KEY_IN_USE: &str = "That key does something in the app you're in, so it can't be the talk key.";
const WIN_ALONE: &str = "The Windows key only works together with Ctrl or Alt.";
const CHORD_KEYS: &str = "Only Ctrl, Alt and the Windows key can be held together.";

/// Follows the keys through one press: everything held between the first key going down and the
/// last one coming up, offered once they're all let go. That way Enter is never pressed with a
/// chord's modifiers still down.
#[derive(Default)]
struct Capture {
    /// False until no key is down, so a key already held when the window opens isn't picked.
    armed: bool,
    held: Vec<u16>,
}

impl Capture {
    fn step(&mut self, down: &[u16]) -> Option<Vec<u16>> {
        if down.is_empty() {
            self.armed = true;
            return (!self.held.is_empty()).then(|| std::mem::take(&mut self.held));
        }
        if self.armed {
            for vk in down {
                if !self.held.contains(vk) {
                    self.held.push(*vk);
                }
            }
        }
        None
    }

    fn held(&self) -> &[u16] {
        &self.held
    }
}

struct PickerApp {
    current: Vec<u16>,
    picked: Option<Vec<u16>>,
    refusal: Option<&'static str>,
    /// every key the picker watches: the ones that work alone, and the chord keys
    vks: Vec<u16>,
    capture: Capture,
    was_shift: bool,
    chosen: Rc<Cell<Option<Vec<u16>>>>,
    hwnd: HWND,
    frame: u32,
    was_focused: bool,
    height: f32,
}

impl eframe::App for PickerApp {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.frame += 1;
        if self.frame == 2 {
            // same as History: take focus once shown, then re-pin topmost
            unsafe {
                crate::correction::bring_to_front(self.hwnd);
                let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
        }
        // Esc or clicking another window closes it
        let focused = ctx.input(|i| i.focused);
        self.was_focused |= focused;
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) || (self.was_focused && !focused) {
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }
        let changed = self.picked.clone().filter(|vks| ptt_name_of(vks) != ptt_name_of(&self.current));
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
            self.chosen.set(changed);
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }

        // egui can't tell left Ctrl from right or see a modifier on its own, so the keys are polled
        let now: Vec<u16> = self.vks.iter().copied().filter(|vk| focused && down(*vk)).collect();
        let shift = focused && down(VK_SHIFT.0);
        let other = ctx.input(|i| {
            i.events.iter().any(|e| matches!(e, egui::Event::Key { key, pressed: true, .. } if ptt_vk_of(key.name()).is_none()))
        });
        if let Some(keys) = self.capture.step(&now) {
            if ptt_name_of(&keys).is_some() {
                self.picked = Some(keys);
                self.refusal = None;
            } else {
                self.refusal = Some(if keys.len() == 1 { WIN_ALONE } else { CHORD_KEYS });
            }
        } else if shift && !self.was_shift {
            self.refusal = Some(SHIFT_TAKEN);
        } else if other && !shift {
            self.refusal = Some(KEY_IN_USE);
        }
        self.was_shift = shift;
        ctx.request_repaint_after(POLL);

        // the keys being held show as they go down; they're kept once let go
        let keys = [self.capture.held(), self.picked.as_deref().unwrap_or(&[]), &self.current];
        let shown = keys.iter().find_map(|vks| ptt_label_of(vks)).unwrap_or_default();
        let size = if shown.contains('+') { 16.0 } else { 22.0 };
        let card = Frame::new()
            .fill(BG)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(CornerRadius::same(14))
            .inner_margin(Margin::symmetric(16, 14))
            .show(ui, |ui| {
                ui.set_width(WIDTH - 34.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Push-to-talk key").size(20.0).color(TEXT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        keycap(ui, "Esc");
                        ui.label(RichText::new("Cancel").size(12.0).color(MUTED));
                    });
                });
                ui.label(RichText::new("Press the key you want to hold to talk, or hold Ctrl, Alt or Win together.").color(MUTED));
                ui.add_space(10.0);
                Frame::new()
                    .stroke(Stroke::new(1.0, BORDER))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::symmetric(14, 8))
                    .show(ui, |ui| ui.label(RichText::new(&shown).size(size).color(TEXT)));
                ui.add_space(10.0);
                ui.horizontal(|ui| match &changed {
                    Some(_) => {
                        keycap(ui, "Enter");
                        ui.label(RichText::new(format!("Use {shown}")).color(TEXT));
                    }
                    None => {
                        ui.label(RichText::new("This is your talk key now.").color(MUTED));
                    }
                });
                if let Some(why) = self.refusal {
                    ui.label(RichText::new(why).size(12.0).color(AMBER));
                }
            });

        // Grow or shrink the window to the card so no invisible margin swallows clicks.
        let h = card.response.rect.max.y.ceil() + 1.0;
        if (h - self.height).abs() > 0.5 {
            self.height = h;
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(egui::vec2(WIDTH, h)));
        }
    }
}

/// Shows the picker, centred on screen, with `current` as the key or chord in use. Blocks until
/// closed, returning the keys to switch to if others were chosen.
pub fn show(current: Vec<u16>) -> Option<Vec<u16>> {
    let chosen = Rc::new(Cell::new(None));
    let app_chosen = chosen.clone();
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Murmur — push-to-talk key")
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_taskbar(false)
            .with_resizable(false)
            .with_inner_size([WIDTH, 180.0]),
        centered: true,
        ..Default::default()
    };
    let r = eframe::run_native(
        "murmur-ptt-key",
        opts,
        Box::new(move |cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            load_system_font(&cc.egui_ctx);
            let hwnd = hwnd_of(cc).unwrap_or_default();
            let mut vks = pickable_vks();
            vks.extend(chord_vks().into_iter().filter(|vk| !pickable_vks().contains(vk)));
            Ok(Box::new(PickerApp { current, picked: None, refusal: None, vks, capture: Capture::default(), was_shift: true, chosen: app_chosen, hwnd, frame: 0, was_focused: false, height: 0.0 }))
        }),
    );
    if let Err(e) = r {
        log::error!("ptt key window: {e}");
    }
    chosen.take()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_offered_once_it_is_let_go() {
        let mut c = Capture::default();
        assert_eq!(c.step(&[]), None);
        assert_eq!(c.step(&[0x7C]), None);
        assert_eq!(c.held(), [0x7C], "shown while it's held");
        assert_eq!(c.step(&[]), Some(vec![0x7C]));
        assert_eq!(c.step(&[]), None);
    }

    #[test]
    fn keys_held_together_are_offered_as_one_chord() {
        let mut c = Capture::default();
        c.step(&[]);
        // pressed one after the other, let go one after the other
        for down in [&[0xA2][..], &[0xA2, 0xA4], &[0xA4]] {
            assert_eq!(c.step(down), None);
        }
        assert_eq!(c.step(&[]), Some(vec![0xA2, 0xA4]));
    }

    #[test]
    fn a_key_already_held_when_the_window_opens_is_ignored() {
        let mut c = Capture::default();
        assert_eq!(c.step(&[0xA3]), None);
        assert!(c.held().is_empty());
        assert_eq!(c.step(&[]), None);
        c.step(&[0x7C]);
        assert_eq!(c.step(&[]), Some(vec![0x7C]));
    }
}
