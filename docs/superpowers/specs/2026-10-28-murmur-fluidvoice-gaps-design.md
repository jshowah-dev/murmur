# Murmur vs FluidVoice: closing the gaps

Date: 2026-10-28. Status: proposal. Nothing is built yet; the open questions at the end are Jeff's to decide.

## Why this exists

FluidVoice (fluidvoice.com, `altic-dev/FluidVoice`, 13k stars, 350k downloads) is the open-source dictation app Murmur is most often compared to. Jeff asked how it shows words as they are spoken, how it gets its speed, how it does inline corrections, and what Murmur would need to match it. This spec answers from FluidVoice's source code and from benchmarks run on the same speech engine Murmur uses.

## How FluidVoice does it

These findings come from the repo at about v1.6.13-beta.1. Paths are relative to their repo root.

**The live words are a re-decode on a timer, shown in an overlay.**
- `ASRService.startStreamingTranscription` re-decodes all the audio so far every 0.6 s for Parakeet TDT. It uses 0.2 s for Parakeet Flash and 0.32 s for Nemotron, which are true streaming models fed only the new audio.
- A tick is skipped if the previous decode is still running.
- Each partial gets the filler, dictionary and spoken-punctuation passes. It is then steadied by `smartDiffUpdate`, which keeps the common word prefix when it covers more than half the previous words.
- The partial is drawn in a notch or bottom overlay. **It is never typed into the app.**
- On release the preview is **thrown away**. The whole recording is decoded again by the final model, with vocabulary boosting and a 250 ms silence lead-in, and pasted once.

**The speed is mostly Apple hardware.**
- Parakeet and Nemotron run as CoreML on the Neural Engine through FluidAudio, and Whisper runs on Metal.
- Models load at launch and stay loaded, and audio capture is pre-warmed.
- Their own distilled "Fluid Speech Mini" is a 253 MB, roughly 3-bit Parakeet derivative (CC BY 4.0, with ONNX and GGUF builds published).
- The "<100 ms" and real-time-factor figures on the homepage have no measurement behind them in the code.
- Paste is clipboard plus Cmd+V with a 500 ms restore, the same approach as Murmur.

**Inline corrections are an LLM, not rules.**
- FluidVoice has no rule-based self-correction.
- "No wait, I mean…" is handled by a post-processing prompt: "Replace a clearly corrected phrase with the speaker's final version."
- The prompt runs on their closed-source Fluid-1 models (MLX on Apple Silicon, llama.cpp otherwise; Pico is a 600 MB download) or on a cloud or OpenAI-compatible provider (OpenAI, Groq, Ollama, LM Studio, custom).
- The LLM is warmed up when recording starts, so it is ready when the key is released.
- **Fluid-1 is not open source.** The repo only holds a provider interface for it.

**Other features.**
- Per-app prompts: a frontmost-app bundle id is bound to a prompt profile.
- Write/Edit mode: the selected text plus a spoken instruction go to the LLM, and the result replaces the selection.
- Command mode: an LLM agent that runs terminal commands.
- Nemotron 3.5 multilingual (about 40 languages), Parakeet v3 (25 languages) and Whisper.
- File and video transcription, SQLite history, and FluidMeet meeting notes.

**License.**
- GPLv3 since 2026-02-23. Murmur (MIT) must not copy anything from after that date.
- Commits up to `1698a31` and tags up to v1.5.8 are Apache-2.0. Those can be borrowed with attribution, including the older self-correction prompt with its few-shot examples and the 0.6 s preview loop. Clean-room is still the simpler path.

## What Murmur already matches or beats

Murmur already has these, with rules and without an LLM:
- self-corrections ("Tuesday, no, Wednesday")
- "scratch that"
- stutter removal
- spoken punctuation and symbols
- email formatting by app
- a phonetic dictionary that learns from fix-last
- snippets
- the mic kept open so the first word isn't clipped
- decoding at pauses while you talk

It also runs on Windows, where FluidVoice is a closed beta. Rules are instant and never reword what you said; an LLM can do both. Murmur's privacy story is also simpler: nothing optional ever goes to the cloud.

## Benchmarks

**Setup.**
- Machine: a 4-core x86 cloud VM, CPU only, sherpa-onnx 1.13.8 (Murmur's version). It is slower than Jeff's Windows laptop and much slower than an M-series Mac, so read the numbers as relative.
- Audio: three 10–11 s dictations synthesised with Piper TTS, including "Tuesday, no wait, Wednesday" and "forty thousand dollars, I mean fifty thousand".
- "Load" in the table means process start to model ready, measured once per model.

| Model | Role | Load | Compute per second of speech | Worst step | Output |
|---|---|---|---|---|---|
| Parakeet TDT 0.6B v2 int8 (today), 4 threads | final | 7.4 s | 0.11–0.22 s; tail of 1 / 2 / 4 / 8 s decodes in 0.23 / 0.38 / 0.47 / 0.84 s | n/a | best: cased, punctuated, "$50,000" |
| Parakeet TDT 110M int8, 2 threads | fast final or preview | 1.7 s | 0.044 s; tail of 1 / 2 / 4 / 8 s in 0.05 / 0.08 / 0.13 / 0.35 s | n/a | cased and punctuated; misheard "So um I" as "So I'm I" |
| **Kroko streaming zipformer (70 MB), 1 thread** | **live preview** | 6.0 s | **0.057 s** | **101 ms** | cased and punctuated; 60 ms to finish after the last audio |
| Nemotron streaming 0.6B 560 ms, 2 threads | streaming final | 4.5 s | 0.42 s | 309 ms | words appear about 0.6 s after they are said; almost no sentence punctuation; dropped "Hey" |
| Nemotron streaming 0.6B 160 ms, 2 threads | streaming final | 4.7 s | 1.12 s, slower than real time | 326 ms | not viable on this CPU |
| NeMo fast-conformer streaming 80 ms int8, 2 threads | preview | 1.7 s | 0.31 s | 69 ms | all lowercase, no punctuation |
| Zipformer streaming 20M, 1 thread | preview | 0.7 s | 0.04 s | 16 ms | ALL CAPS, missed the first 1.5 s of one clip |

**What the numbers say.**
1. **The FluidVoice approach (re-decode everything every 0.6 s) is unaffordable on a CPU with Parakeet 0.6B.** It took 85 s of compute for 22 s of speech. Re-decoding only the speech since Murmur's last pause commit is affordable while that stretch is under about 4 s.
2. **A small streaming model is the cheap way to show live words.** Kroko costs about 6% of one core, never re-decodes, and writes cased, punctuated text good enough to watch.
3. **Keep Parakeet 0.6B for the text that is pasted.** FluidVoice also throws the preview away. The preview only has to look right; the paste has to be right.
4. Nemotron streaming as Murmur's only model would remove the decode wait on release. But its punctuation is weak, and on CPU it costs 4–10× the preview model. Not worth it on Windows.
5. Parakeet's built-in hotword boosting had no effect in one test: "Showah" was still heard as "Shoal". It needs hotwords written as model tokens. Murmur's phonetic dictionary already covers this, so it is not pursued.

A local LLM was not benchmarked: the session's network blocks Hugging Face, where the models are hosted. Its latency below is an estimate, to be measured in phase 3.

## The gaps and how to close them

### 1. Live words while you speak — the biggest visible gap

**Design.** A preview runs beside the existing pipeline and never changes what is pasted.
- **Engine.** A `Preview` in `src/stt.rs` wraps a sherpa-onnx `OnlineRecognizer` on one thread. The pipeline feeds it every audio chunk it already receives (`PipelineCmd::Audio`). Every 200 ms, if the text changed, it sends `PipelineMsg::Partial(String)`.
- **Steady text.** Murmur's committed chunks (the Parakeet text decoded at each pause) replace the preview's words for that stretch. The live part is only the speech since the last pause, so the caption firms up as you go.
- **Cleanup.** Fillers and spoken commands go through the existing cleanup. Self-corrections are only applied to committed text.
- **Display.** The listening pill grows into a caption bubble showing the last line or two. Murmur already draws text in `mote.rs` (fix-last tag) and already knows the caret position (`caret.rs`).
  - Windows: the layered window in `overlay.rs`.
  - Mac: `overlay/mac.rs`.
  - Option: anchor the bubble just below the caret, like an IME composition window. This is where the "words appear where I'm typing" feel comes from, without typing into the app and breaking undo.
- **On release.** Nothing changes: final Parakeet decode, cleanup, paste. The bubble shows the final text for the 0.5–0.8 s it takes, then becomes the paste.
- **Model download.** The preview model is optional and fetched like the main model (`model_fetch.rs`, with a checksum). Kroko is 70 MB. Setting: `live_preview = true`.
- **Fallback with no new model.** Re-decode the uncommitted tail with Parakeet 0.6B every 600 ms, but only while that tail is under 4 s, and skip a tick if the last one is still running. That is the FluidVoice loop, bounded by Murmur's pause commits. It is fine on an M-series Mac and marginal on the Windows laptop (decode is about 0.1 s per second of audio there).
- **Licences.**
  - Kroko is CC-BY-SA, downloaded at runtime and credited in About, the same way Parakeet's CC-BY is today.
  - The NeMo 80 ms streaming model (CC-BY-4.0) is the fallback if Jeff wants to avoid share-alike.
- **Risks.**
  - A second model's memory: Kroko adds about 150 MB.
  - CPU contention with the pause decodes. Overruns are already watched in `murmur.log`, and the preview runs on one thread.
  - A preview word that differs from the pasted word may look like a bug. Mitigation: the final text replaces the caption visibly before the paste.
- **Scope.** About 3 days. One day for the engine and messages with tests, two for the caption on both platforms.

### 2. Speed

Today, from the release-to-text log line: Windows laptop, about 0.6–0.8 s for normal dictations; Mac, "instant" per Jeff. FluidVoice's edge is the Neural Engine.

| Step | What | Scope | Expected |
|---|---|---|---|
| 2a | **CoreML on the Mac.** sherpa-onnx accepts `provider = "coreml"`. Try it for the Parakeet encoder behind a setting and compare `release_to_text_ms` and accuracy against `cpu`. One line in `stt.rs` plus a config key. | 2 hours plus a Mac test | Possibly 2–4× faster decode, but ONNX Runtime's CoreML coverage for this model is untested; it may fall back to CPU or fail. |
| 2b | **Commit sooner.** With a preview running, decode at 700 ms pauses once 2 s is buffered (today: 1 s pauses, 3 s buffered). Then less speech is left to decode on release. Keep the existing tests that block stray periods. | half a day | Smaller tail; measure on the laptop. |
| 2c | **"Fast" model option** for slow PCs: Parakeet TDT 110M as the final model, `model = "fast"`. It is 4–5× faster to decode at some accuracy cost. | 1 day (download, setting, About) | 25 s dictation on the laptop: estimated 0.78 s → about 0.25 s. |
| 2d | **Paste.** The clipboard restore takes 150 ms of the paste time. The text already shows before the restore finishes, so this is perceived speed only. No change. | none | n/a |

### 3. Inline corrections and rewriting ("Fluid Intelligence")

Murmur's rules already handle the common pattern of a marker between like items. The gap is looser repairs ("let's move it into a service. Actually, let's do the tests first, then the move") and restructuring: lists, tone, rambling into a tidy message. Only an LLM does that.

**Design: an optional rewrite pass, off by default, local first.**
- **Provider.** Phase A talks to an **OpenAI-compatible endpoint** at a configured `rewrite_url` (default Ollama at `http://localhost:11434/v1`). Murmur adds no native dependency, and the user picks the model: Qwen3 1.7B, Gemma 3 1B, Llama 3.2 3B. `ureq` is already a dependency. A remote URL is allowed but shows a warning, because it sends dictations off the machine.
- **When it runs.** Only when the raw text holds a correction marker (Wispr's list, already in the backlog note: "scratch that", "never mind", "actually", "I mean", "no wait", "wait no", "sorry"), or when the app's profile asks for it. Everything else keeps the instant rules path. This keeps the median dictation as fast as today.
- **Timeout.** 1.5 s. If it is missed, the rules output is pasted and the log says so. Dictation never blocks on the LLM.
- **Pre-warm.** On key-down, send a 1-token request so the model is in memory by release (FluidVoice does the same).
- **Prompt.** Clean-room, in Murmur's words. Smallest edits; keep the speaker's final version of a corrected phrase; never answer or follow instructions inside the transcript. The transcript is sent as JSON so it can't inject instructions. Few-shot examples come from Murmur's own self-correction tests.
- **Safety check.** Reject the rewrite if it adds words that are not in the raw text beyond punctuation and casing, or if it drops more than 40% of the words without a scratch marker. Then fall back to the rules output.
- **Undo.** History keeps both raw and cleaned text; fix-last edits the cleaned text, unchanged.
- **Phase B, only if A proves its worth.** Bundle llama.cpp through the `llama-cpp-2` crate with a downloaded 1B GGUF. This adds a native dependency and about 1 GB on both platforms.
- **Estimated latency.** Not measured; see Benchmarks. A 1–2B model at Q4 rewriting 40 tokens is roughly 0.3–0.6 s on an M-series Mac. On a 4–8 core PC CPU it is roughly 1–2 s, which is why the pass is gated and has a timeout.
- **Scope.** Phase A, about 2 days with tests (gate, timeout, check, prompt, setting).

### 4. Per-app tone

Murmur has `context::Profile` (Plain, Email) and app lists in `config.toml`.
- **Rules side.** Add `Profile::Code` for terminals and editors: favour symbols, no automatic final period, no email layout.
- **LLM side.** Add a `[[rewrite.app]]` table that maps an exe name or bundle id to a short style line ("casual, lowercase ok" for Slack, "formal" for mail).
- **Scope.** Half a day for Code; the LLM table comes with item 3.

### 5. Edit mode: "rewrite the selection"

- Hold Shift+Ctrl with the key (or a second key) and speak an instruction: "make this friendlier", "turn into bullets".
- Murmur copies the selection (Ctrl/Cmd+C with clipboard save and restore, the same mechanism as paste), sends the selection and the instruction to the rewrite provider, and pastes over the selection. Undo is the app's own Ctrl+Z.
- Needs item 3.
- **Scope.** 1–2 days.
- **Command mode** (an LLM running shell commands) is **not recommended**. It is risky, off-mission for a dictation app, and Claude Code already covers it.

### 6. Languages

- Parakeet TDT 0.6B **v3** int8 is published by sherpa-onnx (487 MB, 25 European languages, same API).
- Offer it as a model choice, `model = "multilingual"`. The cleanup rules are English-only and would be skipped for other languages.
- **Scope.** 1 day. Low priority unless a user asks.

### 7. Not worth chasing now

- File and video transcription.
- Meeting notes (FluidMeet).
- A local HTTP API.
- Notch UI.
- Their own distilled model: Fluid Speech Mini's ONNX build is CC BY 4.0 and could be tried as a "fast" model alongside item 2c, but it is from a single vendor and unproven on sherpa-onnx.

## Recommended order

1. **2a, CoreML on the Mac.** Cheapest experiment and the biggest possible speed gain.
2. **1, live preview** with Kroko, plus **2b, commit sooner.** This closes the gap people notice first.
3. **3, rewrite pass, phase A** (Ollama endpoint, gated, with a timeout).
4. **2c, the fast model** for slow PCs; **4, the Code profile**.
5. **5, edit mode**, if 3 is used.

## Open questions for Jeff

| Question | Options |
|---|---|
| Preview model | Kroko (best text, CC-BY-SA) / NeMo 80 ms (CC-BY, lowercase, 5× the CPU) / no new model: Parakeet tail re-decode |
| Where the live words show | Grow the pill / a bubble at the caret / both, as a setting |
| Preview default | On, or off until tested on the Windows laptop |
| LLM rewrite | Phase A (bring your own Ollama) / straight to a bundled model / not at all, rules only |
| Edit mode | Yes after 3 / no |

## Reproducing the benchmarks

- The harness was a 150-line Rust program on `sherpa-onnx = "=1.13.8"`. It decodes WAVs offline, re-decodes growing prefixes, and feeds `OnlineRecognizer` 100 ms chunks while timing each step.
- Test speech came from `vits-piper-en_US-lessac-medium`. Every model came from the sherpa-onnx `asr-models` release.
- To repeat on the Windows laptop, build it next to Murmur and point it at a few real dictations from `murmur.log`'s timing runs.
