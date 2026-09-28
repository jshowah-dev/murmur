# Murmur

Offline push-to-talk dictation for Windows. Hold a key, speak, let go — the text is typed wherever your cursor is. Speech recognition runs entirely on your machine; nothing you say leaves it.

<!-- demo GIF -->

## Features

- **Push-to-talk** — hold Right Ctrl (configurable), speak, release. Works in any app that accepts typing.
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

1. Download `murmur-vX.Y.Z-windows-x64.zip` from [Releases](../../releases) and unzip it anywhere. Keep the DLLs next to `murmur.exe`.
2. Run `murmur.exe`. Murmur isn't code-signed yet, so Windows SmartScreen may say "Windows protected your PC". Click **More info → Run anyway**.
3. On first launch Murmur downloads the speech model (about 460 MB, one time) to `%LOCALAPPDATA%\Murmur\models`, then shows a short "you're ready" screen.

To start Murmur with Windows, put a shortcut to `murmur.exe` in `shell:startup`.

## Updating

Murmur doesn't update itself. To get notified of new versions, click **Watch → Custom → Releases** on this repository.

1. Quit Murmur (tray icon → Quit).
2. Download the new zip from [Releases](../../releases) and unzip it over the old folder, replacing the files.
3. Start `murmur.exe`.

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
| Open dictionary | Edit `dictionary.toml` |
| Open snippets | Edit `snippets.toml` |
| Open config folder | Open `%APPDATA%\Murmur` |
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

`%APPDATA%\Murmur\dictionary.toml` maps what you say to what should be written. Fix-last adds entries automatically.

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
