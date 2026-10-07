use super::Window;
use anyhow::{Context, Result};
use objc2::rc::Retained;
use block2::{DynBlock, RcBlock};
use objc2::runtime::{AnyObject, Bool, ProtocolObject};
use objc2::{define_class, msg_send, AllocAnyThread, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationOptions, NSApplicationActivationPolicy, NSBackingStoreType, NSColor, NSEvent, NSEventMask,
    NSPanel, NSRunningApplication, NSScreen, NSStatusWindowLevel, NSView, NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask,
    NSWorkspace,
};
use objc2_core_foundation::CFData;
use objc2_core_graphics::{
    kCGColorSpaceSRGB, CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGEventSource, CGEventSourceStateID, CGImage,
    CGImageAlphaInfo, CGImageByteOrderInfo,
};
use objc2_foundation::{
    NSBundle, NSDate, NSDefaultRunLoopMode, NSDictionary, NSError, NSNumber, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification, UNNotificationPresentationOptions, UNNotificationRequest,
    UNNotificationResponse, UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};
use objc2_quartz_core::{CALayer, CATransaction};
use std::fs::{File, TryLockError};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// A screen rectangle in Win32's shape, so the shared code needs one kind of rectangle.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

// Windows virtual-key codes, which config.toml's key names map to on both systems
const VK_SHIFT: u16 = 0x10;
const VK_CONTROL: u16 = 0x11;
const VK_MENU: u16 = 0x12;
const VK_CAPITAL: u16 = 0x14;
const VK_ESCAPE: u16 = 0x1B;
const VK_LWIN: u16 = 0x5B;
const VK_RWIN: u16 = 0x5C;
const VK_F1: u16 = 0x70;
const VK_LSHIFT: u16 = 0xA0;
const VK_RSHIFT: u16 = 0xA1;
const VK_LCONTROL: u16 = 0xA2;
const VK_RCONTROL: u16 = 0xA3;
const VK_LMENU: u16 = 0xA4;
const VK_RMENU: u16 = 0xA5;

/// The device-dependent bits of the modifier flags (IOKit's NX_DEVICE*KEYMASK): one per side.
/// macOS reports these without the Input Monitoring permission that per-key state needs.
fn modifier_bits(vk: u16) -> Option<u64> {
    const LCTL: u64 = 0x0001;
    const LSHIFT: u64 = 0x0002;
    const RSHIFT: u64 = 0x0004;
    const LCMD: u64 = 0x0008;
    const RCMD: u64 = 0x0010;
    const LALT: u64 = 0x0020;
    const RALT: u64 = 0x0040;
    const RCTL: u64 = 0x2000;
    Some(match vk {
        VK_SHIFT => LSHIFT | RSHIFT,
        VK_CONTROL => LCTL | RCTL,
        VK_MENU => LALT | RALT,
        VK_LSHIFT => LSHIFT,
        VK_RSHIFT => RSHIFT,
        VK_LCONTROL => LCTL,
        VK_RCONTROL => RCTL,
        VK_LMENU => LALT,
        VK_RMENU => RALT,
        VK_LWIN => LCMD,
        VK_RWIN => RCMD,
        _ => return None,
    })
}

/// The Mac key code for a non-modifier key Murmur watches: Esc, Caps Lock, F1 to F20.
fn key_code(vk: u16) -> Option<u16> {
    // kVK_F1 … kVK_F20 from HIToolbox's Events.h; they are not in order
    const F_KEYS: [u16; 20] =
        [0x7A, 0x78, 0x63, 0x76, 0x60, 0x61, 0x62, 0x64, 0x65, 0x6D, 0x67, 0x6F, 0x69, 0x6B, 0x71, 0x6A, 0x40, 0x4F, 0x50, 0x5A];
    match vk {
        VK_ESCAPE => Some(0x35),
        VK_CAPITAL => Some(0x39),
        _ => F_KEYS.get(vk.checked_sub(VK_F1)? as usize).copied(),
    }
}

/// Whether the key with this Windows virtual-key code is held right now. Modifiers need no
/// permission; other keys read as up until Murmur has Input Monitoring.
pub fn key_down(vk: u16) -> bool {
    if let Some(bits) = modifier_bits(vk) {
        return CGEventSource::flags_state(CGEventSourceStateID::HIDSystemState).0 & bits != 0;
    }
    key_code(vk).is_some_and(|code| CGEventSource::key_state(CGEventSourceStateID::CombinedSessionState, code))
}

/// The ViewportBuilder's always-on-top is a floating window level on macOS, which stays put when
/// the window is raised, so there's nothing to redo.
pub fn keep_on_top(_w: Window) {}

/// The NSWindow holding the egui view `w`.
fn ns_window(w: Window) -> Option<Retained<NSWindow>> {
    MainThreadMarker::new()?;
    let view = unsafe { (w.0 as *const NSView).as_ref() }?;
    view.window()
}

/// Moves `w`'s top-left to (x, y) points from the top-left of the main display.
pub fn move_to(w: Window, x: i32, y: i32) {
    let (Some(win), Some(mtm)) = (ns_window(w), MainThreadMarker::new()) else { return };
    win.setFrameTopLeftPoint(NSPoint::new(x as f64, primary_height(mtm) - y as f64));
}

/// `w`'s frame in points from the top-left of the main display.
pub fn window_rect(w: Window) -> Option<Rect> {
    let (win, mtm) = (ns_window(w)?, MainThreadMarker::new()?);
    Some(flipped(win.frame(), primary_height(mtm)))
}

pub fn is_minimized(_w: Window) -> bool {
    false
}

/// Murmur's windows all belong to one app, so "in front" means Murmur is the active app.
pub fn is_foreground(_w: Window) -> bool {
    MainThreadMarker::new().is_some_and(|mtm| NSApplication::sharedApplication(mtm).isActive())
}

/// Makes Murmur the active app, so its window takes the keyboard.
pub fn raise(_w: Window) {
    if let Some(mtm) = MainThreadMarker::new() {
        NSApplication::sharedApplication(mtm).activate();
    }
}

/// Gives the focus back to the app a dictation went to (`inject::foreground_hwnd`, a pid).
pub fn focus_target(target: isize) {
    if let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(target as libc::pid_t) {
        app.activateWithOptions(NSApplicationActivationOptions::empty());
    }
}

// Finding another process's window by title needs Screen Recording; the editor's Dock icon
// (`show_in_dock`) brings it back instead.
pub fn raise_titled(_title: &str) {}

/// Gives this process a Dock icon and a place in ⌘Tab, so its window can't be lost behind others.
/// For the editor, which outlives the menu click that opened it.
pub fn show_in_dock() {
    if let Some(mtm) = MainThreadMarker::new() {
        NSApplication::sharedApplication(mtm).setActivationPolicy(NSApplicationActivationPolicy::Regular);
    }
}

pub fn open_path(p: &Path) {
    let _ = std::process::Command::new("open").arg(p).spawn();
}

/// System Settings › Accessibility › Display › Reduce motion.
pub fn reduced_motion() -> bool {
    NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

/// Held for as long as this process is the one instance of `name`: an exclusive lock on a file.
pub struct Instance(#[allow(dead_code)] File);

fn lock_file(name: &str) -> Result<File> {
    // tests take and drop instances with made-up names; those lock files go in the temp folder
    let dir = if cfg!(test) { std::env::temp_dir() } else { murmur_lib::config::config_dir() };
    std::fs::create_dir_all(&dir).context("create config dir")?;
    let path = dir.join(super::lock_name(name));
    File::options().create(true).truncate(false).write(true).open(&path).with_context(|| format!("open {}", path.display()))
}

/// Claims the single instance `name`; None when another process already has it.
pub fn single_instance(name: &str) -> Result<Option<Instance>> {
    let f = lock_file(name)?;
    match f.try_lock() {
        Ok(()) => Ok(Some(Instance(f))),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(e)) => Err(e).context("lock"),
    }
}

/// Whether some process holds the single instance `name`, from outside it.
pub fn instance_exists(name: &str) -> bool {
    lock_file(name).is_ok_and(|f| matches!(f.try_lock(), Err(TryLockError::WouldBlock)))
}

/// Makes Murmur a menu bar app (no Dock icon) and gets AppKit ready for `pump`. Call first thing,
/// on the main thread.
pub fn init_app() {
    let mtm = MainThreadMarker::new().expect("init_app on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
}

/// Hands AppKit the events waiting for it (the menu bar item's clicks among them), without waiting.
pub fn pump() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);
    let past = NSDate::distantPast();
    while let Some(ev) = unsafe { app.nextEventMatchingMask_untilDate_inMode_dequeue(NSEventMask::Any, Some(&past), NSDefaultRunLoopMode, true) } {
        app.sendEvent(&ev);
    }
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrustedWithOptions(options: *const NSDictionary<NSString, NSNumber>) -> bool;
}

/// Whether Murmur may post the Cmd+V that pastes. With `prompt`, macOS shows its "allow in
/// System Settings" dialog when it may not.
pub fn accessibility_trusted(prompt: bool) -> bool {
    // the value of ApplicationServices' kAXTrustedCheckOptionPrompt
    let key = NSString::from_str("AXTrustedCheckOptionPrompt");
    let options = NSDictionary::from_slices(&[&*key], &[&*NSNumber::numberWithBool(prompt)]);
    unsafe { AXIsProcessTrustedWithOptions(&*options) }
}

/// The app in front, by process id.
pub fn frontmost_pid() -> isize {
    NSWorkspace::sharedWorkspace().frontmostApplication().map_or(0, |a| a.processIdentifier() as isize)
}

/// The height of the display with the menu bar, which AppKit's bottom-up coordinates start from.
fn primary_height(mtm: MainThreadMarker) -> f64 {
    NSScreen::screens(mtm).firstObject().map_or(0.0, |s| s.frame().size.height)
}

/// An AppKit rect (bottom-up from the main display) as a top-left one.
fn flipped(r: NSRect, primary: f64) -> Rect {
    let top = primary - (r.origin.y + r.size.height);
    Rect { left: r.origin.x as i32, top: top as i32, right: (r.origin.x + r.size.width) as i32, bottom: (top + r.size.height) as i32 }
}

/// The usable part (no menu bar, no Dock) of the screen holding `at`, or of the nearest screen
/// to it, in points from the top-left of the main display.
pub fn work_area_at(at: Rect) -> Rect {
    let Some(mtm) = MainThreadMarker::new() else { return Rect { left: 0, top: 0, right: 1440, bottom: 900 } };
    let primary = primary_height(mtm);
    let screens: Vec<(Rect, Rect)> = NSScreen::screens(mtm).iter().map(|s| (flipped(s.frame(), primary), flipped(s.visibleFrame(), primary))).collect();
    nearest_screen(&screens, at).unwrap_or(Rect { left: 0, top: 0, right: 1440, bottom: 900 })
}

/// Of `screens` as (frame, usable area), the usable area of the one holding `at`'s centre, else
/// of the one nearest it.
fn nearest_screen(screens: &[(Rect, Rect)], at: Rect) -> Option<Rect> {
    let (cx, cy) = ((at.left + at.right) / 2, (at.top + at.bottom) / 2);
    let gap = |f: &Rect| {
        let dx = (f.left - cx).max(cx - (f.right - 1)).max(0) as i64;
        let dy = (f.top - cy).max(cy - (f.bottom - 1)).max(0) as i64;
        dx * dx + dy * dy
    };
    screens.iter().min_by_key(|(frame, _)| gap(frame)).map(|&(_, work)| work)
}

/// The usable part (no menu bar, no Dock) of the screen you're working on, as (x, y, w, h) points
/// from the top-left of the main display, and its pixels per point.
pub fn work_area() -> ((f32, f32, f32, f32), f32) {
    let Some(mtm) = MainThreadMarker::new() else { return ((0.0, 0.0, 1440.0, 900.0), 2.0) };
    let Some(screen) = NSScreen::mainScreen(mtm) else { return ((0.0, 0.0, 1440.0, 900.0), 2.0) };
    let f = screen.visibleFrame();
    let top = primary_height(mtm) - (f.origin.y + f.size.height);
    ((f.origin.x as f32, top as f32, f.size.width as f32, f.size.height as f32), screen.backingScaleFactor() as f32)
}

/// The mouse pointer, in points from the top-left of the main display.
pub fn cursor() -> (f32, f32) {
    let Some(mtm) = MainThreadMarker::new() else { return (0.0, 0.0) };
    let p = NSEvent::mouseLocation();
    (p.x as f32, (primary_height(mtm) - p.y) as f32)
}

/// A borderless window above other windows, on every Space and over full-screen apps, that
/// never takes the focus or a click, and shows the pixels it's given: the pill's window.
pub struct Panel {
    panel: Retained<NSPanel>,
}

/// A borderless, click-through, never-focused panel above other windows, on every Space and
/// over full-screen apps, drawing through its layer.
fn overlay_panel(mtm: MainThreadMarker) -> Result<Retained<NSPanel>> {
    let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1.0, 1.0));
    let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
    let panel = NSPanel::initWithContentRect_styleMask_backing_defer(NSPanel::alloc(mtm), rect, style, NSBackingStoreType::Buffered, false);
    unsafe { panel.setReleasedWhenClosed(false) };
    panel.setOpaque(false);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    panel.setHasShadow(false);
    panel.setLevel(NSStatusWindowLevel);
    panel.setIgnoresMouseEvents(true);
    // an accessory app is never active, and a panel would otherwise hide whenever it isn't
    panel.setHidesOnDeactivate(false);
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::IgnoresCycle,
    );
    panel.contentView().context("panel has no content view")?.setWantsLayer(true);
    Ok(panel)
}

/// Premultiplied BGRA `pixels`, `pw`×`ph`, as an image a layer can show.
fn image(pixels: &[u32], (pw, ph): (usize, usize)) -> Option<objc2_core_foundation::CFRetained<CGImage>> {
    if pixels.len() != pw * ph {
        log::warn!("panel: {} pixels for a {pw}x{ph} image", pixels.len());
        return None;
    }
    let bytes: Vec<u8> = pixels.iter().flat_map(|p| p.to_le_bytes()).collect();
    let provider = CGDataProvider::with_cf_data(Some(&CFData::from_bytes(&bytes)));
    let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }));
    // 0xAARRGGBB stored little-endian: B, G, R, A in memory
    let info = CGBitmapInfo(CGImageAlphaInfo::PremultipliedFirst.0 | CGImageByteOrderInfo::Order32Little.0);
    unsafe { CGImage::new(pw, ph, 8, 32, pw * 4, space.as_deref(), info, provider.as_deref(), std::ptr::null(), false, CGColorRenderingIntent::RenderingIntentDefault) }
}

/// Orders `panel` front unless it's on screen. Asked of the panel rather than remembered: when an
/// egui window's event loop ends, winit closes every window the app has, these panels included.
fn front(panel: &NSPanel) {
    if !panel.isVisible() {
        panel.orderFrontRegardless();
    }
}

fn set_contents(layer: &CALayer, image: &CGImage, scale: f32) {
    unsafe { layer.setContents(Some(&*(image as *const CGImage).cast::<AnyObject>())) };
    layer.setContentsScale(scale as f64);
}

impl Panel {
    pub fn new() -> Result<Panel> {
        let mtm = MainThreadMarker::new().context("a panel is made on the main thread")?;
        Ok(Panel { panel: overlay_panel(mtm)? })
    }

    /// Shows `pw`×`ph` premultiplied BGRA `pixels` (`scale` pixels per point) in a `w`×`h` point
    /// frame whose top-left is at (x, y) points from the top-left of the main display.
    pub fn show(&self, (x, y, w, h): (f32, f32, f32, f32), pixels: &[u32], (pw, ph): (usize, usize), scale: f32) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let Some(layer) = self.panel.contentView().and_then(|v| v.layer()) else { return };
        let Some(image) = image(pixels, (pw, ph)) else { return };
        let frame = NSRect::new(NSPoint::new(x as f64, primary_height(mtm) - y as f64 - h as f64), NSSize::new(w as f64, h as f64));
        CATransaction::begin();
        // a new frame replaces the last at once, as on Windows: no cross-fade
        CATransaction::setDisableActions(true);
        set_contents(&layer, &image, scale);
        self.panel.setFrame_display(frame, false);
        CATransaction::commit();
        front(&self.panel);
    }
}

/// A click-through panel over the whole screen that never moves: what moves is a layer inside
/// it, which Core Animation draws in step with the display. Moving a window every frame instead
/// smears, because the window server moves it apart from redrawing it. The mote's window.
pub struct Stage {
    panel: Retained<NSPanel>,
    sprite: Retained<CALayer>,
    shown: std::cell::Cell<bool>,
}

impl Stage {
    pub fn new() -> Result<Stage> {
        let mtm = MainThreadMarker::new().context("a stage is made on the main thread")?;
        let panel = overlay_panel(mtm)?;
        let root = panel.contentView().and_then(|v| v.layer()).context("stage has no layer")?;
        let sprite = CALayer::new();
        root.addSublayer(&sprite);
        Ok(Stage { panel, sprite, shown: std::cell::Cell::new(false) })
    }

    /// Shows `pw`×`ph` premultiplied BGRA `pixels` (`scale` pixels per point) in a `w`×`h` point
    /// rectangle whose top-left is at (x, y) points from the top-left of the main display.
    pub fn show(&self, (x, y, w, h): (f32, f32, f32, f32), pixels: &[u32], size: (usize, usize), scale: f32) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let Some(screen) = NSScreen::mainScreen(mtm) else { return };
        let Some(image) = image(pixels, size) else { return };
        let sf = screen.frame();
        // AppKit owns the view's layer, which counts y up from the bottom like the rest of AppKit:
        // the sprite's bottom edge, from the screen's bottom edge
        let bottom = (primary_height(mtm) - (y as f64 + h as f64)) - sf.origin.y;
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        if self.panel.frame() != sf {
            self.panel.setFrame_display(sf, false);
        }
        set_contents(&self.sprite, &image, scale);
        self.sprite.setFrame(NSRect::new(NSPoint::new(x as f64 - sf.origin.x, bottom), NSSize::new(w as f64, h as f64)));
        self.sprite.setHidden(false);
        CATransaction::commit();
        self.shown.set(true);
        front(&self.panel);
    }

    pub fn hide(&self) {
        if self.shown.replace(false) {
            CATransaction::begin();
            CATransaction::setDisableActions(true);
            self.sprite.setHidden(true);
            CATransaction::commit();
            self.panel.orderOut(None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
        Rect { left, top, right, bottom }
    }

    #[test]
    fn the_work_area_is_the_screens_the_caret_is_on_or_the_nearest() {
        // a laptop with the menu bar, and a display to its right that sits 200 points higher
        let laptop = (r(0, 0, 1710, 1107), r(0, 33, 1710, 1020));
        let right = (r(1710, -200, 4270, 1240), r(1710, -200, 4270, 1240));
        let screens = [laptop, right];
        let caret = |x, y| r(x, y, x, y + 18);
        assert_eq!(nearest_screen(&screens, caret(400, 500)), Some(laptop.1));
        assert_eq!(nearest_screen(&screens, caret(2000, -100)), Some(right.1), "above the laptop's top edge");
        assert_eq!(nearest_screen(&screens, caret(1709, 500)), Some(laptop.1), "the laptop's last column");
        assert_eq!(nearest_screen(&screens, caret(1710, 500)), Some(right.1), "the right display's first column");
        assert_eq!(nearest_screen(&screens, caret(-50, 300)), Some(laptop.1), "off every screen: the nearest");
        assert_eq!(nearest_screen(&screens, caret(5000, 1300)), Some(right.1));
        assert_eq!(nearest_screen(&[], caret(0, 0)), None);
    }

    #[test]
    fn an_appkit_rect_flips_to_top_left() {
        let f = NSRect::new(NSPoint::new(10.0, 87.0), NSSize::new(1710.0, 986.0));
        assert_eq!(flipped(f, 1107.0), r(10, 34, 1720, 1020));
    }

    #[test]
    fn each_side_of_a_modifier_has_its_own_bit() {
        assert_eq!(modifier_bits(VK_RMENU), Some(0x40));
        assert_eq!(modifier_bits(VK_LMENU), Some(0x20));
        assert_eq!(modifier_bits(VK_MENU), Some(0x60));
        assert_eq!(modifier_bits(VK_RCONTROL), Some(0x2000));
        assert_eq!(modifier_bits(VK_ESCAPE), None);
    }

    #[test]
    fn f_keys_and_esc_have_mac_key_codes() {
        assert_eq!(key_code(VK_ESCAPE), Some(0x35));
        assert_eq!(key_code(VK_F1), Some(0x7A));
        assert_eq!(key_code(VK_F1 + 12), Some(0x69)); // F13
        assert_eq!(key_code(VK_F1 + 20), None); // F21: none on a Mac
        assert_eq!(key_code(0x41), None);
    }
}

/// Notifications go through UserNotifications. `init_notifications` asks macOS once (it remembers
/// the answer) and sets `clicked` when one is clicked. Outside an app bundle (tests, `cargo run`)
/// the notification center throws, so there are none.
pub fn init_notifications(clicked: &'static AtomicBool) {
    if NSBundle::mainBundle().bundleIdentifier().is_none() {
        return;
    }
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let delegate = NoticeDelegate::new(clicked);
    center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    // the center holds its delegate weakly, and Murmur needs it until it quits
    std::mem::forget(delegate);
    let answered = RcBlock::new(|allowed: Bool, _: *mut NSError| log::info!("notifications allowed: {}", allowed.as_bool()));
    center.requestAuthorizationWithOptions_completionHandler(UNAuthorizationOptions::Alert, &answered);
}

/// Posts a notification; false if there's no notification center. Not allowed (yet) means
/// macOS drops it quietly.
pub fn notify(title: &str, body: &str) -> bool {
    if NSBundle::mainBundle().bundleIdentifier().is_none() {
        return false;
    }
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    content.setBody(&NSString::from_str(body));
    // one id, so a new notification replaces the last, like a balloon: a click belongs to the last
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&NSString::from_str("murmur"), &content, None);
    let posted = RcBlock::new(|err: *mut NSError| {
        if let Some(e) = unsafe { err.as_ref() } {
            log::warn!("notification: {}", e.localizedDescription());
        }
    });
    UNUserNotificationCenter::currentNotificationCenter().addNotificationRequest_withCompletionHandler(&request, Some(&posted));
    true
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "MurmurNoticeDelegate"]
    #[ivars = &'static AtomicBool]
    struct NoticeDelegate;

    unsafe impl NSObjectProtocol for NoticeDelegate {}

    unsafe impl UNUserNotificationCenterDelegate for NoticeDelegate {
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(&self, _center: &UNUserNotificationCenter, _response: &UNNotificationResponse, done: &DynBlock<dyn Fn()>) {
            self.ivars().store(true, Ordering::Relaxed);
            done.call(());
        }

        // shown even while Murmur is the app in front
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(&self, _center: &UNUserNotificationCenter, _n: &UNNotification, done: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>) {
            done.call((UNNotificationPresentationOptions::Banner | UNNotificationPresentationOptions::List,));
        }
    }
);

impl NoticeDelegate {
    fn new(clicked: &'static AtomicBool) -> Retained<Self> {
        let this = Self::alloc().set_ivars(clicked);
        unsafe { msg_send![super(this), init] }
    }
}
