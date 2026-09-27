# Murmur packaging: GitHub Release + in-app model download

Date: 2026-09-27. Status: design approved in chat, pending review of this written spec.

## Goal

Someone who finds Murmur on LinkedIn downloads one zip from GitHub Releases, runs `murmur.exe`, and gets to their first dictation with only clicks: no terminal and no script. An interrupted download recovers, and a corrupt download is caught.

## Decisions (Jeff, 2026-09-27)

| Question | Decision |
|---|---|
| Model setup | In-app download on first run. `setup-model.cmd` is **deleted**. |
| Progress UI | An egui window in the History style (status line, progress bar, Cancel). No consent prompt. |
| Mechanism | `ureq` for HTTP (progress, Range resume, streaming SHA-256 via `sha2`). `%SystemRoot%\System32\tar.exe` for extraction. |
| Source | k2-fsa release URLs directly, with pinned SHA-256. No mirror, no fallback. |
| Release build | GitHub Actions on a `v*` tag, which creates a **draft** Release. `workflow_dispatch` is used for dry runs. |

Out of scope: consent prompt, model choice, settings UI, fallback mirror, free-disk-space pre-check, proxy configuration.

## Pinned assets

Base URL: `https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/`

| File | Size (bytes) | SHA-256 |
|---|---|---|
| `silero_vad.onnx` | 643,854 | `9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6` |
| `sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8.tar.bz2` | 482,468,385 | `157c157bc51155e03e37d2466522a3a737dd9c72bb25f36eb18912964161e1ad` |

The digests come from the GitHub releases API (`assets[].digest`). The VAD digest also matches the local installed file.

## Components

### `src/model_fetch.rs` (library, no UI)

- `pub struct Asset { url, file, sha256, size }` and two consts, `VAD` and `PARAKEET`. Tests build their own `Asset` pointing at 127.0.0.1.
- `download(asset, dir, progress: impl FnMut(u64), cancel: &AtomicBool) -> Result<PathBuf>`:
  1. The target is `dir/<file>`; if it already exists, return it. The working file is `dir/<file>.part`.
  2. If the `.part` exists with *n* bytes: if *n* > `size`, delete it and start from 0. Otherwise hash the existing bytes and request `Range: bytes=n-`.
  3. Response **206** → append. **200** → truncate and write from 0. Any other status → an error that includes the status code.
  4. Stream in 64 KB chunks: write, update the SHA-256, call `progress` with the total bytes so far (the caller throttles the UI), and check `cancel` on each chunk. Cancel returns `Cancelled` and keeps the `.part`.
  5. At the end, a size or SHA-256 mismatch deletes the `.part` and returns `ChecksumMismatch`. A match renames `.part` → `<file>`.
  - Timeouts: 15 s connect, 30 s for response headers. The body is fetched in 8 MB Range requests with a 120 s budget each; ureq 3.4 has no per-read timeout (only `timeout_recv_body`, a total per request), so chunking is what turns a stall into "Download interrupted". A server that answers a Range request with 200 is read in one response without a body timeout.
- `extract(archive, dir) -> Result<PathBuf>`:
  1. Remove any leftover `dir/.staging`, then create it.
  2. Run `tar.exe -xjf <archive> -C <dir>/.staging` with `CREATE_NO_WINDOW`. A missing `tar.exe` or a non-zero exit returns an error, removes `.staging` and **keeps the archive**.
  3. Rename `.staging/<model>` → `dir/<model>`, remove `.staging`, delete the archive.
- `is_installed(model_dir) -> bool`: true when `encoder.int8.onnx` exists. It only becomes true after the final rename, so a crash mid-extract never looks installed.

### `src/setup_ui.rs`

- `run() -> SetupOutcome { Installed, Quit }`. It blocks, like `history_ui::show`.
- A worker thread calls `download(VAD)`, `download(PARAKEET)`, then `extract`, and sends messages over a channel. The UI shows:
  - "Downloading speech model: X / Y MB" with a bar over the combined size of both assets (updated about every 250 ms)
  - then "Unpacking…" without a percentage
  - then closes itself
- On error: the message, plus **Retry** (re-runs the worker, which resumes from the `.part`) and **Quit**.
- **Cancel** and the window's close button set the cancel flag and return `Quit`.

### `src/main.rs` changes

- The model check moves **before** `pipeline::spawn`.
- Model missing, and `model_dir` is the default: call `setup_ui::run()`. `Installed` → continue startup. `Quit` → exit the process.
- Model missing, and `model_dir` is customised in `config.toml`: no download; tray notice "Model missing at <path>".
- The "run setup-model.cmd" tray text is removed.

## Error handling

| Case | Message | State left behind |
|---|---|---|
| Network drop / stall (chunk over 120 s) | "Download interrupted" | `.part` kept; Retry resumes |
| HTTP 404 / other status | "Model file not available at k2-fsa (HTTP n)" | nothing new; needs a Murmur update if the asset moved |
| Checksum mismatch | "Download corrupted; Retry starts it over" | `.part` deleted |
| `tar.exe` missing or non-zero exit | "Couldn't unpack the model (tar exit n)" | archive kept, `.staging` removed |
| Disk full / IO error | OS message passed through | `.part` kept |
| Cancel / close window | (exits) | `.part` kept; next launch resumes |

Logging: start, finish, byte counts and errors at INFO. None of it contains user data.

## Release workflow: `.github/workflows/release.yml`

- **Triggers:** `push: tags: ['v*']` and `workflow_dispatch`.
- **Runner:** `windows-latest`.
- **Steps:**
  1. checkout → stable toolchain → `Swatinem/rust-cache`
  2. `cargo build --release --locked` (`sherpa-onnx-sys` downloads its prebuilt DLLs)
  3. copy the 4 DLLs (`onnxruntime.dll`, `onnxruntime_providers_shared.dll`, `sherpa-onnx-c-api.dll`, `sherpa-onnx-cxx-api.dll`) into `target/release/deps`
  4. `cargo test --release --bin murmur`, `--lib`, `--test stt_integration`. The last one skips without a model, so on CI it proves nothing; a comment in the workflow says so.
- **Version guard (tag runs):** the tag `vX.Y.Z` must equal `Cargo.toml` `version`, or the job fails.
- **Package:** `murmur-vX.Y.Z-windows-x64.zip` holds `murmur.exe`, the 4 DLLs, `README.md` and `LICENSE`, with `murmur-vX.Y.Z-windows-x64.zip.sha256` alongside. Dispatch runs use the version from `Cargo.toml`.
- **Tag run:** README and LICENSE are required, and a missing one fails the job. It runs `gh release create <tag> --draft` with both files; `permissions: contents: write`. Jeff publishes the draft by hand.
- **Dispatch run:** README and LICENSE are optional. It uploads the zip as a workflow artifact only, which allows a dry run on the private repo before #3 exists.
- **Not done:** a `build.rs` DLL copy into `deps`. A crate's build script isn't guaranteed to run after `sherpa-onnx-sys` places its DLLs. Locally the manual copy stays, and it's documented in the #3 README.

## Testing

**Automated (`model_fetch`)**
- Downloads run against an in-process `std::net::TcpListener` server that can honour or ignore Range, drop mid-stream, or return 404. Cases:
  - a fresh download
  - resume via 206
  - Range ignored (200) → restart
  - checksum mismatch → `.part` deleted
  - oversize `.part` → restart
  - cancel → `.part` kept
  - 404 → error with status
- `extract` uses a tiny `.tar.bz2` built with `tar.exe -cjf` in the test. It checks that a good archive gets renamed into place and the archive deleted, and that a bad archive gives an error, the archive is kept, and no final dir exists.
- The existing baselines must stay green: 31 bin, 48 lib, 1 `stt_integration`.

**Manual smoke (Jeff's machine; manipulate `%LOCALAPPDATA%\Murmur` via explorer-launched scripts)**
1. Move `models` aside and launch. The window downloads, unpacks and closes, and dictation works.
2. Kill Murmur mid-download and relaunch. It resumes, and the log shows the Range offset.
3. Cancel. Murmur exits and the `.part` is kept.
4. Corrupt the `.part`. You get the checksum error, then a clean retry.
5. Restore the original `models`.

**Clean machine (Windows Sandbox):** unzip the CI dispatch artifact and run it in Sandbox. This is the only check for a missing **VC++ runtime** (`vcruntime140.dll`, needed by the Rust exe and the ONNX DLLs).

## Open

- **VC++ runtime on a clean machine:** unknown until the Sandbox run. If it's missing, pick the fix then (static CRT for the Rust side and/or ship the runtime DLLs next to the exe).
- **`--lib` tests on CI without a model:** the `vad`/`stt` lib tests' behaviour without a model is unverified. Check it during the plan (skip vs fail), and gate them if needed.
- **`ureq` 3.x API:** confirm the timeout, header and status-handling API at the source when writing the plan.
