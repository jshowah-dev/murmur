//! Mutes the default speaker while dictating so Spotify/YouTube/etc. go quiet.
//! Restores the mute state that was in effect before the press.
//!
//! CoreAudio by FFI: the default output device's mute control, or, on a device without one,
//! its volume turned to zero and put back.
use anyhow::{bail, Result};
use std::ffi::c_void;

type ObjectId = u32;

#[repr(C)]
struct Address {
    selector: u32,
    scope: u32,
    element: u32,
}

#[link(name = "CoreAudio", kind = "framework")]
unsafe extern "C" {
    fn AudioObjectHasProperty(id: ObjectId, addr: *const Address) -> u8;
    fn AudioObjectIsPropertySettable(id: ObjectId, addr: *const Address, settable: *mut u8) -> i32;
    fn AudioObjectGetPropertyData(id: ObjectId, addr: *const Address, qsize: u32, qdata: *const c_void, size: *mut u32, data: *mut c_void) -> i32;
    fn AudioObjectSetPropertyData(id: ObjectId, addr: *const Address, qsize: u32, qdata: *const c_void, size: u32, data: *const c_void) -> i32;
}

const fn code(s: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*s)
}

const SYSTEM_OBJECT: ObjectId = 1;
const SCOPE_GLOBAL: u32 = code(b"glob");
const SCOPE_OUTPUT: u32 = code(b"outp");
const DEFAULT_OUTPUT: u32 = code(b"dOut");
const MUTE: u32 = code(b"mute");
const VOLUME: u32 = code(b"volm");
/// The main element, then the left and right channels.
const ELEMENTS: [u32; 3] = [0, 1, 2];

fn out(selector: u32, element: u32) -> Address {
    Address { selector, scope: SCOPE_OUTPUT, element }
}

fn settable(id: ObjectId, a: &Address) -> bool {
    let mut s = 0u8;
    unsafe { AudioObjectHasProperty(id, a) != 0 && AudioObjectIsPropertySettable(id, a, &mut s) == 0 && s != 0 }
}

fn get<T: Default>(id: ObjectId, a: &Address) -> Result<T> {
    let mut v = T::default();
    let mut size = size_of::<T>() as u32;
    let err = unsafe { AudioObjectGetPropertyData(id, a, 0, std::ptr::null(), &mut size, &mut v as *mut T as *mut c_void) };
    if err != 0 {
        bail!("AudioObjectGetPropertyData {:08x}: {err}", a.selector);
    }
    Ok(v)
}

fn set<T>(id: ObjectId, a: &Address, v: T) -> Result<()> {
    let err = unsafe { AudioObjectSetPropertyData(id, a, 0, std::ptr::null(), size_of::<T>() as u32, &v as *const T as *const c_void) };
    if err != 0 {
        bail!("AudioObjectSetPropertyData {:08x}: {err}", a.selector);
    }
    Ok(())
}

fn default_output() -> Result<ObjectId> {
    let id: ObjectId = get(SYSTEM_OBJECT, &Address { selector: DEFAULT_OUTPUT, scope: SCOPE_GLOBAL, element: 0 })?;
    if id == 0 {
        bail!("no default output device");
    }
    Ok(id)
}

/// What `mute` changed, to put back: on the device it was done to, even if the default changes.
enum Held {
    Mute { device: ObjectId, prior: u32 },
    Volume { device: ObjectId, prior: Vec<(u32, f32)> },
}

pub struct OutputMute {
    held: Option<Held>,
}

impl OutputMute {
    pub fn new() -> Self {
        Self { held: None }
    }

    fn silence() -> Result<Held> {
        let device = default_output()?;
        if settable(device, &out(MUTE, 0)) {
            let prior: u32 = get(device, &out(MUTE, 0))?;
            if prior == 0 {
                set(device, &out(MUTE, 0), 1u32)?;
            }
            return Ok(Held::Mute { device, prior });
        }
        let mut prior = Vec::new();
        for e in ELEMENTS.into_iter().filter(|&e| settable(device, &out(VOLUME, e))) {
            let v: f32 = get(device, &out(VOLUME, e))?;
            if let Err(err) = set(device, &out(VOLUME, e), 0f32) {
                // don't leave the channels already turned down silent
                for (e, v) in prior {
                    let _ = set(device, &out(VOLUME, e), v);
                }
                return Err(err);
            }
            prior.push((e, v));
        }
        if prior.is_empty() {
            bail!("output device {device} has no mute or volume control");
        }
        Ok(Held::Volume { device, prior })
    }

    /// Mute the default output, remembering its prior state. No-op if already holding a mute.
    pub fn mute(&mut self) {
        if self.held.is_some() {
            return;
        }
        match Self::silence() {
            Ok(h) => self.held = Some(h),
            Err(e) => log::warn!("mute output: {e:#}"),
        }
    }

    /// Put the output back to what it was before `mute()`.
    pub fn restore(&mut self) {
        let result = match self.held.take() {
            None | Some(Held::Mute { prior: 1.., .. }) => Ok(()),
            Some(Held::Mute { device, .. }) => set(device, &out(MUTE, 0), 0u32),
            Some(Held::Volume { device, prior }) => prior.into_iter().try_for_each(|(e, v)| set(device, &out(VOLUME, e), v)),
        };
        if let Err(e) = result {
            log::warn!("restore output: {e:#}");
        }
    }
}

impl Drop for OutputMute {
    fn drop(&mut self) {
        self.restore();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trips the real default output: needs an audio device (skipped without one).
    #[test]
    fn mute_then_restore_round_trips() {
        let Ok(device) = default_output() else { return };
        let read = || -> (Option<u32>, Vec<f32>) {
            let mute = settable(device, &out(MUTE, 0)).then(|| get(device, &out(MUTE, 0)).unwrap());
            let vol = ELEMENTS.iter().filter(|&&e| settable(device, &out(VOLUME, e))).map(|&e| get(device, &out(VOLUME, e)).unwrap()).collect();
            (mute, vol)
        };
        let before = read();
        let mut m = OutputMute::new();
        m.mute();
        match read() {
            (Some(muted), _) => assert_eq!(muted, 1),
            (None, vol) => assert!(vol.iter().all(|&v| v == 0.0), "{vol:?}"),
        }
        m.restore();
        assert_eq!(read(), before);
        // second restore with nothing held is a no-op
        m.restore();
    }
}
