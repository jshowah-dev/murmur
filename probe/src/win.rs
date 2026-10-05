use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::sleep;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
    VIRTUAL_KEY, VK_CONTROL, VK_RCONTROL, VK_V,
};

const CF_UNICODETEXT: u32 = 13;

fn key_down(vk: VIRTUAL_KEY) -> bool {
    unsafe { (GetAsyncKeyState(vk.0 as i32) as u16 & 0x8000) != 0 }
}

fn set_clipboard(text: &str) {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        OpenClipboard(None).expect("OpenClipboard");
        EmptyClipboard().expect("EmptyClipboard");
        let h: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2).expect("GlobalAlloc");
        let p = GlobalLock(h) as *mut u16;
        std::ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len());
        let _ = GlobalUnlock(h);
        SetClipboardData(CF_UNICODETEXT, Some(HANDLE(h.0))).expect("SetClipboardData");
        CloseClipboard().expect("CloseClipboard");
    }
}

fn key_input(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { Default::default() },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn send_ctrl_v() {
    let inputs = [
        key_input(VK_CONTROL, false),
        key_input(VK_V, false),
        key_input(VK_V, true),
        key_input(VK_CONTROL, true),
    ];
    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

pub fn main() {
    println!("murmur-probe: hold Right Ctrl to record, release to paste a marker. Ctrl+C to quit.");
    let host = cpal::default_host();
    let device = host.default_input_device().expect("no input device");
    let config = device.default_input_config().expect("no input config").config();
    let count = Arc::new(AtomicUsize::new(0));

    loop {
        while !key_down(VK_RCONTROL) {
            sleep(Duration::from_millis(30));
        }
        let pressed_at = Instant::now();
        count.store(0, Ordering::SeqCst);
        let c = count.clone();
        let stream = device
            .build_input_stream(
                config,
                move |data: &[f32], _| {
                    c.fetch_add(data.len(), Ordering::SeqCst);
                },
                |e| eprintln!("stream error {e:?}"),
                None,
            )
            .expect("build_input_stream");
        stream.play().expect("play");
        while key_down(VK_RCONTROL) {
            sleep(Duration::from_millis(30));
        }
        drop(stream);
        let held = pressed_at.elapsed();
        if held < Duration::from_millis(150) {
            continue;
        }
        let samples = count.load(Ordering::SeqCst);
        let marker = format!("[murmur probe: held {} ms, {} samples] ", held.as_millis(), samples);
        set_clipboard(&marker);
        sleep(Duration::from_millis(50));
        send_ctrl_v();
        println!("{marker}");
    }
}
