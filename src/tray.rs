use anyhow::Result;
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    TogglePause,
    FixLast,
    OpenDictionary,
    OpenConfigDir,
    Quit,
}

pub struct Tray {
    _icon: TrayIcon,
    pause: MenuItem,
    ids: [(MenuId, TrayEvent); 5],
}

fn icon(paused: bool) -> Icon {
    // 16x16 solid circle, grey when paused, green when live
    let (r, g, b) = if paused { (120u8, 120u8, 120u8) } else { (80u8, 200u8, 120u8) };
    let mut rgba = vec![0u8; 16 * 16 * 4];
    for y in 0..16 {
        for x in 0..16 {
            let dx = x as f32 - 7.5;
            let dy = y as f32 - 7.5;
            if dx * dx + dy * dy <= 7.0 * 7.0 {
                let i = (y * 16 + x) * 4;
                rgba[i] = r;
                rgba[i + 1] = g;
                rgba[i + 2] = b;
                rgba[i + 3] = 255;
            }
        }
    }
    Icon::from_rgba(rgba, 16, 16).expect("icon")
}

impl Tray {
    pub fn create() -> Result<Tray> {
        let menu = Menu::new();
        let pause = MenuItem::new("Pause", true, None);
        let fix = MenuItem::new("Fix last (Shift+PTT)", true, None);
        let dict = MenuItem::new("Open dictionary", true, None);
        let cfg = MenuItem::new("Open config folder", true, None);
        let quit = MenuItem::new("Quit", true, None);
        menu.append_items(&[&pause, &fix, &PredefinedMenuItem::separator(), &dict, &cfg, &PredefinedMenuItem::separator(), &quit])?;
        let ids = [
            (pause.id().clone(), TrayEvent::TogglePause),
            (fix.id().clone(), TrayEvent::FixLast),
            (dict.id().clone(), TrayEvent::OpenDictionary),
            (cfg.id().clone(), TrayEvent::OpenConfigDir),
            (quit.id().clone(), TrayEvent::Quit),
        ];
        let _icon = TrayIconBuilder::new().with_menu(Box::new(menu)).with_tooltip("Murmur").with_icon(icon(false)).build()?;
        Ok(Tray { _icon, pause, ids })
    }

    pub fn poll(&self) -> Option<TrayEvent> {
        let ev = MenuEvent::receiver().try_recv().ok()?;
        self.ids.iter().find(|(id, _)| *id == ev.id).map(|(_, e)| *e)
    }

    pub fn set_paused(&self, paused: bool) {
        self.pause.set_text(if paused { "Resume" } else { "Pause" });
        let _ = self._icon.set_icon(Some(icon(paused)));
    }

    pub fn notify(&self, title: &str, body: &str) {
        log::info!("notify: {title}: {body}");
        // tray-icon has no balloon API; use the tooltip as a lightweight notice.
        let _ = self._icon.set_tooltip(Some(format!("Murmur — {title}: {body}")));
    }
}
