use crate::platform::Rect as RECT;
#[cfg(windows)]
use windows::core::BOOL;
#[cfg(windows)]
use windows::Win32::Foundation::{HWND, POINT};
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{ClientToScreen, GetMonitorInfoW, MonitorFromRect, MONITORINFO, MONITOR_DEFAULTTONEAREST};
#[cfg(windows)]
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, SAFEARRAY};
#[cfg(windows)]
use windows::Win32::System::Ole::{SafeArrayAccessData, SafeArrayDestroy, SafeArrayGetUBound, SafeArrayUnaccessData};
#[cfg(windows)]
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationTextPattern, IUIAutomationTextPattern2, IUIAutomationTextRange,
    TextPatternRangeEndpoint_End, TextPatternRangeEndpoint_Start, TextUnit_Character, UIA_TextPattern2Id, UIA_TextPatternId,
};
#[cfg(windows)]
use windows::Win32::UI::HiDpi::{SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{GetGUIThreadInfo, GetWindowRect, GetWindowThreadProcessId, GUITHREADINFO};

/// Where the dictation landed, in physical screen pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Anchor {
    /// The text caret: the dialog goes just below this line.
    Caret(RECT),
    /// Only the focused control or window is known: the dialog is centered over it.
    Area(RECT),
}

impl Anchor {
    fn rect(&self) -> RECT {
        match *self {
            Anchor::Caret(r) | Anchor::Area(r) => r,
        }
    }
}

/// Runs `f` with per-monitor DPI awareness so every coordinate it sees or sets is in physical
/// pixels; the rest of Murmur is DPI-unaware and would otherwise get scaled coordinates.
#[cfg(windows)]
pub fn physical<T>(f: impl FnOnce() -> T) -> T {
    unsafe {
        let old = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let r = f();
        SetThreadDpiAwarenessContext(old);
        r
    }
}

/// macOS has no per-thread DPI awareness to switch.
#[cfg(target_os = "macos")]
pub fn physical<T>(f: impl FnOnce() -> T) -> T {
    f()
}

/// Must be called while the target app still has focus. Asks Accessibility for the focused
/// element's insertion point (native Mac text fields and views), then for the element's frame.
/// Points from the top-left of the main display, which is how Accessibility reports them.
#[cfg(target_os = "macos")]
pub fn find(target: isize) -> Option<Anchor> {
    ax::find(target)
}

/// The character just before the insertion point in the focused text field, if the app says.
#[cfg(target_os = "macos")]
pub fn char_before() -> Option<char> {
    ax::char_before()
}

/// The usable part of the screen the anchor is on, or the nearest one.
#[cfg(target_os = "macos")]
pub fn work_area(anchor: &Anchor) -> RECT {
    crate::platform::work_area_at(anchor.rect())
}

#[cfg(target_os = "macos")]
mod ax {
    use super::{Anchor, RECT};
    use objc2_core_foundation::{CFRetained, CFString};
    use std::ffi::c_void;
    use std::ptr::null;

    type Ref = *const c_void;

    #[repr(C)]
    #[derive(Debug, Default, Clone, Copy)]
    struct Point {
        x: f64,
        y: f64,
    }

    #[repr(C)]
    #[derive(Debug, Default, Clone, Copy)]
    struct Size {
        w: f64,
        h: f64,
    }

    #[repr(C)]
    #[derive(Debug, Default, Clone, Copy)]
    struct Rect {
        origin: Point,
        size: Size,
    }

    // AXValueType
    const CG_POINT: u32 = 1;
    const CG_SIZE: u32 = 2;
    const CG_RECT: u32 = 3;
    const CF_RANGE: u32 = 4;

    #[repr(C)]
    #[derive(Debug, Default, Clone, Copy)]
    struct CfRange {
        location: isize,
        length: isize,
    }

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementCreateSystemWide() -> Ref;
        fn AXUIElementCreateApplication(pid: i32) -> Ref;
        fn AXUIElementSetAttributeValue(element: Ref, attribute: Ref, value: Ref) -> i32;
        fn AXUIElementCopyAttributeValue(element: Ref, attribute: Ref, value: *mut Ref) -> i32;
        fn AXUIElementCopyParameterizedAttributeValue(element: Ref, attribute: Ref, parameter: Ref, value: *mut Ref) -> i32;
        fn AXUIElementSetMessagingTimeout(element: Ref, seconds: f32) -> i32;
        fn AXValueGetValue(value: Ref, kind: u32, out: *mut c_void) -> bool;
        fn AXValueCreate(kind: u32, value: *const c_void) -> Ref;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(cf: Ref);
        static kCFBooleanTrue: Ref;
        static kCFTypeArrayCallBacks: c_void;
        fn CFArrayCreate(allocator: Ref, values: *const Ref, count: isize, callbacks: *const c_void) -> Ref;
        fn CFGetTypeID(cf: Ref) -> usize;
        fn CFStringGetTypeID() -> usize;
        fn CFStringGetLength(s: Ref) -> isize;
        fn CFStringGetCharacterAtIndex(s: Ref, idx: isize) -> u16;
    }

    /// Apps already asked to turn their accessibility on, by pid.
    static WOKEN: std::sync::Mutex<Vec<i32>> = std::sync::Mutex::new(Vec::new());

    /// Electron apps (Slack, VS Code, the Claude app) and Chromium browsers build their
    /// accessibility tree only once something asks for it, through AXManualAccessibility.
    /// Asked once per app; the tree arrives a moment later, so the first dictation may miss.
    fn wake(pid: i32) {
        let Ok(mut woken) = WOKEN.lock() else { return };
        if pid <= 0 || woken.contains(&pid) {
            return;
        }
        woken.push(pid);
        let app = Owned(unsafe { AXUIElementCreateApplication(pid) });
        let attr = name("AXManualAccessibility");
        let err = unsafe { AXUIElementSetAttributeValue(app.0, CFRetained::as_ptr(&attr).as_ptr() as Ref, kCFBooleanTrue) };
        log::info!("asked app {pid} for its accessibility tree (AX error {err})");
    }

    /// A CoreFoundation object this code got from a Create or Copy call, released on drop.
    struct Owned(Ref);

    impl Drop for Owned {
        fn drop(&mut self) {
            unsafe { CFRelease(self.0) };
        }
    }

    fn name(s: &str) -> CFRetained<CFString> {
        CFString::from_str(s)
    }

    fn attribute(element: &Owned, attr: &str) -> Option<Owned> {
        let attr = name(attr);
        let mut out = null();
        let err = unsafe { AXUIElementCopyAttributeValue(element.0, CFRetained::as_ptr(&attr).as_ptr() as Ref, &mut out) };
        (err == 0 && !out.is_null()).then(|| Owned(out))
    }

    fn parameterized(element: &Owned, attr: &str, parameter: &Owned) -> Option<Owned> {
        let attr = name(attr);
        let mut out = null();
        let err = unsafe { AXUIElementCopyParameterizedAttributeValue(element.0, CFRetained::as_ptr(&attr).as_ptr() as Ref, parameter.0, &mut out) };
        (err == 0 && !out.is_null()).then(|| Owned(out))
    }

    /// The text-marker range between markers `a` and `b`, in either order.
    fn marker_range(element: &Owned, a: &Owned, b: &Owned) -> Option<Owned> {
        let pair = [a.0, b.0];
        let array = Owned(unsafe { CFArrayCreate(null(), pair.as_ptr(), 2, &kCFTypeArrayCallBacks) });
        parameterized(element, "AXTextMarkerRangeForUnorderedTextMarkers", &array)
    }

    fn bounds_of(element: &Owned, range: &Owned) -> Option<Rect> {
        parameterized(element, "AXBoundsForTextMarkerRange", range).and_then(|b| value::<Rect>(&b, CG_RECT)).filter(|r| r.size.h > 0.0)
    }

    /// The insertion point in a web view. WebKit (Mail) answers the selection's bounds with the
    /// caret itself; Chromium with the whole line, so the caret is taken from the character
    /// before it (its right edge), or at the very start, the one after it (its left edge).
    fn web_caret(element: &Owned) -> Option<Rect> {
        let selection = attribute(element, "AXSelectedTextMarkerRange")?;
        let whole = bounds_of(element, &selection)?;
        if whole.size.w <= 2.0 {
            return Some(whole);
        }
        let caret = parameterized(element, "AXEndTextMarkerForTextMarkerRange", &selection)?;
        let before = parameterized(element, "AXPreviousTextMarkerForTextMarker", &caret)
            .and_then(|prev| marker_range(element, &prev, &caret))
            .and_then(|r| bounds_of(element, &r))
            .map(|c| Rect { origin: Point { x: c.origin.x + c.size.w, y: c.origin.y }, size: Size { w: 0.0, h: c.size.h } });
        before.or_else(|| {
            parameterized(element, "AXNextTextMarkerForTextMarker", &caret)
                .and_then(|next| marker_range(element, &caret, &next))
                .and_then(|r| bounds_of(element, &r))
                .map(|c| Rect { size: Size { w: 0.0, h: c.size.h }, ..c })
        })
    }

    fn value<T: Default>(v: &Owned, kind: u32) -> Option<T> {
        let mut out = T::default();
        unsafe { AXValueGetValue(v.0, kind, (&mut out as *mut T).cast()) }.then_some(out)
    }

    fn rect(x: f64, y: f64, w: f64, h: f64) -> RECT {
        RECT { left: x.round() as i32, top: y.round() as i32, right: (x + w).round() as i32, bottom: (y + h).round() as i32 }
    }

    /// Last character of a CFString; half of a surrogate pair reads as U+FFFD, still not a space.
    fn last_char(s: &Owned) -> Option<char> {
        unsafe {
            if CFGetTypeID(s.0) != CFStringGetTypeID() {
                return None;
            }
            let n = CFStringGetLength(s.0);
            if n <= 0 {
                return None;
            }
            let unit = CFStringGetCharacterAtIndex(s.0, n - 1);
            Some(char::from_u32(unit as u32).unwrap_or('\u{FFFD}'))
        }
    }

    pub(super) fn char_before() -> Option<char> {
        let system = Owned(unsafe { AXUIElementCreateSystemWide() });
        unsafe { AXUIElementSetMessagingTimeout(system.0, 0.25) };
        let focused = attribute(&system, "AXFocusedUIElement")?;
        // native text fields: a character range
        if let Some(sel) = attribute(&focused, "AXSelectedTextRange").and_then(|v| value::<CfRange>(&v, CF_RANGE)) {
            if sel.location <= 0 {
                return None;
            }
            let range = CfRange { location: sel.location - 1, length: 1 };
            let param = Owned(unsafe { AXValueCreate(CF_RANGE, (&range as *const CfRange).cast()) });
            return parameterized(&focused, "AXStringForRange", &param).and_then(|s| last_char(&s));
        }
        // web views: text markers
        let selection = attribute(&focused, "AXSelectedTextMarkerRange")?;
        let start = parameterized(&focused, "AXStartTextMarkerForTextMarkerRange", &selection)?;
        let prev = parameterized(&focused, "AXPreviousTextMarkerForTextMarker", &start)?;
        let range = marker_range(&focused, &prev, &start)?;
        parameterized(&focused, "AXStringForTextMarkerRange", &range).and_then(|s| last_char(&s))
    }

    pub(super) fn find(pid: isize) -> Option<Anchor> {
        wake(pid as i32);
        let system = Owned(unsafe { AXUIElementCreateSystemWide() });
        // an app that's busy mustn't hold up the release for long
        unsafe { AXUIElementSetMessagingTimeout(system.0, 0.25) };
        let focused = attribute(&system, "AXFocusedUIElement")?;
        let bounds = |range: &str, bounds_for: &str| {
            attribute(&focused, range)
                .and_then(|range| parameterized(&focused, bounds_for, &range))
                .and_then(|b| value::<Rect>(&b, CG_RECT))
                .filter(|r| r.size.h > 0.0)
        };
        // native text fields answer with a character range; web views (WebKit in Mail, Chromium in
        // browsers and Electron apps) with text markers
        let caret = bounds("AXSelectedTextRange", "AXBoundsForRange").or_else(|| web_caret(&focused));
        if let Some(r) = caret {
            return Some(Anchor::Caret(rect(r.origin.x, r.origin.y, r.size.w, r.size.h)));
        }
        let at = attribute(&focused, "AXPosition").and_then(|v| value::<Point>(&v, CG_POINT))?;
        let size = attribute(&focused, "AXSize").and_then(|v| value::<Size>(&v, CG_SIZE))?;
        (size.w > 0.0 && size.h > 0.0).then(|| Anchor::Area(rect(at.x, at.y, size.w, size.h)))
    }
}

/// Must be called while `target` (an HWND) still has focus. Tries the Win32 caret (Notepad, Office),
/// then the UI Automation caret (Chrome, Electron), then the focused control, then the window.
#[cfg(windows)]
pub fn find(target: isize) -> Option<Anchor> {
    let target = HWND(target as *mut _);
    physical(|| unsafe {
        if let Some(r) = system_caret(target) {
            return Some(Anchor::Caret(r));
        }
        if let Some(a) = uia_anchor() {
            return Some(a);
        }
        let mut r = RECT::default();
        GetWindowRect(target, &mut r).ok()?;
        non_empty(r).then_some(Anchor::Area(r))
    })
}

/// The character just before the insertion point in the focused text control, if UI Automation says.
#[cfg(windows)]
pub fn char_before() -> Option<char> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED); // already-initialized is fine
        let uia: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()?;
        let el = uia.GetFocusedElement().ok()?;
        let tp = el.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId).ok()?;
        let range = tp.GetSelection().ok()?.GetElement(0).ok()?;
        // collapse to the selection's start, then reach back one character
        range.MoveEndpointByRange(TextPatternRangeEndpoint_End, &range, TextPatternRangeEndpoint_Start).ok()?;
        if range.MoveEndpointByUnit(TextPatternRangeEndpoint_Start, TextUnit_Character, -1).ok()? == 0 {
            return None;
        }
        range.GetText(1).ok()?.to_string().chars().last()
    }
}

/// Whether the caret's centre lies within `field`, give or take a pixel of rounding.
#[cfg_attr(target_os = "macos", allow(dead_code))] // Accessibility's caret is checked by its height
fn inside(caret: RECT, field: RECT) -> bool {
    let (x, y) = ((caret.left + caret.right) / 2, (caret.top + caret.bottom) / 2);
    x >= field.left - 1 && x <= field.right + 1 && y >= field.top - 1 && y <= field.bottom + 1
}

#[cfg(windows)]
fn non_empty(r: RECT) -> bool {
    r.right > r.left && r.bottom > r.top
}

#[cfg(windows)]
unsafe fn system_caret(target: HWND) -> Option<RECT> {
    unsafe {
        let tid = GetWindowThreadProcessId(target, None);
        let mut gti = GUITHREADINFO { cbSize: size_of::<GUITHREADINFO>() as u32, ..Default::default() };
        GetGUIThreadInfo(tid, &mut gti).ok()?;
        if gti.hwndCaret.is_invalid() || gti.rcCaret.bottom <= gti.rcCaret.top {
            return None;
        }
        let mut a = POINT { x: gti.rcCaret.left, y: gti.rcCaret.top };
        let mut b = POINT { x: gti.rcCaret.right, y: gti.rcCaret.bottom };
        if !ClientToScreen(gti.hwndCaret, &mut a).as_bool() || !ClientToScreen(gti.hwndCaret, &mut b).as_bool() {
            return None;
        }
        Some(RECT { left: a.x, top: a.y, right: b.x, bottom: b.y })
    }
}

#[cfg(windows)]
unsafe fn uia_anchor() -> Option<Anchor> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED); // already-initialized is fine
        let uia: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()?;
        let el = uia.GetFocusedElement().ok()?;
        let field = el.CurrentBoundingRectangle().ok();
        // WebView2 apps (new Outlook) can report a caret on the wrong monitor; only believe one
        // inside the focused field, and otherwise ask the next way
        let believable = |r: &RECT| field.is_none_or(|f| inside(*r, f));
        if let Ok(tp) = el.GetCurrentPatternAs::<IUIAutomationTextPattern2>(UIA_TextPattern2Id) {
            let mut active = BOOL::default();
            if let Some(r) = tp.GetCaretRange(&mut active).ok().and_then(|range| caret_rect(&range)).filter(believable) {
                return Some(Anchor::Caret(r));
            }
        }
        // Chrome's contenteditable fields (Gmail) offer only the older pattern: the caret is the
        // selection's empty range
        if let Ok(tp) = el.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) {
            if let Some(r) = tp.GetSelection().ok().and_then(|sel| sel.GetElement(0).ok()).and_then(|range| caret_rect(&range)).filter(believable) {
                return Some(Anchor::Caret(r));
            }
        }
        let r = field?;
        non_empty(r).then_some(Anchor::Area(r))
    }
}

/// Rectangle of a caret range. An empty range has none; take the previous character's right edge.
#[cfg(windows)]
unsafe fn caret_rect(range: &IUIAutomationTextRange) -> Option<RECT> {
    unsafe {
        if let Some(r) = range_rect(range) {
            return Some(r);
        }
        range.MoveEndpointByUnit(TextPatternRangeEndpoint_Start, TextUnit_Character, -1).ok()?;
        let mut r = range_rect(range)?;
        r.left = r.right;
        Some(r)
    }
}

/// First rectangle of a text range; UIA returns them as flat [left, top, width, height] doubles.
#[cfg(windows)]
unsafe fn range_rect(range: &IUIAutomationTextRange) -> Option<RECT> {
    unsafe {
        let sa: *mut SAFEARRAY = range.GetBoundingRectangles().ok()?;
        if sa.is_null() {
            return None;
        }
        let mut out = None;
        if SafeArrayGetUBound(sa, 1).is_ok_and(|ub| ub >= 3) {
            let mut data: *mut core::ffi::c_void = std::ptr::null_mut();
            if SafeArrayAccessData(sa, &mut data).is_ok() {
                let d = std::slice::from_raw_parts(data as *const f64, 4);
                let r = RECT { left: d[0] as i32, top: d[1] as i32, right: (d[0] + d[2]) as i32, bottom: (d[1] + d[3]) as i32 };
                if r.bottom > r.top {
                    out = Some(r);
                }
                let _ = SafeArrayUnaccessData(sa);
            }
        }
        let _ = SafeArrayDestroy(sa);
        out
    }
}

/// Usable area (excluding the taskbar) of the monitor the anchor is on.
#[cfg(windows)]
pub fn work_area(anchor: &Anchor) -> RECT {
    physical(|| unsafe {
        let mon = MonitorFromRect(&anchor.rect(), MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut mi);
        mi.rcWork
    })
}

const GAP: i32 = 8;
/// Nudge left so the dialog's text roughly lines up with the caret instead of its border.
const INDENT: i32 = 24;

/// Top-left for a `w`×`h` dialog: below the caret line (above it if there is no room),
/// or centered over an area; always kept inside `work`.
pub fn place(anchor: &Anchor, w: i32, h: i32, work: RECT) -> (i32, i32) {
    let (x, y) = match *anchor {
        Anchor::Caret(c) => {
            let below = c.bottom + GAP;
            let y = if below + h <= work.bottom { below } else { c.top - GAP - h };
            (c.left - INDENT, y)
        }
        Anchor::Area(a) => ((a.left + a.right - w) / 2, (a.top + a.bottom - h) / 2),
    };
    (x.clamp(work.left, (work.right - w).max(work.left)), y.clamp(work.top, (work.bottom - h).max(work.top)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: RECT = RECT { left: 0, top: 0, right: 1920, bottom: 1040 };

    #[test]
    fn a_caret_outside_the_focused_field_is_not_believed() {
        let field = RECT { left: 910, top: 406, right: 1657, bottom: 895 };
        assert!(inside(RECT { left: 910, top: 406, right: 911, bottom: 427 }, field), "at the field's corner");
        assert!(inside(RECT { left: 1200, top: 600, right: 1200, bottom: 618 }, field), "zero-width caret");
        assert!(!inside(RECT { left: 729, top: -176, right: 729, bottom: -175 }, field), "on another monitor");
        assert!(!inside(RECT { left: 908, top: 350, right: 1645, bottom: 382 }, field), "the line above the field");
    }

    fn caret(x: i32, y: i32) -> Anchor {
        Anchor::Caret(RECT { left: x, top: y, right: x + 1, bottom: y + 20 })
    }

    #[test]
    fn caret_goes_below_line() {
        assert_eq!(place(&caret(500, 300), 540, 150, WORK), (476, 328));
    }

    #[test]
    fn caret_near_bottom_flips_above() {
        assert_eq!(place(&caret(500, 950), 540, 150, WORK), (476, 792));
    }

    #[test]
    fn caret_near_right_edge_is_clamped() {
        assert_eq!(place(&caret(1900, 300), 540, 150, WORK).0, 1380);
    }

    #[test]
    fn area_is_centered() {
        let a = Anchor::Area(RECT { left: 100, top: 100, right: 1100, bottom: 700 });
        assert_eq!(place(&a, 540, 150, WORK), (330, 325));
    }

    #[test]
    fn second_monitor_offsets_respected() {
        let work = RECT { left: 1920, top: 0, right: 3840, bottom: 1040 };
        assert_eq!(place(&caret(1925, 300), 540, 150, work).0, 1920);
    }
}
