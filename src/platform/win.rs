use super::Window;
use anyhow::Result;
use std::path::Path;
use windows::core::{BOOL, HSTRING, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, HWND};
use windows::Win32::System::Threading::{AttachThreadInput, CreateMutexW, GetCurrentThreadId, OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId, IsIconic, SetForegroundWindow, SetWindowPos, ShowWindow,
    SystemParametersInfoW, HWND_NOTOPMOST, HWND_TOPMOST, SPI_GETCLIENTAREAANIMATION, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_RESTORE,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
};

pub type Rect = windows::Win32::Foundation::RECT;

pub(crate) fn hwnd(w: Window) -> HWND {
    HWND(w.0 as *mut _)
}

/// Whether the key with this virtual-key code is held right now.
pub fn key_down(vk: u16) -> bool {
    unsafe { (GetAsyncKeyState(vk as i32) as u16 & 0x8000) != 0 }
}

/// Keeps `w` above other windows without taking the focus.
pub fn keep_on_top(w: Window) {
    unsafe {
        let _ = SetWindowPos(hwnd(w), Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

/// Moves `w`'s top-left to (x, y), screen pixels in the caller's DPI awareness.
pub fn move_to(w: Window, x: i32, y: i32) {
    unsafe {
        let _ = SetWindowPos(hwnd(w), None, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

/// `w`'s frame in screen pixels, in the caller's DPI awareness.
pub fn window_rect(w: Window) -> Option<Rect> {
    let mut r = Rect::default();
    unsafe { GetWindowRect(hwnd(w), &mut r) }.ok().map(|_| r)
}

pub fn is_minimized(w: Window) -> bool {
    unsafe { IsIconic(hwnd(w)) }.as_bool()
}

pub fn is_foreground(w: Window) -> bool {
    let fg = unsafe { GetForegroundWindow() };
    fg == hwnd(w)
}

/// Windows only lets the process that received the last input activate a window. The hotkey
/// press went to the target app, so attach to its input queue for the activation call;
/// without this the dialog opens behind the target and merely flashes on the taskbar.
pub fn raise(w: Window) {
    unsafe {
        let fg = GetForegroundWindow();
        let fg_tid = GetWindowThreadProcessId(fg, None);
        let me = GetCurrentThreadId();
        let attached = fg_tid != 0 && fg_tid != me && AttachThreadInput(fg_tid, me, true).as_bool();
        let _ = SetWindowPos(hwnd(w), Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
        let _ = SetWindowPos(hwnd(w), Some(HWND_NOTOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
        let _ = SetForegroundWindow(hwnd(w));
        if attached {
            let _ = AttachThreadInput(fg_tid, me, false);
        }
    }
}

/// Gives the focus back to the window a dictation went to (`inject::foreground_hwnd`).
pub fn focus_target(target: isize) {
    raise(Window(target));
}

/// Brings the window titled `title` to the front, if there is one, restoring it if minimized.
pub fn raise_titled(title: &str) {
    unsafe {
        if let Ok(h) = FindWindowW(PCWSTR::null(), &HSTRING::from(title)) {
            // SW_RESTORE would also un-maximize, so only use it on a minimized window
            if IsIconic(h).as_bool() {
                let _ = ShowWindow(h, SW_RESTORE);
            }
            raise(Window(h.0 as isize));
        }
    }
}

pub fn open_path(p: &Path) {
    let _ = std::process::Command::new("explorer.exe").arg(p).spawn();
}

/// Windows' "Show animations in Windows" setting, off meaning reduced motion.
pub fn reduced_motion() -> bool {
    let mut on = BOOL(1);
    let ok = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, Some(&mut on as *mut BOOL as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)) };
    ok.is_ok() && !on.as_bool()
}

/// Held for as long as this process is the one instance of `name`.
pub struct Instance(#[allow(dead_code)] HANDLE);

impl Drop for Instance {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// Claims the single instance `name`; None when another process already has it.
pub fn single_instance(name: &str) -> Result<Option<Instance>> {
    unsafe {
        let m = CreateMutexW(None, false, &HSTRING::from(name))?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = CloseHandle(m);
            return Ok(None);
        }
        Ok(Some(Instance(m)))
    }
}

/// Whether some process holds the single instance `name`, from outside it.
pub fn instance_exists(name: &str) -> bool {
    match unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, &HSTRING::from(name)) } {
        Ok(h) => {
            let _ = unsafe { CloseHandle(h) };
            true
        }
        Err(_) => false,
    }
}
