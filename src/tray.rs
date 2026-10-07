use crate::notice::Balloon;
use anyhow::Result;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(windows)]
use std::sync::atomic::AtomicU32;
#[cfg(windows)]
use tray_icon::menu::ContextMenu;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
#[cfg(windows)]
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
#[cfg(windows)]
use windows::Win32::UI::HiDpi::GetDpiForSystem;
#[cfg(windows)]
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass, Shell_NotifyIconW, NIF_INFO, NIIF_INFO, NIM_MODIFY, NOTIFYICONDATAW};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, SetForegroundWindow};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    TogglePause,
    FixLast,
    History,
    EditDictionary,
    EditSnippets,
    PttKey,
    OpenConfigDir,
    ToggleAutostart,
    Update,
    /// The "update available" balloon was clicked.
    BalloonUpdate,
    DownloadModel,
    About,
    Quit,
}

pub struct Tray {
    _icon: TrayIcon,
    menu: Menu,
    pause: MenuItem,
    fix: MenuItem,
    ptt: MenuItem,
    autostart: CheckMenuItem,
    update: MenuItem,
    model: MenuItem,
    /// whether (model, update) are in the menu
    shown: Cell<(bool, bool)>,
    ids: [(MenuId, TrayEvent); 12],
    /// what a click on the balloon on screen does
    click: RefCell<BalloonClick>,
    /// (paused, live) as the icon shows them
    shows: Cell<(bool, bool)>,
}

/// Menu position of "About Murmur" before any optional item is added.
const ABOUT_AT: usize = 10;

/// Where the model and update items go when shown: above About, model first.
fn extras_positions(model: bool, update: bool) -> (Option<usize>, Option<usize>) {
    let m = model.then_some(ABOUT_AT);
    let u = update.then_some(ABOUT_AT + model as usize);
    (m, u)
}

// Icon resource ids from build.rs (1 is the app icon)
#[cfg(windows)]
const ICON_LIVE: u16 = 2;
#[cfg(windows)]
const ICON_PAUSED: u16 = 3;

/// The tray's small-icon size at `dpi`: 16 px at 100% scaling, 32 px at 200%.
#[cfg(windows)]
fn tray_px(dpi: u32) -> u32 {
    (16 * dpi + 48) / 96
}

/// Windows has no listening icon: the pill shows that.
#[cfg(windows)]
fn icon(paused: bool, _live: bool) -> Icon {
    // Murmur is DPI-unaware, so it has to ask for the real DPI or it always gets 96
    let px = tray_px(crate::caret::physical(|| unsafe { GetDpiForSystem() }));
    Icon::from_resource(if paused { ICON_PAUSED } else { ICON_LIVE }, Some((px, px))).expect("icon")
}

/// A menu bar template image (macOS tints it for the menu bar): a microphone, outlined at rest,
/// solid while listening, faint when paused. 36 px is the menu bar's 18 pt at 2x.
#[cfg(target_os = "macos")]
fn icon(paused: bool, live: bool) -> Icon {
    const PX: i32 = 36;
    let shape = |inset: f32| {
        let mut c = crate::canvas::Canvas::scaled(PX, PX, 1.0);
        c.capsule(12.0 + inset, 3.0 + inset, 12.0 - 2.0 * inset, 21.0 - 2.0 * inset, 0xFFFFFF, 1.0);
        if inset == 0.0 {
            c.capsule(16.75, 24.0, 2.5, 6.5, 0xFFFFFF, 1.0);
            c.capsule(11.0, 29.5, 14.0, 3.0, 0xFFFFFF, 1.0);
        }
        c.into_bgra().into_iter().map(|p| (p >> 24) as f32 / 255.0).collect::<Vec<_>>()
    };
    let outer = shape(0.0);
    let inner = if live { vec![0.0; outer.len()] } else { shape(2.5) };
    let alpha = if paused { 0.4 } else { 1.0 };
    let rgba = outer.iter().zip(&inner).flat_map(|(o, i)| [0, 0, 0, ((o - i).max(0.0) * alpha * 255.0).round() as u8]).collect();
    Icon::from_rgba(rgba, PX as u32, PX as u32).expect("icon")
}

#[cfg(windows)]
const AUTOSTART_LABEL: &str = "Start with Windows";
#[cfg(target_os = "macos")]
const AUTOSTART_LABEL: &str = "Open at Login";
/// The autostart item as a sentence names it.
#[cfg(windows)]
pub const AUTOSTART_PHRASE: &str = "start with Windows";
#[cfg(target_os = "macos")]
pub const AUTOSTART_PHRASE: &str = "open at login";

impl Tray {
    pub fn create(ptt_label: &str) -> Result<Tray> {
        let menu = Menu::new();
        let pause = MenuItem::new("Pause", true, None);
        let fix = MenuItem::new(fix_item_label(ptt_label), true, None);
        let hist = MenuItem::new("History…", true, None);
        let dict = MenuItem::new("Dictionary…", true, None);
        let snip = MenuItem::new("Snippets…", true, None);
        let ptt = MenuItem::new(ptt_item_label(ptt_label), true, None);
        let cfg = MenuItem::new("Open config folder", true, None);
        let autostart = CheckMenuItem::new(AUTOSTART_LABEL, true, crate::autostart::is_enabled(), None);
        let about = MenuItem::new("About Murmur", true, None);
        let quit = MenuItem::new("Quit", true, None);
        // not in the menu until there's something to offer
        let update = MenuItem::new("Update", true, None);
        let model = MenuItem::new("Download new speech model", true, None);
        menu.append_items(&[&pause, &fix, &hist, &PredefinedMenuItem::separator(), &dict, &snip, &ptt, &cfg, &autostart, &PredefinedMenuItem::separator(), &about, &quit])?;
        let ids = [
            (pause.id().clone(), TrayEvent::TogglePause),
            (fix.id().clone(), TrayEvent::FixLast),
            (hist.id().clone(), TrayEvent::History),
            (dict.id().clone(), TrayEvent::EditDictionary),
            (snip.id().clone(), TrayEvent::EditSnippets),
            (ptt.id().clone(), TrayEvent::PttKey),
            (cfg.id().clone(), TrayEvent::OpenConfigDir),
            (autostart.id().clone(), TrayEvent::ToggleAutostart),
            (update.id().clone(), TrayEvent::Update),
            (model.id().clone(), TrayEvent::DownloadModel),
            (about.id().clone(), TrayEvent::About),
            (quit.id().clone(), TrayEvent::Quit),
        ];
        let builder = TrayIconBuilder::new().with_menu(Box::new(menu.clone())).with_tooltip("Murmur").with_icon(icon(false, false));
        #[cfg(target_os = "macos")]
        let builder = builder.with_icon_as_template(true);
        let _icon = builder.build()?;
        // tray-icon reports clicks on the icon but not on a balloon, so listen on its window for those
        #[cfg(windows)]
        if !unsafe { SetWindowSubclass(HWND(_icon.window_handle() as *mut _), Some(balloon_proc), 1, 0) }.as_bool() {
            log::warn!("balloon clicks unavailable: SetWindowSubclass failed");
        }
        #[cfg(target_os = "macos")]
        crate::platform::init_notifications(&BALLOON_CLICKED);
        let shows = Cell::new((false, false));
        Ok(Tray { _icon, menu, pause, fix, ptt, autostart, update, model, shown: Cell::new((false, false)), ids, click: RefCell::new(BalloonClick::Nothing), shows })
    }

    pub fn poll(&self) -> Option<TrayEvent> {
        #[cfg(target_os = "macos")]
        self.show_icon(self.shows.get().0, crate::overlay::live());
        if BALLOON_CLICKED.swap(false, Ordering::Relaxed) {
            let (ev, open) = on_click(&self.click.borrow());
            for p in &open {
                open_in_notepad(p);
            }
            if ev.is_some() {
                return ev;
            }
        }
        let ev = MenuEvent::receiver().try_recv().ok()?;
        self.ids.iter().find(|(id, _)| *id == ev.id).map(|(_, e)| *e)
    }

    /// Shows the tray menu at the cursor, owned by `hwnd`. The pick arrives through `poll` like a
    /// tray pick. Showing the menu makes `hwnd` the foreground window, so the app you were in gets
    /// focus back afterwards, ready for the next dictation.
    #[cfg(windows)]
    pub fn show_menu(&self, owner: crate::platform::Window) {
        unsafe {
            let prev = GetForegroundWindow();
            self.menu.show_context_menu_for_hwnd(owner.0, None);
            if !prev.is_invalid() {
                let _ = SetForegroundWindow(prev);
            }
        }
    }

    /// The pill has no menu on macOS yet: it isn't on screen until phase 3.
    #[cfg(target_os = "macos")]
    pub fn show_menu(&self, _owner: crate::platform::Window) {}

    pub fn set_paused(&self, paused: bool) {
        self.pause.set_text(if paused { "Resume" } else { "Pause" });
        self.show_icon(paused, self.shows.get().1);
    }

    fn show_icon(&self, paused: bool, live: bool) {
        if self.shows.get() != (paused, live) {
            self.shows.set((paused, live));
            let _ = self._icon.set_icon(Some(icon(paused, live)));
            #[cfg(target_os = "macos")]
            self._icon.set_icon_as_template(true);
        }
    }

    pub fn set_ptt_label(&self, label: &str) {
        self.fix.set_text(fix_item_label(label));
        self.ptt.set_text(ptt_item_label(label));
    }

    /// muda flips a CheckMenuItem on click; the caller sets it back from the registry.
    pub fn set_autostart_checked(&self, on: bool) {
        self.autostart.set_checked(on);
    }

    /// Shows the update item with `label`, or takes it out of the menu with None.
    pub fn set_update(&self, label: Option<&str>, enabled: bool) {
        let (model, _) = self.shown.get();
        self.show_extras(model, relabel(&self.update, label, enabled));
    }

    /// Shows the model-download item with `label`, or takes it out of the menu with None.
    pub fn set_model(&self, label: Option<&str>, enabled: bool) {
        let (_, update) = self.shown.get();
        self.show_extras(relabel(&self.model, label, enabled), update);
    }

    /// muda has no hidden items, so the optional ones are taken out and put back in order.
    fn show_extras(&self, model: bool, update: bool) {
        let (had_model, had_update) = self.shown.get();
        if had_model {
            let _ = self.menu.remove(&self.model);
        }
        if had_update {
            let _ = self.menu.remove(&self.update);
        }
        let (m, u) = extras_positions(model, update);
        if let Some(at) = m {
            let _ = self.menu.insert(&self.model, at);
        }
        if let Some(at) = u {
            let _ = self.menu.insert(&self.update, at);
        }
        self.shown.set((model, update));
    }

    pub fn notify(&self, title: &str, body: &str) {
        self.show_balloon(title, body, BalloonClick::Nothing);
    }

    /// The "update available" balloon: a click raises `BalloonUpdate`.
    pub fn notify_update(&self, title: &str, body: &str) {
        self.show_balloon(title, body, BalloonClick::Update);
    }

    /// A notice; a click opens its files.
    pub fn show(&self, b: &Balloon) {
        self.show_balloon(&b.title, &b.body, click_for(b));
    }

    fn show_balloon(&self, title: &str, body: &str, click: BalloonClick) {
        // a click belongs to the balloon on screen, which is the last one shown
        *self.click.borrow_mut() = click;
        #[cfg(target_os = "macos")]
        let ok = crate::platform::notify(title, body);
        #[cfg(windows)]
        let ok = balloon(&self._icon, title, body);
        log::info!("notify: {title}: {body} (balloon accepted: {ok})");
        if !ok {
            // Fallback: Shell_NotifyIconW failed, use the tooltip as a lightweight notice.
            let _ = self._icon.set_tooltip(Some(format!("Murmur — {title}: {body}")));
        }
    }
}

fn fix_item_label(key: &str) -> String {
    format!("Fix last dictation (Shift + {key})")
}

fn ptt_item_label(key: &str) -> String {
    format!("Push-to-talk key: {key}…")
}

fn relabel(item: &MenuItem, label: Option<&str>, enabled: bool) -> bool {
    if let Some(text) = label {
        item.set_text(text);
        item.set_enabled(enabled);
    }
    label.is_some()
}

/// What a click on the balloon on screen does. It belongs to the last balloon shown.
#[derive(Debug, Clone, PartialEq)]
enum BalloonClick {
    Nothing,
    Update,
    Open(Vec<PathBuf>),
}

fn click_for(b: &Balloon) -> BalloonClick {
    if b.open.is_empty() { BalloonClick::Nothing } else { BalloonClick::Open(b.open.clone()) }
}

/// The event a click raises, and the files it opens.
fn on_click(c: &BalloonClick) -> (Option<TrayEvent>, Vec<PathBuf>) {
    match c {
        BalloonClick::Nothing => (None, vec![]),
        BalloonClick::Update => (Some(TrayEvent::BalloonUpdate), vec![]),
        BalloonClick::Open(paths) => (None, paths.clone()),
    }
}

/// Notepad (TextEdit on macOS): always there, and it shows the line number a notice names.
fn open_in_notepad(p: &Path) {
    #[cfg(windows)]
    let spawned = std::process::Command::new("notepad.exe").arg(p).spawn();
    #[cfg(target_os = "macos")]
    let spawned = std::process::Command::new("open").arg("-t").arg(p).spawn();
    if let Err(e) = spawned {
        log::error!("open {}: {e}", p.display());
    }
}

/// Set by `balloon_proc` (a notification's click on macOS), taken by `poll`.
static BALLOON_CLICKED: AtomicBool = AtomicBool::new(false);
/// tray-icon's private callback message (`WM_USER_TRAYICON`, tray-icon 0.25) and shellapi.h's
/// NIN_BALLOONUSERCLICK, which arrives as its lparam. If tray-icon renumbers, clicks just stop working.
#[cfg(windows)]
const TRAY_CALLBACK: u32 = 6002;
#[cfg(windows)]
const NIN_BALLOONUSERCLICK: u32 = 0x0405;

#[cfg(windows)]
fn is_balloon_click(msg: u32, lparam: isize) -> bool {
    msg == TRAY_CALLBACK && lparam as u32 == NIN_BALLOONUSERCLICK
}

#[cfg(windows)]
unsafe extern "system" fn balloon_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _id: usize, _data: usize) -> LRESULT {
    if is_balloon_click(msg, lp.0) {
        BALLOON_CLICKED.store(true, Ordering::Relaxed);
    }
    unsafe { DefSubclassProc(hwnd, msg, wp, lp) }
}

/// tray-icon has no balloon API, but it registers the icon with Shell_NotifyIconW using an
/// internal counter shared with its unique-id generator, so the uID is not fixed (2 with
/// tray-icon 0.24: the builder burns 1 on its string id). NIM_MODIFY on a wrong uID just
/// returns false, so probe a few and remember the one Windows accepts.
#[cfg(windows)]
static BALLOON_UID: AtomicU32 = AtomicU32::new(0);

#[cfg(windows)]
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
#[cfg(windows)]
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

    #[cfg(windows)]
    #[test]
    fn tray_icon_size_follows_dpi() {
        assert_eq!(tray_px(96), 16);
        assert_eq!(tray_px(120), 20);
        assert_eq!(tray_px(144), 24);
        assert_eq!(tray_px(192), 32);
    }

    #[test]
    fn a_click_does_what_the_last_balloon_offers() {
        let files = vec![PathBuf::from("C:\\m\\config.toml"), PathBuf::from("C:\\m\\dictionary.toml")];
        assert_eq!(on_click(&BalloonClick::Nothing), (None, vec![]));
        assert_eq!(on_click(&BalloonClick::Update), (Some(TrayEvent::BalloonUpdate), vec![]));
        assert_eq!(on_click(&BalloonClick::Open(files.clone())), (None, files));
        assert_eq!(click_for(&Balloon { title: "t".into(), body: "b".into(), open: vec![] }), BalloonClick::Nothing);
        assert_eq!(
            click_for(&Balloon { title: "t".into(), body: "b".into(), open: vec![PathBuf::from("C:\\m\\config.toml")] }),
            BalloonClick::Open(vec![PathBuf::from("C:\\m\\config.toml")])
        );
    }

    #[test]
    fn optional_items_sit_above_about() {
        // Pause, Fix last, History, ─, Dictionary, Snippets, Push-to-talk key, Config, Start with Windows, ─, [model], [update], About, Quit
        assert_eq!(extras_positions(false, false), (None, None));
        assert_eq!(extras_positions(false, true), (None, Some(10)));
        assert_eq!(extras_positions(true, false), (Some(10), None));
        assert_eq!(extras_positions(true, true), (Some(10), Some(11)));
    }

    #[cfg(windows)]
    #[test]
    fn only_a_balloon_click_on_the_tray_callback_counts() {
        assert!(is_balloon_click(6002, 0x0405));
        // the balloon timing out or being dismissed (NIN_BALLOONTIMEOUT), and a click on the icon
        assert!(!is_balloon_click(6002, 0x0404));
        assert!(!is_balloon_click(6002, 0x0202));
        assert!(!is_balloon_click(6003, 0x0405));
    }

    #[test]
    fn the_ptt_item_names_the_current_key() {
        assert_eq!(ptt_item_label("Right Ctrl"), "Push-to-talk key: Right Ctrl…");
    }

    #[test]
    fn the_fix_item_names_shift_and_the_current_key() {
        // either Shift works, so the label doesn't name a side
        assert_eq!(fix_item_label("Right Ctrl"), "Fix last dictation (Shift + Right Ctrl)");
        assert_eq!(fix_item_label("Left Ctrl + Left Alt"), "Fix last dictation (Shift + Left Ctrl + Left Alt)");
    }
}
