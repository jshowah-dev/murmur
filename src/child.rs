//! On macOS, Murmur's egui windows run in child processes. Run in Murmur's own process, a window
//! leaves the menu bar icon unable to open its menu once it closes (root cause not found; see
//! `correction_ui::card_process`, the first to move out). Each window has a flag: the request
//! goes in on stdin and the reply comes back on stdout, both as TOML.

use anyhow::Context;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::io::{Read, Write};
use std::process::{Command, Stdio};

/// TOML wants a table at the top, and a window may close without an answer.
#[derive(Serialize, Deserialize)]
struct Msg<T> {
    v: Option<T>,
}

/// Runs Murmur as `flag`, hands it `req` and waits until the window closes. None when it closed
/// without an answer, or couldn't run or died (which reads the same).
pub fn ask<Req: Serialize, Rep: DeserializeOwned>(flag: &str, req: &Req) -> Option<Rep> {
    let run = || -> anyhow::Result<Option<Rep>> {
        let mut child = Command::new(std::env::current_exe()?).arg(flag).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn()?;
        child.stdin.take().expect("piped").write_all(encode(Some(req))?.as_bytes())?;
        let out = child.wait_with_output()?;
        decode(&String::from_utf8_lossy(&out.stdout))
    };
    run().unwrap_or_else(|e| {
        log::error!("{flag}: {e:#}");
        None
    })
}

/// The child's side: reads the request, shows the window, writes its answer.
pub fn serve<Req: DeserializeOwned, Rep: Serialize>(window: impl FnOnce(Req) -> Option<Rep>) -> anyhow::Result<()> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let req = decode(&input)?.context("no request")?;
    std::io::stdout().write_all(encode(window(req).as_ref())?.as_bytes())?;
    Ok(())
}

fn encode<T: Serialize>(v: Option<&T>) -> anyhow::Result<String> {
    Ok(toml::to_string(&Msg { v })?)
}

fn decode<T: DeserializeOwned>(s: &str) -> anyhow::Result<Option<T>> {
    Ok(toml::from_str::<Msg<T>>(s)?.v)
}

/// Round-trips `v` the way `ask` and `serve` carry it, for the windows' tests.
#[cfg(test)]
pub fn round_trip<T: Serialize + DeserializeOwned>(v: &T) -> Option<T> {
    decode(&encode(Some(v)).unwrap()).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_answer_and_nothing_back_both_read_as_none() {
        assert_eq!(decode::<u32>(&encode::<u32>(None).unwrap()).unwrap(), None);
        assert_eq!(decode::<u32>("").unwrap(), None, "a child that died says nothing");
    }
}
