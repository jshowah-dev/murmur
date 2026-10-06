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
    TextPatternRangeEndpoint_Start, TextUnit_Character, UIA_TextPattern2Id, UIA_TextPatternId,
};
#[cfg(windows)]
use windows::Win32::UI::HiDpi::{SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{GetGUIThreadInfo, GetWindowRect, GetWindowThreadProcessId, GUITHREADINFO};

/// Where the dictation landed, in physical screen pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(target_os = "macos", allow(dead_code))] // nothing finds a caret on macOS until phase 3
pub enum Anchor {
    /// The text caret: the dialog goes just below this line.
    Caret(RECT),
    /// Only the focused control or window is known: the dialog is centered over it.
    Area(RECT),
}

impl Anchor {
    #[cfg(windows)]
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

// TODO(macos phase 3): the focused element's caret via Accessibility (kAXBoundsForRangeParameterizedAttribute).
#[cfg(target_os = "macos")]
pub fn find(_target: isize) -> Option<Anchor> {
    None
}

// TODO(macos phase 2): the visible frame of the screen the anchor is on.
#[cfg(target_os = "macos")]
pub fn work_area(_anchor: &Anchor) -> RECT {
    RECT { left: 0, top: 0, right: 1440, bottom: 900 }
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

/// Whether the caret's centre lies within `field`, give or take a pixel of rounding.
#[cfg_attr(target_os = "macos", allow(dead_code))] // until the Accessibility caret lands
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
