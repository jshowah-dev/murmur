use anyhow::Result;
use std::sync::atomic::{AtomicU32, Ordering};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::Shell::{Shell_NotifyIconW, NIF_INFO, NIIF_INFO, NIM_MODIFY, NOTIFYICONDATAW};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    TogglePause,
    FixLast,
    History,
    EditDictionary,
    OpenSnippets,
    OpenConfigDir,
    ToggleAutostart,
    Quit,
}

pub struct Tray {
    _icon: TrayIcon,
    pause: MenuItem,
    autostart: CheckMenuItem,
    ids: [(MenuId, TrayEvent); 8],
}

// Icon resource ids from build.rs (1 is the app icon)
const ICON_LIVE: u16 = 2;
const ICON_PAUSED: u16 = 3;

/// The tray's small-icon size at `dpi`: 16 px at 100% scaling, 32 px at 200%.
fn tray_px(dpi: u32) -> u32 {
    (16 * dpi + 48) / 96
}

fn icon(paused: bool) -> Icon {
    // Murmur is DPI-unaware, so it has to ask for the real DPI or it always gets 96
    let px = tray_px(crate::caret::physical(|| unsafe { GetDpiForSystem() }));
    Icon::from_resource(if paused { ICON_PAUSED } else { ICON_LIVE }, Some((px, px))).expect("icon")
}

impl Tray {
    pub fn create() -> Result<Tray> {
        let menu = Menu::new();
        let pause = MenuItem::new("Pause", true, None);
        let fix = MenuItem::new("Fix last (Left Shift+PTT)", true, None);
        let hist = MenuItem::new("History…", true, None);
        let dict = MenuItem::new("Dictionary…", true, None);
        let snip = MenuItem::new("Open snippets", true, None);
        let cfg = MenuItem::new("Open config folder", true, None);
        let autostart = CheckMenuItem::new("Start with Windows", true, crate::autostart::is_enabled(), None);
        let quit = MenuItem::new("Quit", true, None);
        menu.append_items(&[&pause, &fix, &hist, &PredefinedMenuItem::separator(), &dict, &snip, &cfg, &autostart, &PredefinedMenuItem::separator(), &quit])?;
        let ids = [
            (pause.id().clone(), TrayEvent::TogglePause),
            (fix.id().clone(), TrayEvent::FixLast),
            (hist.id().clone(), TrayEvent::History),
            (dict.id().clone(), TrayEvent::EditDictionary),
            (snip.id().clone(), TrayEvent::OpenSnippets),
            (cfg.id().clone(), TrayEvent::OpenConfigDir),
            (autostart.id().clone(), TrayEvent::ToggleAutostart),
            (quit.id().clone(), TrayEvent::Quit),
        ];
        let _icon = TrayIconBuilder::new().with_menu(Box::new(menu)).with_tooltip("Murmur").with_icon(icon(false)).build()?;
        Ok(Tray { _icon, pause, autostart, ids })
    }

    pub fn poll(&self) -> Option<TrayEvent> {
        let ev = MenuEvent::receiver().try_recv().ok()?;
        self.ids.iter().find(|(id, _)| *id == ev.id).map(|(_, e)| *e)
    }

    pub fn set_paused(&self, paused: bool) {
        self.pause.set_text(if paused { "Resume" } else { "Pause" });
        let _ = self._icon.set_icon(Some(icon(paused)));
    }

    /// muda flips a CheckMenuItem on click; the caller sets it back from the registry.
    pub fn set_autostart_checked(&self, on: bool) {
        self.autostart.set_checked(on);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_icon_size_follows_dpi() {
        assert_eq!(tray_px(96), 16);
        assert_eq!(tray_px(120), 20);
        assert_eq!(tray_px(144), 24);
        assert_eq!(tray_px(192), 32);
    }
}
