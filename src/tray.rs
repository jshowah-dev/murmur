use anyhow::Result;
use std::sync::atomic::{AtomicU32, Ordering};
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
    TestNotify,
    Quit,
}

pub struct Tray {
    _icon: TrayIcon,
    pause: MenuItem,
    ids: [(MenuId, TrayEvent); 6],
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
        let fix = MenuItem::new("Fix last (Left Shift+PTT)", true, None);
        let dict = MenuItem::new("Open dictionary", true, None);
        let cfg = MenuItem::new("Open config folder", true, None);
        let test = MenuItem::new("Test notification", true, None);
        let quit = MenuItem::new("Quit", true, None);
        menu.append_items(&[&pause, &fix, &PredefinedMenuItem::separator(), &dict, &cfg, &test, &PredefinedMenuItem::separator(), &quit])?;
        let ids = [
            (pause.id().clone(), TrayEvent::TogglePause),
            (fix.id().clone(), TrayEvent::FixLast),
            (dict.id().clone(), TrayEvent::OpenDictionary),
            (cfg.id().clone(), TrayEvent::OpenConfigDir),
            (test.id().clone(), TrayEvent::TestNotify),
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
        let ok = balloon(&self._icon, title, body);
        log::info!("notify: {title}: {body} (balloon accepted: {ok})");
        if !ok {
            // Fallback: Shell_NotifyIconW failed, use the tooltip as a lightweight notice.
            let _ = self._icon.set_tooltip(Some(format!("Murmur — {title}: {body}")));
        }
    }
}

/// tray-icon has no balloon API, but it registers the icon with Shell_NotifyIconW using an
/// internal counter shared with its unique-id generator, so the uID is not fixed (2 with
/// tray-icon 0.24: the builder burns 1 on its string id). NIM_MODIFY on a wrong uID just
/// returns false, so probe a few and remember the one Windows accepts.
static BALLOON_UID: AtomicU32 = AtomicU32::new(0);

fn balloon(icon: &TrayIcon, title: &str, body: &str) -> bool {
    let hwnd = HWND(icon.window_handle() as *mut _);
    let known = BALLOON_UID.load(Ordering::Relaxed);
    let candidates: Vec<u32> = if known != 0 { vec![known] } else { (1..=4).collect() };
    for uid in candidates {
        let mut nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: uid,
            uFlags: NIF_INFO,
            dwInfoFlags: NIIF_INFO,
            ..Default::default()
        };
        set_wide_field(&mut nid.szInfoTitle, title);
        set_wide_field(&mut nid.szInfo, body);
        if unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid).as_bool() } {
            if known == 0 {
                log::info!("balloon uID resolved to {uid}");
                BALLOON_UID.store(uid, Ordering::Relaxed);
            }
            return true;
        }
    }
    false
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
