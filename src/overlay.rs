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
    SWP_NOMOVE, SWP_NOSIZE, SW_HIDE, SW_SHOWNOACTIVATE, ULW_ALPHA, WM_QUIT, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::Win32::Graphics::Gdi::{MonitorFromWindow, GetMonitorInfoW, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::Graphics::Gdi::AC_SRC_ALPHA;
use windows::Win32::Graphics::Gdi::BLENDFUNCTION;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OverlayState {
    Hidden,
    Listening(f32),
    Processing,
}

const W: i32 = 160;
const H: i32 = 36;

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
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                PCWSTR(class.as_ptr()),
                PCWSTR(wide("Murmur").as_ptr()),
                WS_POPUP,
                0, 0, W, H,
                None, None, Some(hinst.into()), None,
            )?;
            Ok(Overlay { hwnd, state: OverlayState::Hidden, tick: 0 })
        }
    }

    fn target_position(&self) -> (i32, i32) {
        unsafe {
            let fg = GetForegroundWindow();
            let mon = MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST);
            let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            let r: RECT = if GetMonitorInfoW(mon, &mut mi).as_bool() { mi.rcWork } else { RECT { left: 0, top: 0, right: 1920, bottom: 1080 } };
            let _ = GetWindowRect(fg, &mut RECT::default());
            ((r.left + r.right) / 2 - W / 2, r.bottom - H - 24)
        }
    }

    /// Paint the pill into a 32-bit DIB and push it with UpdateLayeredWindow.
    fn paint(&mut self) {
        unsafe {
            let screen = GetDC(None);
            let mem = CreateCompatibleDC(Some(screen));
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: W,
                    biHeight: -H,
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
            let px = std::slice::from_raw_parts_mut(bits as *mut u32, (W * H) as usize);
            px.fill(0);
            // pill body
            let rgn = CreateRoundRectRgn(0, 0, W, H, H, H);
            let body = CreateSolidBrush(COLORREF(0x00202020));
            let _ = FillRgn(mem, rgn, body);
            // opaque alpha for the body pixels
            for p in px.iter_mut() {
                if *p != 0 {
                    *p |= 0xE600_0000;
                }
            }
            // content
            match self.state {
                OverlayState::Listening(level) => {
                    let bar_w = ((W - 40) as f32 * level.clamp(0.0, 1.0)) as i32;
                    let r = RECT { left: 20, top: H / 2 - 3, right: 20 + bar_w.max(4), bottom: H / 2 + 3 };
                    let fg = CreateSolidBrush(COLORREF(0x0060D060));
                    FillRect(mem, &r, fg);
                    let _ = DeleteObject(fg.into());
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
                OverlayState::Hidden => {}
            }
            for p in px.iter_mut() {
                if *p & 0xFF00_0000 == 0 && *p != 0 {
                    *p |= 0xFF00_0000;
                }
            }
            let _ = DeleteObject(body.into());
            let _ = DeleteObject(rgn.into());

            let (x, y) = self.target_position();
            let blend = BLENDFUNCTION { BlendOp: 0, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
            let _ = UpdateLayeredWindow(
                self.hwnd,
                Some(screen),
                Some(&POINT { x, y }),
                Some(&SIZE { cx: W, cy: H }),
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
        let was_hidden = self.state == OverlayState::Hidden;
        self.state = state;
        self.tick = self.tick.wrapping_add(1);
        unsafe {
            match state {
                OverlayState::Hidden => {
                    let _ = ShowWindow(self.hwnd, SW_HIDE);
                }
                _ => {
                    self.paint();
                    if was_hidden {
                        let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOSIZE | SWP_NOMOVE | SWP_NOACTIVATE);
                        let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                    }
                }
            }
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
