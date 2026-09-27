use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{ClientToScreen, GetMonitorInfoW, MonitorFromRect, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, SAFEARRAY};
use windows::Win32::System::Ole::{SafeArrayAccessData, SafeArrayDestroy, SafeArrayGetUBound, SafeArrayUnaccessData};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationTextPattern2, IUIAutomationTextRange, TextPatternRangeEndpoint_Start, TextUnit_Character,
    UIA_TextPattern2Id,
};
use windows::Win32::UI::HiDpi::{SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
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
pub fn physical<T>(f: impl FnOnce() -> T) -> T {
    unsafe {
        let old = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let r = f();
        SetThreadDpiAwarenessContext(old);
        r
    }
}

/// Must be called while `target` still has focus. Tries the Win32 caret (Notepad, Office),
/// then the UI Automation caret (Chrome, Electron), then the focused control, then the window.
pub fn find(target: HWND) -> Option<Anchor> {
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

fn non_empty(r: RECT) -> bool {
    r.right > r.left && r.bottom > r.top
}

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

unsafe fn uia_anchor() -> Option<Anchor> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED); // already-initialized is fine
        let uia: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()?;
        let el = uia.GetFocusedElement().ok()?;
        if let Ok(tp) = el.GetCurrentPatternAs::<IUIAutomationTextPattern2>(UIA_TextPattern2Id) {
            let mut active = BOOL::default();
            if let Ok(range) = tp.GetCaretRange(&mut active) {
                if let Some(r) = range_rect(&range) {
                    return Some(Anchor::Caret(r));
                }
                // an empty range at the end of the text has no rectangle; take the previous character's right edge
                if range.MoveEndpointByUnit(TextPatternRangeEndpoint_Start, TextUnit_Character, -1).is_ok() {
                    if let Some(mut r) = range_rect(&range) {
                        r.left = r.right;
                        return Some(Anchor::Caret(r));
                    }
                }
            }
        }
        let r = el.CurrentBoundingRectangle().ok()?;
        non_empty(r).then_some(Anchor::Area(r))
    }
}

/// First rectangle of a text range; UIA returns them as flat [left, top, width, height] doubles.
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
