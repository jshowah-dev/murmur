//! Open at Login through SMAppService's main-app login item (macOS 13+). It registers the bundle
//! Murmur is running from, so a rebuilt `Murmur.app` at the same path stays registered.
use anyhow::{anyhow, Result};
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2_foundation::NSError;

#[link(name = "ServiceManagement", kind = "framework")]
unsafe extern "C" {}

/// SMAppServiceStatus
const ENABLED: isize = 1;
const REQUIRES_APPROVAL: isize = 2;

fn main_app() -> Result<Retained<AnyObject>> {
    let class = AnyClass::get(c"SMAppService").ok_or_else(|| anyhow!("SMAppService needs macOS 13"))?;
    Ok(unsafe { msg_send![class, mainAppService] })
}

fn status(service: &AnyObject) -> isize {
    unsafe { msg_send![service, status] }
}

pub fn is_enabled() -> bool {
    main_app().is_ok_and(|s| status(&s) == ENABLED)
}

pub fn set(enabled: bool) -> Result<()> {
    let service = main_app()?;
    let r: Result<(), Retained<NSError>> =
        if enabled { unsafe { msg_send![&service, registerAndReturnError: _] } } else { unsafe { msg_send![&service, unregisterAndReturnError: _] } };
    r.map_err(|e| anyhow!("{}", e.localizedDescription()))?;
    if enabled && status(&service) == REQUIRES_APPROVAL {
        let class = AnyClass::get(c"SMAppService").expect("checked in main_app");
        let _: () = unsafe { msg_send![class, openSystemSettingsLoginItems] };
        return Err(anyhow!("allow Murmur under System Settings › General › Login Items"));
    }
    Ok(())
}
