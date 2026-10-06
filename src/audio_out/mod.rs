//! Mutes the default speaker while dictating so Spotify/YouTube/etc. go quiet.
//! Restores the mute state that was in effect before the press.

#[cfg(windows)]
mod win;
#[cfg(windows)]
pub use win::*;

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
pub use mac::*;
