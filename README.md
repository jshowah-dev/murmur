# Murmur

Offline push-to-talk dictation for Windows. Hold a key, speak, let go — the text is typed wherever your cursor is. Speech recognition runs entirely on your machine; nothing you say leaves it.

<!-- demo GIF -->

## Features

- **Push-to-talk** — hold Right Ctrl (configurable), speak, release. Works in any app that accepts typing.
- **Live text** — the words appear above the pill as you speak, then are typed when you let go.
- **Hands-free** — double-tap the key to keep recording without holding it. Tap again to finish, Esc to cancel. Stops on its own after 5 minutes.
- **Spoken commands** — say "new line" or "new paragraph".
- **Filler removal** — drops "um", "uh", "er" and friends.
- **Custom dictionary** — teach it names and jargon, with phonetic matching for near-misses.
- **Fix last** — hold Shift with the key to edit the last dictation; Murmur learns your corrections into the dictionary.
- **Snippets** — say a trigger phrase and a saved block of text is pasted instead.
- **History** — the last 25 dictations, from the tray.
- **Quiet while you talk** — mutes your speakers while the key is held.
- **Fast, local model** — NVIDIA Parakeet TDT 0.6B v2 (int8) via [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx), CPU only.

## Install

Requires Windows 10 or 11 (x64). No GPU needed.

1. Download `murmur-vX.Y.Z-setup.exe` from [Releases](../../releases) and run it. It installs for your user only (no admin prompt), adds Murmur to the Start menu and, if you leave the box ticked, starts it with Windows. Prefer no installer? Download `murmur-vX.Y.Z-windows-x64.zip` instead and unzip it anywhere, keeping the DLLs next to `murmur.exe`.
2. Murmur isn't code-signed yet, so Windows SmartScreen may say "Windows protected your PC". Click **More info → Run anyway**.
3. On first launch Murmur downloads the speech model (about 460 MB, one time) to `%LOCALAPPDATA%\Murmur\models`, then shows a short "you're ready" screen.

To start Murmur with Windows, tick **Start with Windows** in the tray menu.

### Uninstall

Settings → Apps → Murmur → Uninstall. It asks whether to also delete the speech model and your settings, dictionary and snippets; both default to keeping them. If you used the zip, quit Murmur, untick **Start with Windows** first, then delete the folder.

## Updating

Murmur doesn't update itself. To get notified of new versions, click **Watch → Custom → Releases** on this repository.

1. Download the new `murmur-vX.Y.Z-setup.exe` from [Releases](../../releases) and run it. It closes Murmur, updates it in place and keeps your start-with-Windows choice.
2. Zip users: quit Murmur (tray icon → Quit), unzip the new zip over the old folder, replacing the files, and start `murmur.exe`.

Your settings, dictionary and snippets live in `%APPDATA%\Murmur` and the model in `%LOCALAPPDATA%\Murmur\models`, so they are kept and nothing is downloaded again.

## Usage

- **Dictate:** hold **Right Ctrl**, speak, release. The text appears at the cursor.
- **Hands-free:** double-tap Right Ctrl. Tap once more to paste, or press Esc to discard.
- **Fix the last dictation:** hold **Shift** and press Right Ctrl.

Murmur lives in the system tray (teal microphone; grey when paused). Right-click it for:

| Menu item | What it does |
|---|---|
| Pause / Resume | Stop listening for the hotkey |
| Fix last (Left Shift+PTT) | Edit the last dictation |
| History… | Browse recent dictations |
| Dictionary… | Add, edit, delete and test dictionary terms |
| Open snippets | Edit `snippets.toml` |
| Open config folder | Open `%APPDATA%\Murmur` |
| Start with Windows | Start Murmur when you sign in (on/off) |
| Quit | Exit Murmur |

## Configuration

Settings live in `%APPDATA%\Murmur\config.toml`, created with defaults on first run. Restart Murmur after editing it.

| Key | Default | Meaning |
|---|---|---|
| `ptt_key` | `"RControl"` | Push-to-talk key: `RControl`, `LControl`, `RAlt`, `LAlt`, `RShift`, `CapsLock`, `ScrollLock`, `Pause`, or `F1`–`F24` |
| `model_dir` | `%LOCALAPPDATA%\Murmur\models\sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8` | Speech model folder. Only the default location is downloaded automatically |
| `threads` | `8` | CPU threads for recognition |
| `min_silence_ms` | `500` | Silence that ends a speech segment |
| `fillers` | `["um", "uh", "er", "hmm", "mm"]` | Words removed from the output |
| `spoken_commands` | `true` | Turn "new line" / "new paragraph" into line breaks |
| `mic_always_on` | `true` | Keep the mic open between dictations so the first word isn't clipped; closes after `idle_unload_minutes` |
| `mute_output` | `true` | Mute the speakers while the key is held |
| `hands_free_max_minutes` | `5` | Hands-free recordings stop and paste after this long |
| `idle_unload_minutes` | `15` | Idle minutes before the model is unloaded and the mic closes, to free memory. `0` = never |
| `debug_log` | `false` | Also log dictated text. Leave off for privacy |

### Dictionary

The dictionary maps what you say to what should be written. Open it from the tray with **Dictionary…**: pick a term to edit it, **+ New term** to add one, and type or dictate into the test box to see what the dictionary does to a phrase. Changes apply to the next dictation once you save; dictation keeps working while the editor is open. Fix-last adds entries automatically.

- **Written as:** the text Murmur writes.
- **Heard as:** what the speech model tends to hear instead, e.g. `cooper netties` for Kubernetes.
- **Also match sound-alikes:** also catch words that sound like the term. On by default.

The terms live in `%APPDATA%\Murmur\dictionary.toml` (the editor's **Open dictionary file** link opens it). You can edit the file by hand; saving from the editor removes comments from it.

```toml
[[term]]
written = "Kubernetes"
spoken = ["cooper netties"]
phonetic = true   # also match words that sound alike (default)
```

### Snippets

`%APPDATA%\Murmur\snippets.toml`. Say the trigger anywhere in a dictation and the text is pasted exactly as written. Triggers match whole words, ignoring case and punctuation. Changes apply on the next dictation.

```toml
[[snippet]]
trigger = "my signature"
text = """
Best,
Your Name"""
```

## Privacy

- Audio is processed in memory and never saved or sent anywhere.
- The only network access is the one-time model download from the [sherpa-onnx releases](https://github.com/k2-fsa/sherpa-onnx/releases/tag/asr-models).
- `%APPDATA%\Murmur\murmur.log` records events, not what you said, unless you turn on `debug_log`.

## Build from source

Requires the Rust stable toolchain with the MSVC target (`x86_64-pc-windows-msvc`) and the Visual Studio C++ build tools.

```bash
cargo build --release
```

The sherpa-onnx prebuilt libraries are downloaded during the build. `target/release/` then holds `murmur.exe` plus the four DLLs it needs. The C runtime is linked statically (see `.cargo/config.toml`), so no Visual C++ redistributable is required.

To build the installer, install [Inno Setup 6](https://jrsoftware.org/isinfo.php) (`winget install JRSoftware.InnoSetup`) and run, from the repo root after `cargo build --release`:

```bash
ISCC.exe /DAppVersion=X.Y.Z installer\murmur.iss
```

Tests:

```bash
cargo test --release
```

If the ONNX tests crash, copy `target/release/*.dll` into `target/release/deps/` (Windows otherwise loads an older `onnxruntime.dll` from System32). The speech integration test is skipped unless the model is installed.

## Credits and licences

Murmur is released under the [MIT License](LICENSE).

It builds on:

- **[Parakeet TDT 0.6B v2](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v2)** by NVIDIA, licensed under [CC-BY-4.0](https://creativecommons.org/licenses/by/4.0/). Murmur uses the int8 ONNX conversion published by the sherpa-onnx project. The model is downloaded on first run and is not distributed with Murmur.
- **[sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx)** by k2-fsa (Apache-2.0), which bundles [ONNX Runtime](https://github.com/microsoft/onnxruntime) (MIT).
- **[Silero VAD](https://github.com/snakers4/silero-vad)** (MIT), for voice activity detection.
