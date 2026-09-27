use crate::correction_ui::{hwnd_of, keycap, load_system_font, BG, BORDER, GREEN, MUTED, TEXT};
use eframe::egui::{
    self, text::LayoutJob, text::TextFormat, Color32, CornerRadius, FontId, Frame, Key, Margin, Modifiers, Stroke,
    ViewportCommand,
};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE};

const WIDTH: f32 = 540.0;
const MAX_LIST: f32 = 460.0;
const HOVER: Color32 = Color32::from_rgb(0x2A, 0x2A, 0x2A);
const FLASH: Duration = Duration::from_millis(1000);

fn ago(secs: u64) -> String {
    match secs {
        0..5 => "just now".into(),
        5..60 => format!("{secs} s ago"),
        60..3600 => format!("{} min ago", secs / 60),
        _ => format!("{} h ago", secs / 3600),
    }
}

struct HistoryApp {
    items: Vec<(String, Instant)>,
    hwnd: HWND,
    frame: u32,
    was_focused: bool,
    copied: Option<(usize, Instant)>,
    height: f32,
}

impl eframe::App for HistoryApp {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.frame += 1;
        if self.frame == 2 {
            // same as the fix dialog: take focus once shown, then re-pin topmost
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

        let flashing = self.copied.filter(|(_, at)| at.elapsed() < FLASH);
        if let Some((_, at)) = flashing {
            // one wake-up to clear the flash; never a repaint loop (it starves the overlay and tray)
            ctx.request_repaint_after(FLASH.saturating_sub(at.elapsed()));
        }

        let card = Frame::new()
            .fill(BG)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(CornerRadius::same(14))
            .inner_margin(Margin::symmetric(10, 12))
            .show(ui, |ui| {
                ui.set_width(WIDTH - 22.0);
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    let (dot, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                    ui.painter().circle_filled(dot.center(), 4.0, GREEN);
                    ui.label(egui::RichText::new("History · click to copy").size(12.0).color(MUTED));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(6.0);
                        keycap(ui, "Esc");
                        ui.label(egui::RichText::new("Close").size(12.0).color(MUTED));
                    });
                });
                ui.add_space(6.0);
                egui::ScrollArea::vertical().max_height(MAX_LIST).auto_shrink([false, true]).show(ui, |ui| {
                    for (i, (text, at)) in self.items.iter().enumerate() {
                        let id = ui.id().with(("row", i));
                        let hovered = ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
                        let row = Frame::new()
                            .fill(if hovered { HOVER } else { Color32::TRANSPARENT })
                            .corner_radius(CornerRadius::same(8))
                            .inner_margin(Margin::symmetric(8, 6))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(ago(at.elapsed().as_secs())).size(11.0).color(MUTED));
                                    if flashing.is_some_and(|(k, _)| k == i) {
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            ui.label(egui::RichText::new("Copied").size(11.0).color(GREEN));
                                        });
                                    }
                                });
                                let mut job = LayoutJob::single_section(
                                    text.clone(),
                                    TextFormat { font_id: FontId::proportional(15.0), color: TEXT, ..Default::default() },
                                );
                                job.wrap.max_width = ui.available_width();
                                job.wrap.max_rows = 3;
                                job.wrap.overflow_character = Some('…');
                                ui.add(egui::Label::new(job).selectable(false));
                            })
                            .response
                            .interact(egui::Sense::click());
                        if row.hovered() != hovered {
                            ctx.data_mut(|d| d.insert_temp(id, row.hovered()));
                            ctx.request_repaint();
                        }
                        if row.clicked() {
                            match crate::inject::set_clipboard_text(text) {
                                Ok(()) => self.copied = Some((i, Instant::now())),
                                Err(e) => log::error!("history copy: {e}"),
                            }
                            ctx.request_repaint();
                        }
                    }
                });
            });

        // Grow or shrink the window to the card so no invisible margin swallows clicks.
        let h = card.response.rect.max.y.ceil() + 1.0;
        if (h - self.height).abs() > 0.5 {
            self.height = h;
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(egui::vec2(WIDTH, h)));
        }
    }
}

/// Shows recent dictations, newest first, centred on screen. Blocks until closed.
pub fn show(items: Vec<(String, Instant)>) {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Murmur — history")
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_taskbar(false)
            .with_resizable(false)
            .with_inner_size([WIDTH, 200.0]),
        centered: true,
        ..Default::default()
    };
    let r = eframe::run_native(
        "murmur-history",
        opts,
        Box::new(move |cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            load_system_font(&cc.egui_ctx);
            let hwnd = hwnd_of(cc).unwrap_or_default();
            Ok(Box::new(HistoryApp { items, hwnd, frame: 0, was_focused: false, copied: None, height: 0.0 }))
        }),
    );
    if let Err(e) = r {
        log::error!("history window: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ago_buckets() {
        assert_eq!(ago(0), "just now");
        assert_eq!(ago(4), "just now");
        assert_eq!(ago(42), "42 s ago");
        assert_eq!(ago(60), "1 min ago");
        assert_eq!(ago(3599), "59 min ago");
        assert_eq!(ago(7200), "2 h ago");
    }
}
