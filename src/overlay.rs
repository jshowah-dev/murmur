use anyhow::{anyhow, Result};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    COLORREF, ERROR_CLASS_ALREADY_EXISTS, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, CreateRoundRectRgn, CreateSolidBrush, DeleteDC, DeleteObject, FillRect, FillRgn,
    GetDC, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::Foundation::GetLastError;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetForegroundWindow, GetWindowRect, PeekMessageW, RegisterClassW,
    SetWindowPos, ShowWindow, TranslateMessage, UpdateLayeredWindow, HWND_TOPMOST, MSG, PM_REMOVE, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SW_SHOWNOACTIVATE, ULW_ALPHA, WM_QUIT, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::Win32::Graphics::Gdi::{MonitorFromWindow, GetMonitorInfoW, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::Graphics::Gdi::AC_SRC_ALPHA;
use windows::Win32::Graphics::Gdi::BLENDFUNCTION;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OverlayState {
    /// Always-on resting pill: small, dim, green dot.
    Idle,
    /// Resting pill while dictation is paused from the tray: small, dim, grey dot.
    Paused,
    Listening(f32),
    /// Hands-free listening: level bar plus a red dot, the key can be let go.
    Locked(f32),
    Processing,
}

impl OverlayState {
    fn is_resting(self) -> bool {
        matches!(self, OverlayState::Idle | OverlayState::Paused)
    }
    /// (width, height, body alpha) — the resting pill is small and dim, live states are full size.
    fn geometry(self) -> (i32, i32, u32) {
        if self.is_resting() { (IDLE_W, IDLE_H, 0x80) } else { (W, H, 0xE6) }
    }
}

const W: i32 = 160;
const H: i32 = 36;
const IDLE_W: i32 = 56;
const IDLE_H: i32 = 14;

pub struct Overlay {
    hwnd: HWND,
    state: OverlayState,
    tick: u32,
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

impl Overlay {
    pub fn create() -> Result<Overlay> {
        unsafe {
            let hinst = GetModuleHandleW(None)?;
            let class = wide("MurmurOverlay");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: hinst.into(),
                lpszClassName: PCWSTR(class.as_ptr()),
                ..Default::default()
            };
            if RegisterClassW(&wc) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
                return Err(anyhow!("RegisterClassW"));
            }
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT,
                PCWSTR(class.as_ptr()),
                PCWSTR(wide("Murmur").as_ptr()),
                WS_POPUP,
                0, 0, W, H,
                None, None, Some(hinst.into()), None,
            )?;
            Ok(Overlay { hwnd, state: OverlayState::Idle, tick: 0 })
        }
    }

    /// Bottom-centre of the work area on the foreground window's monitor; the pill's
    /// bottom edge stays put as it grows from the resting size to the live size.
    fn target_position(&self, w: i32, h: i32) -> (i32, i32) {
        unsafe {
            let fg = GetForegroundWindow();
            let mon = MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST);
            let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            let r: RECT = if GetMonitorInfoW(mon, &mut mi).as_bool() { mi.rcWork } else { RECT { left: 0, top: 0, right: 1920, bottom: 1080 } };
            let _ = GetWindowRect(fg, &mut RECT::default());
            ((r.left + r.right) / 2 - w / 2, r.bottom - h - 24)
        }
    }

    /// Paint the pill into a 32-bit DIB and push it with UpdateLayeredWindow.
    fn paint(&mut self) {
        let (w, h, alpha) = self.state.geometry();
        unsafe {
            let screen = GetDC(None);
            let mem = CreateCompatibleDC(Some(screen));
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let bmp: HBITMAP = CreateDIBSection(Some(mem), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).unwrap_or_default();
            if bmp.is_invalid() || bits.is_null() {
                let _ = DeleteDC(mem);
                ReleaseDC(None, screen);
                return;
            }
            let old = SelectObject(mem, bmp.into());

            // premultiplied BGRA: fully transparent background
            let px = std::slice::from_raw_parts_mut(bits as *mut u32, (w * h) as usize);
            px.fill(0);
            // pill body
            let rgn = CreateRoundRectRgn(0, 0, w, h, h, h);
            let body = CreateSolidBrush(COLORREF(0x00202020));
            let _ = FillRgn(mem, rgn, body);
            // body alpha: dim when resting, near-opaque when live (premultiplied)
            let ch = 0x20 * alpha / 255;
            let body_px = (alpha << 24) | (ch << 16) | (ch << 8) | ch;
            for p in px.iter_mut() {
                if *p != 0 {
                    *p = body_px;
                }
            }
            // content
            match self.state {
                OverlayState::Idle | OverlayState::Paused => {
                    let c = if self.state == OverlayState::Idle { 0x0060D060 } else { 0x00808080 };
                    let d = h - 8;
                    let dot_rgn = CreateRoundRectRgn(w / 2 - d / 2, 4, w / 2 + d / 2, 4 + d, d, d);
                    let dot = CreateSolidBrush(COLORREF(c));
                    let _ = FillRgn(mem, dot_rgn, dot);
                    let _ = DeleteObject(dot.into());
                    let _ = DeleteObject(dot_rgn.into());
                }
                OverlayState::Listening(level) | OverlayState::Locked(level) => {
                    let locked = matches!(self.state, OverlayState::Locked(_));
                    // the locked bar stops short of the dot at the right end
                    let span = if locked { W - 58 } else { W - 40 };
                    let bar_w = (span as f32 * level.clamp(0.0, 1.0)) as i32;
                    let r = RECT { left: 20, top: H / 2 - 3, right: 20 + bar_w.max(4), bottom: H / 2 + 3 };
                    let fg = CreateSolidBrush(COLORREF(0x0060D060));
                    FillRect(mem, &r, fg);
                    let _ = DeleteObject(fg.into());
                    if locked {
                        let d = 10;
                        let x = W - 20 - d;
                        let dot_rgn = CreateRoundRectRgn(x, H / 2 - d / 2, x + d, H / 2 + d / 2, d, d);
                        let dot = CreateSolidBrush(COLORREF(0x004040E0));
                        let _ = FillRgn(mem, dot_rgn, dot);
                        let _ = DeleteObject(dot.into());
                        let _ = DeleteObject(dot_rgn.into());
                    }
                }
                OverlayState::Processing => {
                    let on = (self.tick / 4) % 3;
                    for i in 0..3 {
                        let x = W / 2 - 18 + i * 14;
                        let r = RECT { left: x, top: H / 2 - 3, right: x + 6, bottom: H / 2 + 3 };
                        let c = if i as u32 == on { 0x00FFFFFF } else { 0x00707070 };
                        let b = CreateSolidBrush(COLORREF(c));
                        FillRect(mem, &r, b);
                        let _ = DeleteObject(b.into());
                    }
                }
            }
            for p in px.iter_mut() {
                if *p & 0xFF00_0000 == 0 && *p != 0 {
                    *p |= 0xFF00_0000;
                }
            }
            let _ = DeleteObject(body.into());
            let _ = DeleteObject(rgn.into());

            let (x, y) = self.target_position(w, h);
            let blend = BLENDFUNCTION { BlendOp: 0, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
            let _ = UpdateLayeredWindow(
                self.hwnd,
                Some(screen),
                Some(&POINT { x, y }),
                Some(&SIZE { cx: w, cy: h }),
                Some(mem),
                Some(&POINT { x: 0, y: 0 }),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            );
            let _ = SelectObject(mem, old);
            let _ = DeleteObject(bmp.into());
            let _ = DeleteDC(mem);
            ReleaseDC(None, screen);
        }
    }

    pub fn set(&mut self, state: OverlayState) {
        self.state = state;
        self.tick = self.tick.wrapping_add(1);
        self.paint();
        unsafe {
            // re-assert topmost on every state change so a fullscreen app cannot bury the pill
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOSIZE | SWP_NOMOVE | SWP_NOACTIVATE);
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
        }
    }

    /// Re-paint the resting pill so it follows the foreground window's monitor.
    pub fn refresh_resting(&mut self) {
        if self.state.is_resting() {
            self.paint();
        }
    }

    /// Drain pending messages for this thread. Returns false on WM_QUIT.
    pub fn pump_once(&mut self) -> bool {
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    return false;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        true
    }
}
