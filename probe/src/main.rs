// The probe measures Windows' clipboard and SendInput path; there is nothing to probe elsewhere.
#[cfg(windows)]
mod win;

fn main() {
    #[cfg(windows)]
    win::main();
    #[cfg(not(windows))]
    eprintln!("murmur-probe is Windows-only");
}
