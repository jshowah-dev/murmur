// TODO(macos phase 3): mute the default output device through CoreAudio.
pub struct OutputMute;

impl OutputMute {
    pub fn new() -> Self {
        OutputMute
    }

    pub fn mute(&mut self) {}

    pub fn restore(&mut self) {}
}
