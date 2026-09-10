//! Mutes the default speaker while dictating so Spotify/YouTube/etc. go quiet.
//! Restores the mute state that was in effect before the press.
use anyhow::{Context, Result};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_APARTMENTTHREADED};

pub struct OutputMute {
    was_muted: Option<bool>,
}

impl OutputMute {
    pub fn new() -> Self {
        // Already-initialised on this thread (tray/overlay) reports RPC_E_CHANGED_MODE; harmless.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        Self { was_muted: None }
    }

    fn endpoint() -> Result<IAudioEndpointVolume> {
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).context("MMDeviceEnumerator")?;
            let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole).context("default render device")?;
            device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).context("IAudioEndpointVolume")
        }
    }

    /// Mute the default output, remembering its prior state. No-op if already holding a mute.
    pub fn mute(&mut self) {
        if self.was_muted.is_some() {
            return;
        }
        match Self::endpoint().and_then(|ep| unsafe {
            let prior = ep.GetMute()?.as_bool();
            if !prior {
                ep.SetMute(true, std::ptr::null())?;
            }
            Ok(prior)
        }) {
            Ok(prior) => self.was_muted = Some(prior),
            Err(e) => log::warn!("mute output: {e:#}"),
        }
    }

    /// Put the mute state back to what it was before `mute()`.
    pub fn restore(&mut self) {
        let Some(prior) = self.was_muted.take() else { return };
        if prior {
            return;
        }
        if let Err(e) = Self::endpoint().and_then(|ep| unsafe { Ok(ep.SetMute(false, std::ptr::null())?) }) {
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

    /// Round-trips the real default endpoint: needs an audio device (skipped on CI-less boxes).
    #[test]
    fn mute_then_restore_round_trips() {
        let Ok(ep) = OutputMute::endpoint() else { return };
        let before = unsafe { ep.GetMute().unwrap().as_bool() };
        let mut m = OutputMute::new();
        m.mute();
        assert!(unsafe { ep.GetMute().unwrap().as_bool() });
        m.restore();
        assert_eq!(unsafe { ep.GetMute().unwrap().as_bool() }, before);
        // second restore with nothing held is a no-op
        m.restore();
        assert_eq!(unsafe { ep.GetMute().unwrap().as_bool() }, before);
    }
}
