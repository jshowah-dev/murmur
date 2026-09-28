fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        for ico in ["assets/murmur.ico", "assets/tray-live.ico", "assets/tray-paused.ico"] {
            println!("cargo:rerun-if-changed={ico}");
        }
        // the ids are the ones src/tray.rs loads
        winresource::WindowsResource::new()
            .set_icon("assets/murmur.ico")
            .set_icon_with_id("assets/tray-live.ico", "2")
            .set_icon_with_id("assets/tray-paused.ico", "3")
            .compile()
            .expect("embed icon");
    }
}
