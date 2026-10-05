//! Puts a finished dictation into the app in front.

#[cfg(windows)]
mod win;
#[cfg(windows)]
pub use win::*;

#[cfg(not(windows))]
mod mac;
#[cfg(not(windows))]
pub use mac::*;
