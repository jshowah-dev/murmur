//! The push-to-talk key picker: press the key you want to hold to talk, Enter to keep it.

use crate::correction_ui::{hwnd_of, keycap, load_system_font, AMBER, BG, BORDER, MUTED, TEXT};
use crate::hotkey::down;
use eframe::egui::{self, CornerRadius, Frame, Key, Margin, Modifiers, RichText, Stroke, ViewportCommand};
use murmur_lib::config::{pickable_vks, ptt_label_of, ptt_vk_of};
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

/// The key in `vks` that went down between two polls, if one did.
fn newly_down(was: &[bool], now: &[bool], vks: &[u16]) -> Option<u16> {
    vks.iter().zip(was.iter().zip(now)).find(|(_, (was, now))| !**was && **now).map(|(vk, _)| *vk)
}

struct PickerApp {
    current: u16,
    picked: Option<u16>,
    refusal: Option<&'static str>,
    vks: Vec<u16>,
    /// Which of `vks` were down at the last poll. All true at first, so a key already held when
    /// the window opens isn't picked.
    was: Vec<bool>,
    was_shift: bool,
    chosen: Rc<Cell<Option<u16>>>,
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
        let changed = self.picked.filter(|vk| *vk != self.current);
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
            self.chosen.set(changed);
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }

        // egui can't tell left Ctrl from right or see a modifier on its own, so the keys are polled
        let now: Vec<bool> = self.vks.iter().map(|vk| focused && down(*vk)).collect();
        let shift = focused && down(VK_SHIFT.0);
        let other = ctx.input(|i| {
            i.events.iter().any(|e| matches!(e, egui::Event::Key { key, pressed: true, .. } if ptt_vk_of(key.name()).is_none()))
        });
        if let Some(vk) = newly_down(&self.was, &now, &self.vks) {
            self.picked = Some(vk);
            self.refusal = None;
        } else if shift && !self.was_shift {
            self.refusal = Some(SHIFT_TAKEN);
        } else if other && !shift {
            self.refusal = Some(KEY_IN_USE);
        }
        self.was = now;
        self.was_shift = shift;
        ctx.request_repaint_after(POLL);

        let shown = ptt_label_of(self.picked.unwrap_or(self.current)).unwrap_or_default();
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
                ui.label(RichText::new("Press the key you want to hold to talk.").color(MUTED));
                ui.add_space(10.0);
                Frame::new()
                    .stroke(Stroke::new(1.0, BORDER))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::symmetric(14, 8))
                    .show(ui, |ui| ui.label(RichText::new(&shown).size(22.0).color(TEXT)));
                ui.add_space(10.0);
                ui.horizontal(|ui| match changed {
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

/// Shows the picker, centred on screen, with `current` as the key in use. Blocks until closed,
/// returning the key to switch to if another one was chosen.
pub fn show(current: u16) -> Option<u16> {
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
            let vks = pickable_vks();
            let was = vec![true; vks.len()];
            Ok(Box::new(PickerApp { current, picked: None, refusal: None, vks, was, was_shift: true, chosen: app_chosen, hwnd, frame: 0, was_focused: false, height: 0.0 }))
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
    fn a_key_is_picked_when_it_goes_down_not_while_it_stays_down() {
        let vks = [0xA3, 0x7C];
        assert_eq!(newly_down(&[false, false], &[false, true], &vks), Some(0x7C));
        assert_eq!(newly_down(&[false, true], &[false, true], &vks), None);
        assert_eq!(newly_down(&[true, true], &[false, false], &vks), None);
    }
}
