use anyhow::Result;
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::{Shell_NotifyIconW, NIF_INFO, NIIF_INFO, NIM_MODIFY, NOTIFYICONDATAW};

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
        if !balloon(&self._icon, title, body) {
            // Fallback: Shell_NotifyIconW failed, use the tooltip as a lightweight notice.
            let _ = self._icon.set_tooltip(Some(format!("Murmur — {title}: {body}")));
        }
    }
}

/// tray-icon has no balloon API, but it internally registers the icon with Shell_NotifyIconW
/// and a uID counter that starts at 1; Murmur creates exactly one tray icon, so its uID is
/// always 1. `TrayIcon::window_handle` gives us its hidden window directly.
fn balloon(icon: &TrayIcon, title: &str, body: &str) -> bool {
    unsafe {
        let hwnd = HWND(icon.window_handle() as *mut _);
        let mut nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            uFlags: NIF_INFO,
            dwInfoFlags: NIIF_INFO,
            ..Default::default()
        };
        set_wide_field(&mut nid.szInfoTitle, title);
        set_wide_field(&mut nid.szInfo, body);
        Shell_NotifyIconW(NIM_MODIFY, &nid).as_bool()
    }
}

/// Copies `s` into a fixed-size UTF-16 buffer, truncating on a UTF-16 code-unit boundary
/// (leaving room for the trailing NUL) if it doesn't fit. This truncation point may fall
/// in the middle of a surrogate pair, splitting it.
fn set_wide_field<const N: usize>(field: &mut [u16; N], s: &str) {
    let wide: Vec<u16> = s.encode_utf16().collect();
    let max = N - 1;
    let len = wide.len().min(max);
    field[..len].copy_from_slice(&wide[..len]);
    field[len] = 0;
}
