fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=assets/murmur.ico");
        winresource::WindowsResource::new().set_icon("assets/murmur.ico").compile().expect("embed icon");
    }
}
