# Murmur answers where you acted: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Nothing heard, fix-last results and a failed clipboard write are answered at the caret by the mote, which stretches into a capsule holding the words, instead of by corner balloons.

**Architecture:** `canvas.rs` learns to draw text (GDI into a DIB, grayscale coverage composited per span colour). `mote.rs`'s `Flight` gains speaking phases (unfurl, hold, furl) and `Mote` draws a capsule sized to the message. `correction.rs` returns a `FixOutcome` instead of notifying, `main.rs` turns outcomes and empty dictations into `mote.say`, `inject.rs` types the text when the clipboard write fails, and History gets an empty state.

**Tech Stack:** Rust 2021, `windows` 0.62 (Win32 GDI, layered windows), eframe/egui 0.36 for the fix-last card and History.

**Spec:** `docs/superpowers/specs/2026-10-02-murmur-answers-where-you-acted-design.md`

## Global Constraints

- Copy, verbatim: "Didn't catch that", "Nothing to fix yet", "Learned " + `spoken → written` terms joined with ", ", "Copied, press Ctrl+V to paste", and with learned terms " · Copied, press Ctrl+V". History: "Nothing dictated yet. Hold {key} and speak."
- Plain voice, never first person.
- Hold = `max(duration.locate, duration.read_per_char × characters)`; `read_per_char` = 60 ms, a new kit token. Characters are `chars()`, not bytes.
- Durations and easings come from `crate::motion` through `motion::scaled(...)`; no raw durations (the mote's existing `FLIGHT` is the one ledgered exception).
- Material: body `0x202020` at 0.9 alpha; text `#EEEEEE`; spoken `#D98C7A`; written `#F5C54A`; separators `#999999`. These come from `correction_ui`'s palette, not new literals.
- Font: "Segoe UI Variable Text", 12 pt = `round(16 × dpi/96)` px, grayscale antialiased.
- Reduced motion (`editor_kit::reduced_motion()`): no flight, no stretch; the tag appears open, holds the same time, and disappears.
- Never activates, never takes focus; the tag window is click-through.
- Balloons stay for: updates, model download, mic lost, startup errors, dictation errors, "dictionary not loaded".
- Every commit ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Never push.

## Review Focus

1. **Cancelled holds look like empty dictations.** `pipeline::abort` (a short tap, Esc, Shift turning a hold into fix-last) also sends an empty `Done`. Only a `Release` that sent `Stop` may say "Didn't catch that". Pinned in Task 6 (`unheard` takes `expect_words`).
2. **The caret lookup is slower than an empty decode** (UI Automation apps such as Chrome take tens of ms; an empty decode is near-instant). The message must still land at the caret, not fall back to the pill. Pinned in Task 6 (`Unheard::WaitForCaret`).
3. **A long message near the right edge of the screen** must stay on screen. Pinned in Task 4 (`fit` clamps into the work area).
4. **Non-ASCII terms** (é, CJK) in "Learned": the hold counts characters, and GDI draws them. Pinned in Task 2 (`measure` on "日本") and Task 3 (`hold_for` on "café").
5. **Many terms learned at once** make an unreadably long tag. Show the first two and " +N more". Pinned in Task 5. (The spec is silent; this is the plan's ruling, flagged to Jeff.)

**Known, not handled (flag to Jeff, don't fix):** someone whose talk key is Right Ctrl and who holds it as a modifier for 150 ms or more (Ctrl+C held) now gets "Didn't catch that". Before, nothing showed.

---

## Prerequisites

- [ ] **P1: Branch.** The close-flash and processing-dots fixes (#4 and #5) must be committed on `fix/close-flash-processing-dots` first, because this plan builds on their `overlay.rs` and `history_ui.rs` (its test helpers `app()` and `frame()`). Then:

```bash
git switch -c feat/answers-where-you-acted fix/close-flash-processing-dots
git cherry-pick c4dfa06
```
Expected: the spec commit applied on the new branch.

- [ ] **P2: Test commands.** Tests run through `%TEMP%\murmur-test.cmd`, which calls vcvars, `cd`s to this worktree and sets `CARGO_TARGET_DIR=C:\Users\JeffLocal\git\murmur\target`. From Git Bash:

```bash
cmd //c "%TEMP%\\murmur-test.cmd" --bin murmur <filter>
```
The whole suite is `cmd //c "%TEMP%\\murmur-test.cmd"`. The baseline is lib 170, bin 142, stt_integration 1. If the build fails with `os error 32`, a Murmur dev build is running from `target\release`. If it fails with `link: extra operand`, see the build traps memory.

---

### Task 1: The `read_per_char` token

**Files:**
- Modify: `~/.claude/skills/considered-ux/kits/jeff.tokens.json`
- Regenerate: `src/motion.rs`

**Interfaces:**
- Produces: `motion::duration::READ_PER_CHAR: std::time::Duration` (60 ms).

- [ ] **Step 1: Add the token to the kit**

In `jeff.tokens.json`, the `duration` object becomes:
```json
"duration": { "hover": 150, "enter": 150, "exit": 100, "emphasis": 300, "bump": 350, "fill": 400, "confirm": 1000, "locate": 1200, "readPerChar": 60 },
```

- [ ] **Step 2: Emit and check**

```bash
node ~/.claude/skills/considered-ux/scripts/emit-tokens.mjs --tokens ~/.claude/skills/considered-ux/kits/jeff.tokens.json --target rust --out src/motion.rs
node ~/.claude/skills/considered-ux/scripts/kit.mjs --check ~/.claude/skills/considered-ux/kits/jeff.md
git diff --ignore-cr-at-eol --stat src/motion.rs
```
Expected: `wrote …motion.rs`; the check passes; `motion.rs` shows exactly 1 insertion, `pub const READ_PER_CHAR: std::time::Duration = std::time::Duration::from_millis(60);`.

- [ ] **Step 3: Build**

Run: `cmd //c "%TEMP%\\murmur-test.cmd" --bin murmur motion`
Expected: compiles; an unused-constant warning for `READ_PER_CHAR` is acceptable until Task 3.

- [ ] **Step 4: Commit**

```bash
git add src/motion.rs
git commit -m "feat(motion): read-time token for messages" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Text on the canvas

**Files:**
- Modify: `src/canvas.rs`

**Interfaces:**
- Produces:
  - `pub(crate) type Span = (String, u32);` (text, 0xRRGGBB)
  - `pub(crate) fn measure(spans: &[Span], px: i32) -> (i32, i32)`: width and height in pixels; `(0, 0)` for no text.
  - `Canvas::text(&mut self, x: f32, y: f32, spans: &[Span], px: i32, alpha: f32)`: draws the spans left to right with the top-left at (x, y).

- [ ] **Step 1: Write the failing tests** (append to `mod tests` in `src/canvas.rs`)

```rust
    #[test]
    fn text_is_drawn_in_each_spans_colour() {
        let spans: Vec<Span> = vec![("Hob".into(), 0xFF0000), ("HAWB".into(), 0x0000FF)];
        let (w, h) = measure(&spans, 16);
        assert!(w > 20 && h >= 16, "{w}x{h}");
        let mut c = Canvas::new(w, h);
        c.text(0.0, 0.0, &spans, 16, 1.0);
        let px = c.into_bgra();
        let cols = |r: std::ops::Range<i32>| px.iter().enumerate().filter(|(i, _)| r.contains(&(*i as i32 % w))).map(|(_, p)| *p).collect::<Vec<_>>();
        assert!(cols(0..w / 3).iter().any(|p| (p >> 16) & 0xFF > 0x80 && p & 0xFF == 0), "red on the left");
        assert!(cols(2 * w / 3..w).iter().any(|p| p & 0xFF > 0x80 && (p >> 16) & 0xFF == 0), "blue on the right");
        for p in px {
            let a = p >> 24;
            assert!((p >> 16) & 0xFF <= a && (p >> 8) & 0xFF <= a && p & 0xFF <= a, "not premultiplied: {p:08X}");
        }
    }

    #[test]
    fn measure_handles_wide_characters_and_nothing() {
        assert!(measure(&[("日本".into(), 0xFFFFFF)], 16).0 > 10);
        assert_eq!(measure(&[], 16), (0, 0));
    }

    #[test]
    fn text_respects_alpha() {
        let spans: Vec<Span> = vec![("Hi".into(), 0xFFFFFF)];
        let (w, h) = measure(&spans, 16);
        let draw = |a: f32| {
            let mut c = Canvas::new(w, h);
            c.text(0.0, 0.0, &spans, 16, a);
            c.into_bgra().iter().map(|p| p >> 24).max().unwrap()
        };
        assert!(draw(0.5) < draw(1.0));
        assert_eq!(draw(0.0), 0);
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cmd //c "%TEMP%\\murmur-test.cmd" --bin murmur canvas`
Expected: compile errors, `cannot find type Span` and `cannot find function measure`.

- [ ] **Step 3: Implement**

Add to the `use` lines at the top of `src/canvas.rs`:
```rust
use windows::core::w;
use windows::Win32::Graphics::Gdi::{
    CreateFontW, GdiFlush, GetTextExtentPoint32W, SetBkMode, SetTextColor, TextOutW, ANTIALIASED_QUALITY, CLIP_DEFAULT_PRECIS,
    DEFAULT_CHARSET, DEFAULT_PITCH, FW_NORMAL, HDC, OUT_DEFAULT_PRECIS, TRANSPARENT,
};
```

Add after `impl Canvas { … }`'s `halo` method, inside `impl Canvas`:
```rust
    /// Draw `spans` left to right with the top-left at (x, y), each in its own colour at `alpha`.
    /// GDI draws white on black with grayscale antialiasing, so any channel is the coverage.
    pub(crate) fn text(&mut self, x: f32, y: f32, spans: &[Span], px: i32, alpha: f32) {
        let (tw, th) = measure(spans, px);
        if tw <= 0 || th <= 0 || alpha <= 0.0 {
            return;
        }
        let (cov, runs) = with_font(px, |dc| unsafe {
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: tw,
                    biHeight: -th,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let Ok(bmp) = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else { return (Vec::new(), Vec::new()) };
            if bits.is_null() {
                let _ = DeleteObject(bmp.into());
                return (Vec::new(), Vec::new());
            }
            let pixels = std::slice::from_raw_parts_mut(bits as *mut u32, (tw * th) as usize);
            pixels.fill(0);
            let old = SelectObject(dc, bmp.into());
            SetBkMode(dc, TRANSPARENT);
            SetTextColor(dc, COLORREF(0x00FF_FFFF));
            let mut at = 0;
            let mut runs = Vec::new();
            for (s, rgb) in spans {
                let wide: Vec<u16> = s.encode_utf16().collect();
                let _ = TextOutW(dc, at, 0, &wide);
                let next = at + extent(dc, &wide).cx;
                runs.push((at, next, *rgb));
                at = next;
            }
            let _ = GdiFlush();
            let cov = pixels.iter().map(|p| ((p >> 8) & 0xFF) as f32 / 255.0).collect::<Vec<_>>();
            let _ = SelectObject(dc, old);
            let _ = DeleteObject(bmp.into());
            (cov, runs)
        });
        let (ox, oy) = (x.round() as i32, y.round() as i32);
        for row in 0..th {
            for col in 0..tw {
                let a = cov.get((row * tw + col) as usize).copied().unwrap_or(0.0) * alpha;
                let (dx, dy) = (ox + col, oy + row);
                if a <= 0.0 || dx < 0 || dy < 0 || dx >= self.w || dy >= self.h {
                    continue;
                }
                let rgb = runs.iter().find(|(s, e, _)| col >= *s && col < *e).or(runs.last()).map_or(0xFFFFFF, |r| r.2);
                let c = [(rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF].map(|v| v as f32 / 255.0);
                let dst = &mut self.px[(dy * self.w + dx) as usize];
                for i in 0..3 {
                    dst[i] = c[i] * a + dst[i] * (1.0 - a);
                }
                dst[3] = a + dst[3] * (1.0 - a);
            }
        }
    }
```

Add after `impl Canvas`, before `push`:
```rust
/// A run of text in one colour (0xRRGGBB).
pub(crate) type Span = (String, u32);

/// Runs `f` with a memory DC holding the UI font, `px` pixels to the em.
fn with_font<T>(px: i32, f: impl FnOnce(HDC) -> T) -> T {
    unsafe {
        let dc = CreateCompatibleDC(None);
        let font = CreateFontW(
            -px, 0, 0, 0, FW_NORMAL.0 as i32, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS,
            ANTIALIASED_QUALITY, DEFAULT_PITCH.0 as u32, w!("Segoe UI Variable Text"),
        );
        let old = SelectObject(dc, font.into());
        let r = f(dc);
        let _ = SelectObject(dc, old);
        let _ = DeleteObject(font.into());
        let _ = DeleteDC(dc);
        r
    }
}

fn extent(dc: HDC, wide: &[u16]) -> SIZE {
    let mut sz = SIZE::default();
    let _ = unsafe { GetTextExtentPoint32W(dc, wide, &mut sz) };
    sz
}

/// Width and height of `spans` drawn side by side at `px`; (0, 0) when there's no text.
pub(crate) fn measure(spans: &[Span], px: i32) -> (i32, i32) {
    if spans.iter().all(|(s, _)| s.is_empty()) {
        return (0, 0);
    }
    with_font(px, |dc| {
        spans.iter().fold((0, 0), |(w, h), (s, _)| {
            let sz = extent(dc, &s.encode_utf16().collect::<Vec<u16>>());
            (w + sz.cx, h.max(sz.cy))
        })
    })
}
```

- [ ] **Step 4: Run to verify they pass**

Run: `cmd //c "%TEMP%\\murmur-test.cmd" --bin murmur canvas`
Expected: 4 passed (3 new plus `glow_only_lights_what_is_drawn`). If `text_is_drawn_in_each_spans_colour` finds no red, check that `pixels.fill(0)` ran before `TextOutW` and that coverage reads the green byte.

- [ ] **Step 5: Commit**

```bash
git add src/canvas.rs
git commit -m "feat(canvas): draw coloured text runs" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: The mote's speaking phases

**Files:**
- Modify: `src/mote.rs` (the `Flight` state machine and its tests; nothing about the window yet)
- Modify: `src/correction_ui.rs:27` (make `SPOKEN` `pub(crate)`, add `rgb`)

**Interfaces:**
- Consumes: `canvas::Span` (Task 2), `motion::duration::READ_PER_CHAR` (Task 1).
- Produces:
  - `pub(crate) struct Message(pub Vec<Span>)` with `Message::plain(&str) -> Message` and `fn chars(&self) -> usize`.
  - `pub(crate) fn hold_for(m: &Message) -> Duration`.
  - `pub(crate) enum Target { Caret(Pt), Pill(Pt) }`.
  - `Flight::say(&mut self, m: Message, from: Pt, to: Target, now: Instant)`, `Flight::dismiss(&mut self, now: Instant)`, `Flight::is_speaking(&self) -> bool`, `Flight::message(&self) -> Option<&Message>`, `Flight::centred(&self) -> bool`.
  - `Sprite` gains `pub open: f32` (0 = dot, 1 = full capsule, eased) and `pub words: f32` (text opacity).
  - `pub(crate) fn caret_point(r: RECT) -> Pt`: the landing point for a caret rect, `(left, mid y)`.
  - In `correction_ui`: `pub(crate) const SPOKEN: Color32` and `pub(crate) fn rgb(c: Color32) -> u32`.

- [ ] **Step 1: Write the failing tests** (append to `mod tests` in `src/mote.rs`)

```rust
    fn msg(s: &str) -> Message {
        Message::plain(s)
    }

    #[test]
    fn hold_reads_at_least_locate_and_grows_with_length() {
        assert_eq!(hold_for(&msg("Didn't catch that")), motion::duration::LOCATE, "17 chars is under locate");
        let long = msg("Learned hob → HAWB · Copied, press Ctrl+V");
        assert_eq!(hold_for(&long), motion::duration::READ_PER_CHAR * long.chars() as u32);
        assert_eq!(msg("café").chars(), 4, "characters, not bytes");
    }

    #[test]
    fn say_flies_unfurls_holds_then_furls_away() {
        let t0 = Instant::now();
        let m = msg("Didn't catch that");
        let hold = hold_for(&m);
        let mut f = Flight::new(false);
        f.say(m, PILL, Target::Caret(CARET), t0);
        let s = f.sprite(t0).unwrap();
        assert!(close(s.at, PILL) && s.open == 0.0, "starts as a dot at the pill");
        let open_at = t0 + FLIGHT + motion::duration::ENTER + ms(1);
        let s = f.sprite(open_at).unwrap();
        assert_eq!(s.at, CARET);
        assert!(s.open > 0.99 && s.words > 0.99, "{s:?}");
        let furling = t0 + FLIGHT + motion::duration::ENTER + hold + motion::duration::EXIT / 2;
        let s = f.sprite(furling).unwrap();
        assert!(s.open < 1.0 && s.open > 0.0, "{s:?}");
        assert!(f.is_speaking());
        let gone = t0 + FLIGHT + motion::duration::ENTER + hold + motion::duration::EXIT * 2 + ms(2);
        assert!(f.sprite(gone).is_none());
        assert!(!f.is_speaking());
    }

    #[test]
    fn say_on_a_settled_mote_unfurls_where_it_is() {
        let t0 = Instant::now();
        let mut f = Flight::new(false);
        f.launch(PILL, CARET, t0);
        let settled = t0 + FLIGHT + ms(10);
        f.sprite(settled);
        f.say(msg("Didn't catch that"), PILL, Target::Pill((0.0, 0.0)), settled);
        let s = f.sprite(settled + motion::duration::ENTER + ms(1)).unwrap();
        assert_eq!(s.at, CARET, "stays at the caret, ignoring the new target");
        assert!(s.open > 0.99);
        assert!(!f.centred(), "a caret message grows rightward from the caret");
    }

    #[test]
    fn say_mid_flight_finishes_the_flight_first() {
        let t0 = Instant::now();
        let mut f = Flight::new(false);
        f.launch(PILL, CARET, t0);
        f.say(msg("Didn't catch that"), PILL, Target::Pill((5.0, 5.0)), t0 + ms(100));
        assert_eq!(f.sprite(t0 + FLIGHT + ms(1)).unwrap().at, CARET);
    }

    #[test]
    fn dismiss_furls_from_where_it_is() {
        let t0 = Instant::now();
        let mut f = Flight::new(false);
        f.launch(PILL, CARET, t0);
        let settled = t0 + FLIGHT + ms(10);
        f.sprite(settled);
        f.say(msg("Nothing to fix yet"), PILL, Target::Caret(CARET), settled);
        let mid = settled + motion::duration::ENTER / 2;
        let half = f.sprite(mid).unwrap().open;
        f.dismiss(mid);
        let after = f.sprite(mid + ms(10)).unwrap();
        assert!(after.open < half && after.open > 0.0, "{half} -> {}", after.open);
        assert!(f.sprite(mid + motion::duration::EXIT * 2 + ms(2)).is_none());
    }

    #[test]
    fn reduced_motion_appears_open_and_still_holds() {
        let t0 = Instant::now();
        let m = msg("Didn't catch that");
        let hold = hold_for(&m);
        let mut f = Flight::new(true);
        f.say(m, PILL, Target::Pill(CARET), t0);
        let s = f.sprite(t0).unwrap();
        assert_eq!(s.at, CARET, "no flight");
        assert!(s.open == 1.0 && s.words == 1.0, "no stretch");
        assert!(f.centred(), "a pill message is centred");
        assert!(f.sprite(t0 + hold - ms(1)).is_some());
        assert!(f.sprite(t0 + hold + ms(1)).is_none());
    }

    #[test]
    fn a_new_launch_replaces_a_message() {
        let t0 = Instant::now();
        let mut f = Flight::new(false);
        f.say(msg("Didn't catch that"), PILL, Target::Caret(CARET), t0);
        f.launch(PILL, CARET, t0 + ms(50));
        assert!(!f.is_speaking());
        assert!(f.message().is_none());
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cmd //c "%TEMP%\\murmur-test.cmd" --bin murmur mote`
Expected: compile errors, `cannot find type Message` / `cannot find function hold_for`.

- [ ] **Step 3: Implement**

In `src/correction_ui.rs`, change line 27 and add `rgb` below the palette:
```rust
pub(crate) const SPOKEN: Color32 = Color32::from_rgb(0xD9, 0x8C, 0x7A);
```
```rust
/// A palette colour as 0xRRGGBB, for the GDI-drawn windows.
pub(crate) fn rgb(c: Color32) -> u32 {
    (c.r() as u32) << 16 | (c.g() as u32) << 8 | c.b() as u32
}
```

In `src/mote.rs`, add to the imports:
```rust
use crate::canvas::Span;
```

Add after `arc_point`:
```rust
/// What the mote says: runs of text, each in its own colour.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Message(pub Vec<Span>);

impl Message {
    pub(crate) fn plain(s: &str) -> Message {
        Message(vec![(s.to_string(), crate::correction_ui::rgb(crate::correction_ui::TEXT))])
    }

    pub(crate) fn chars(&self) -> usize {
        self.0.iter().map(|(s, _)| s.chars().count()).sum()
    }
}

/// How long a message stays open: long enough to read it.
pub(crate) fn hold_for(m: &Message) -> Duration {
    motion::duration::LOCATE.max(motion::duration::READ_PER_CHAR * m.chars() as u32)
}

/// Where a message is said: at a caret it grows rightward from the caret; above the pill it's centred.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Target {
    Caret(Pt),
    Pill(Pt),
}

/// The landing point for a caret rect: its left edge, halfway down the line.
pub(crate) fn caret_point(r: RECT) -> Pt {
    (r.left as f32, (r.top + r.bottom) as f32 / 2.0)
}

/// Text opacity for capsule openness `open`: the words come in once it's about 70% wide.
fn words_for(open: f32) -> f32 {
    ((open - 0.7) / 0.3).clamp(0.0, 1.0)
}
```

Replace `Sprite`, `Phase` and `Flight`'s fields with:
```rust
/// What to draw this frame: where, how opaque, how large (1 = normal), and how far a message
/// capsule has opened (0 = a dot) and how visible its words are.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Sprite {
    pub at: Pt,
    pub alpha: f32,
    pub radius: f32,
    pub open: f32,
    pub words: f32,
}

/// What a flight does on arrival.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Then {
    Settle,
    Dissolve,
    Say,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    Hidden,
    Flying { from: Pt, to: Pt, start: Instant, then: Then },
    Settled { at: Pt, since: Instant },
    /// unfurling into the message, then holding it open
    Speaking { at: Pt, start: Instant },
    /// pulling back to a dot from openness `open`
    Furling { at: Pt, start: Instant, open: f32 },
    Leaving { at: Pt, start: Instant, alpha: f32, grow: bool },
}

pub(crate) struct Flight {
    phase: Phase,
    reduced: bool,
    message: Option<Message>,
    centred: bool,
}
```

Replace `impl Flight`'s `new`, `launch` and `dissolve` with the versions below, and add `say`, `dismiss`, `is_speaking`, `message` and `centred`. `fade`, `leave` and `is_active` are unchanged:
```rust
    pub(crate) fn new(reduced: bool) -> Flight {
        Flight { phase: Phase::Hidden, reduced, message: None, centred: false }
    }

    /// Starts a flight from the pill to the caret, replacing whatever was showing.
    pub(crate) fn launch(&mut self, from: Pt, to: Pt, now: Instant) {
        self.message = None;
        self.phase = if self.reduced { Phase::Settled { at: to, since: now } } else { Phase::Flying { from, to, start: now, then: Then::Settle } };
    }

    /// The words landed: grow and fade into them. Mid-flight, it finishes the flight first.
    pub(crate) fn dissolve(&mut self, now: Instant) {
        if let Phase::Flying { then, .. } = &mut self.phase {
            *then = Then::Dissolve;
            return;
        }
        self.leave(now, true);
    }

    /// Says `m`: a mote already out says it where it is (finishing a flight first); otherwise
    /// one flies from `from` to the target and says it there.
    pub(crate) fn say(&mut self, m: Message, from: Pt, to: Target, now: Instant) {
        self.message = Some(m);
        self.phase = match self.phase {
            Phase::Flying { from, to, start, then } => {
                // a dictation's flight ends at a caret; an earlier message keeps its own placement
                if then != Then::Say {
                    self.centred = false;
                }
                Phase::Flying { from, to, start, then: Then::Say }
            }
            Phase::Settled { at, .. } | Phase::Speaking { at, .. } => {
                self.centred = false;
                Phase::Speaking { at, start: now }
            }
            _ => {
                let (at, centred) = match to {
                    Target::Caret(p) => (p, false),
                    Target::Pill(p) => (p, true),
                };
                self.centred = centred;
                if self.reduced { Phase::Speaking { at, start: now } } else { Phase::Flying { from, to: at, start: now, then: Then::Say } }
            }
        };
    }

    /// Closes an open message early, from however far it has opened.
    pub(crate) fn dismiss(&mut self, now: Instant) {
        match self.phase {
            Phase::Speaking { at, .. } => {
                let open = self.sprite(now).map_or(0.0, |s| s.open);
                self.phase = if self.reduced { Phase::Hidden } else { Phase::Furling { at, start: now, open } };
            }
            Phase::Flying { then: Then::Say, .. } => self.leave(now, false),
            _ => {}
        }
    }

    /// Whether a message is on its way, open, or closing.
    pub(crate) fn is_speaking(&self) -> bool {
        self.message.is_some() && matches!(self.phase, Phase::Flying { then: Then::Say, .. } | Phase::Speaking { .. } | Phase::Furling { .. })
    }

    pub(crate) fn message(&self) -> Option<&Message> {
        self.message.as_ref()
    }

    pub(crate) fn centred(&self) -> bool {
        self.centred
    }
```

In `sprite`, replace the `Flying` arm and add the `Speaking` and `Furling` arms. Every existing `Some(Sprite { .. })` gains `open: 0.0, words: 0.0`:
```rust
            Phase::Flying { from, to, start, then } => {
                let t = secs(start, FLIGHT);
                if t >= 1.0 {
                    let landed = start + motion::scaled(FLIGHT);
                    self.phase = match then {
                        Then::Settle => Phase::Settled { at: to, since: landed },
                        Then::Dissolve => Phase::Leaving { at: to, start: landed, alpha: 1.0, grow: true },
                        Then::Say => Phase::Speaking { at: to, start: landed },
                    };
                    return self.sprite(now);
                }
                Some(Sprite { at: arc_point(from, to, ease(motion::easing::ENTER, t)), alpha: 1.0, radius: 1.0, open: 0.0, words: 0.0 })
            }
            Phase::Speaking { at, start } => {
                let unfurl = if self.reduced { Duration::ZERO } else { motion::scaled(motion::duration::ENTER) };
                let hold = motion::scaled(self.message.as_ref().map_or(motion::duration::LOCATE, hold_for));
                let el = now.saturating_duration_since(start);
                if el >= unfurl + hold {
                    self.phase = if self.reduced { Phase::Hidden } else { Phase::Furling { at, start: start + unfurl + hold, open: 1.0 } };
                    return self.sprite(now);
                }
                let open = if unfurl.is_zero() { 1.0 } else { ease(motion::easing::ENTER, (el.as_secs_f32() / unfurl.as_secs_f32()).min(1.0)) };
                Some(Sprite { at, alpha: 1.0, radius: 1.0, open, words: words_for(open) })
            }
            Phase::Furling { at, start, open } => {
                let t = secs(start, motion::duration::EXIT);
                if t >= 1.0 {
                    self.phase = Phase::Leaving { at, start: start + motion::scaled(motion::duration::EXIT), alpha: 1.0, grow: true };
                    return self.sprite(now);
                }
                // the words go in the first half; the capsule pulls back over the whole furl
                let words = (1.0 - 2.0 * t).max(0.0).min(words_for(open));
                Some(Sprite { at, alpha: 1.0, radius: 1.0, open: open * (1.0 - ease(motion::easing::EXIT, t)), words })
            }
```

Change `landing_point` to use the shared helper:
```rust
pub(crate) fn landing_point(awaiting: Option<isize>, target: isize, foreground: isize, caret: Option<RECT>) -> Option<Pt> {
    let r = caret?;
    (awaiting == Some(target) && foreground == target).then(|| caret_point(r))
}
```

- [ ] **Step 4: Run to verify they pass**

Run: `cmd //c "%TEMP%\\murmur-test.cmd" --bin murmur mote`
Expected: all mote tests pass, the existing ones (`flies_then_settles_at_the_caret`, `dissolve_grows_and_fades_away_fade_just_fades`, …) and the 7 new ones.

- [ ] **Step 5: Commit**

```bash
git add src/mote.rs src/correction_ui.rs
git commit -m "feat(mote): say a message: unfurl, hold, furl" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Drawing the speaking mote

**Files:**
- Modify: `src/mote.rs` (`Mote`, rendering, layout)
- Modify: `src/overlay.rs` (`above_physical`)

**Interfaces:**
- Consumes: `canvas::{measure, Canvas::text}` (Task 2); `Flight::{say, dismiss, is_speaking, message, centred}`, `Message`, `Target`, `Sprite.open/words` (Task 3); `caret::{work_area, Anchor, physical}`.
- Produces:
  - `Mote::say(&mut self, m: Message, from: Pt, to: Target)`, `Mote::dismiss(&mut self)`, `Mote::is_speaking(&self) -> bool`.
  - `Overlay::above_physical(&self) -> (f32, f32)`: the pill's top centre, physical pixels.

- [ ] **Step 1: Write the failing tests** (append to `mod tests` in `src/mote.rs`)

```rust
    #[test]
    fn a_closed_tag_is_the_dot_and_an_open_one_sits_above_the_caret() {
        let at = (400.0, 300.0);
        let dot = tag_box(at, 0.0, (100, 18), 1.0, false);
        assert_eq!((dot.cx, dot.cy), at);
        assert_eq!((dot.w, dot.h), (DOT, DOT));
        let open = tag_box(at, 1.0, (100, 18), 1.0, false);
        assert!((open.cx - open.w / 2.0 - (at.0 - DOT / 2.0)).abs() < 1e-3, "left edge stays at the caret");
        assert_eq!(open.w, 100.0 + 2.0 * open.pad);
        assert!(open.cy + open.h / 2.0 < at.1, "entirely above the caret's middle: {open:?}");
        let centred = tag_box(at, 1.0, (100, 18), 1.0, true);
        assert_eq!(centred.cx, at.0);
        let big = tag_box(at, 1.0, (100, 18), 2.0, false);
        assert!(big.h > open.h && big.pad > open.pad, "scales with DPI");
    }

    #[test]
    fn fit_keeps_the_tag_on_screen() {
        let work = RECT { left: 0, top: 0, right: 1920, bottom: 1040 };
        assert_eq!(fit(1850, 500, 200, 40, work), (1720, 500), "pulled in from the right edge");
        assert_eq!(fit(-30, -10, 200, 40, work), (0, 0));
        let second = RECT { left: 1920, top: 0, right: 3840, bottom: 1040 };
        assert_eq!(fit(1900, 500, 200, 40, second).0, 1920);
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cmd //c "%TEMP%\\murmur-test.cmd" --bin murmur mote`
Expected: compile errors, `cannot find function tag_box` / `fit`.

- [ ] **Step 3: Implement**

In `src/mote.rs`, add to the imports:
```rust
use windows::Win32::UI::HiDpi::GetDpiForSystem;
```

Add after `render`:
```rust
/// The mote's core diameter, which a message capsule grows from.
const DOT: f32 = 5.0;
/// Room around the capsule for its rim and the fading halo.
const MARGIN: f32 = 8.0;

/// A message capsule's centre and size, physical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TagBox {
    pub cx: f32,
    pub cy: f32,
    pub w: f32,
    pub h: f32,
    pub pad: f32,
}

/// The capsule for text of size `text` at openness `open`: the dot at `at` when closed; open,
/// it sits above the line, its left edge at the caret (or centred over a pill target).
fn tag_box(at: Pt, open: f32, text: (i32, i32), scale: f32, centred: bool) -> TagBox {
    let pad = 12.0 * scale;
    let (full_w, full_h) = (text.0 as f32 + 2.0 * pad, 26.0 * scale);
    let lerp = |a: f32, b: f32| a + (b - a) * open;
    let (w, h) = (lerp(DOT, full_w), lerp(DOT, full_h));
    let lift = full_h / 2.0 + 16.0 * scale;
    let cx = if centred { at.0 } else { at.0 - DOT / 2.0 + w / 2.0 };
    TagBox { cx, cy: at.1 - lift * open, w, h, pad }
}

/// Top-left for a `w`×`h` window at (x, y), pulled inside `work`.
fn fit(x: i32, y: i32, w: i32, h: i32, work: RECT) -> (i32, i32) {
    (x.clamp(work.left, (work.right - w).max(work.left)), y.clamp(work.top, (work.bottom - h).max(work.top)))
}

/// A frame of a speaking mote, `w`×`h`, with the capsule centred at `centre`.
fn render_tag(s: &Sprite, m: &Message, b: &TagBox, centre: Pt, w: i32, h: i32, font: i32, text_h: i32) -> Vec<u32> {
    let mut c = Canvas::new(w, h);
    let (cx, cy) = centre;
    let dot = 1.0 - s.open;
    if dot > 0.0 {
        c.halo(cx, cy, 12.0 * s.radius, 0x60D060, 0.5 * s.alpha * dot);
    }
    c.capsule(cx - b.w / 2.0, cy - b.h / 2.0, b.w, b.h, 0x202020, 0.9 * s.alpha);
    if dot > 0.0 {
        let core = DOT * s.radius;
        c.capsule(cx - core / 2.0, cy - core / 2.0, core, core, 0xE8FFE8, s.alpha * dot);
    }
    if s.words > 0.0 {
        c.text(cx - b.w / 2.0 + b.pad, cy - text_h as f32 / 2.0, &m.0, font, s.words * s.alpha);
    }
    c.into_bgra()
}
```

Change `Mote` and its `create`:
```rust
pub(crate) struct Mote {
    hwnd: HWND,
    flight: Flight,
    shown: bool,
    /// physical pixels per 96-dpi pixel, for the message's size
    scale: f32,
}
```
and in `create`, the last line becomes:
```rust
        let scale = crate::caret::physical(|| unsafe { GetDpiForSystem() }) as f32 / 96.0;
        Ok(Mote { hwnd, flight: Flight::new(reduced_motion()), shown: false, scale })
```

Add to `impl Mote`:
```rust
    pub(crate) fn say(&mut self, m: Message, from: Pt, to: Target) {
        self.flight.say(m, from, to, Instant::now());
    }

    pub(crate) fn dismiss(&mut self) {
        self.flight.dismiss(Instant::now());
    }

    pub(crate) fn is_speaking(&self) -> bool {
        self.flight.is_speaking()
    }

    /// The frame for `s`: (x, y, w, h, pixels), physical pixels.
    fn frame(&self, s: &Sprite) -> (i32, i32, i32, i32, Vec<u32>) {
        let Some(m) = self.flight.message().filter(|_| s.open > 0.0) else {
            let (x, y) = ((s.at.0 - S as f32 / 2.0).round() as i32, (s.at.1 - S as f32 / 2.0).round() as i32);
            return (x, y, S, S, render(s));
        };
        let font = (16.0 * self.scale).round() as i32;
        let text = canvas::measure(&m.0, font);
        let b = tag_box(s.at, s.open, text, self.scale, self.flight.centred());
        let (w, h) = (((b.w + 2.0 * MARGIN).ceil() as i32).max(S), ((b.h + 2.0 * MARGIN).ceil() as i32).max(S));
        let (x0, y0) = ((b.cx - w as f32 / 2.0).round() as i32, (b.cy - h as f32 / 2.0).round() as i32);
        let at = RECT { left: s.at.0 as i32, top: s.at.1 as i32, right: s.at.0 as i32 + 1, bottom: s.at.1 as i32 + 1 };
        let (x, y) = fit(x0, y0, w, h, crate::caret::work_area(&crate::caret::Anchor::Area(at)));
        let px = render_tag(s, m, &b, (b.cx - x as f32, b.cy - y as f32), w, h, font, text.1);
        (x, y, w, h, px)
    }
```

Replace the `Some(s)` arm of `animate`:
```rust
            Some(s) => {
                let (x, y, w, h, px) = self.frame(&s);
                crate::caret::physical(|| canvas::push(self.hwnd, x, y, w, h, &px));
                if !self.shown {
                    unsafe {
                        let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOSIZE | SWP_NOMOVE | SWP_NOACTIVATE);
                        let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                    }
                    self.shown = true;
                }
            }
```
(`UpdateLayeredWindow` sets the window's size from the `SIZE` passed to `canvas::push`, so the window grows with the capsule.)

In `src/overlay.rs`, add after `centre_physical`:
```rust
    /// The pill's top centre in physical pixels: where a message goes when there's no caret.
    pub fn above_physical(&self) -> (f32, f32) {
        crate::caret::physical(|| unsafe {
            let mut r = RECT::default();
            let _ = GetWindowRect(self.hwnd, &mut r);
            ((r.left + r.right) as f32 / 2.0, r.top as f32)
        })
    }
```

- [ ] **Step 4: Run to verify they pass**

Run: `cmd //c "%TEMP%\\murmur-test.cmd" --bin murmur`
Expected: all bin tests pass; there may be dead-code warnings for `say`/`dismiss`/`is_speaking`/`above_physical` until Task 6.

- [ ] **Step 5: Commit**

```bash
git add src/mote.rs src/overlay.rs
git commit -m "feat(mote): draw a message as a capsule above the caret" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Fix-last reports an outcome instead of balloons

**Files:**
- Modify: `src/correction.rs`
- Modify: `src/correction_ui.rs` (`show` returns the card's centre)

**Interfaces:**
- Consumes: `mote::Message`, `correction_ui::{rgb, TEXT, MUTED, SPOKEN, AMBER}` (Task 3).
- Produces:
  - `pub struct FixOutcome { pub nothing_to_fix: bool, pub learned: Vec<(String, String)>, pub copied: bool, pub replaced: bool, pub card: Option<(f32, f32)> }` (derives `Debug, Default, PartialEq`).
  - `pub fn fix_last(history: &mut History, dict: &Arc<Mutex<Dictionary>>, tray: &Tray) -> FixOutcome`.
  - `pub(crate) fn fix_message(o: &FixOutcome) -> Option<Message>`.
  - `correction_ui::show(...) -> (Option<String>, Option<(f32, f32)>)`.

- [ ] **Step 1: Write the failing tests** (new `mod tests` at the end of `src/correction.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn text(m: &Message) -> String {
        m.0.iter().map(|(s, _)| s.as_str()).collect()
    }

    fn learned(n: usize) -> Vec<(String, String)> {
        (0..n).map(|i| (format!("hob{i}"), format!("HAWB{i}"))).collect()
    }

    #[test]
    fn nothing_to_fix_says_so() {
        let m = fix_message(&FixOutcome { nothing_to_fix: true, ..Default::default() }).unwrap();
        assert_eq!(text(&m), "Nothing to fix yet");
    }

    #[test]
    fn learned_terms_show_spoken_and_written_in_their_colours() {
        let m = fix_message(&FixOutcome { learned: vec![("hob".into(), "HAWB".into())], replaced: true, ..Default::default() }).unwrap();
        assert_eq!(text(&m), "Learned hob → HAWB");
        let colour = |s: &str| m.0.iter().find(|(t, _)| t == s).unwrap().1;
        assert_eq!(colour("hob"), rgb(SPOKEN));
        assert_eq!(colour("HAWB"), rgb(AMBER));
    }

    #[test]
    fn copied_alone_and_with_learned() {
        let m = fix_message(&FixOutcome { copied: true, ..Default::default() }).unwrap();
        assert_eq!(text(&m), "Copied, press Ctrl+V to paste");
        let m = fix_message(&FixOutcome { copied: true, learned: learned(1), ..Default::default() }).unwrap();
        assert_eq!(text(&m), "Learned hob0 → HAWB0 · Copied, press Ctrl+V");
    }

    #[test]
    fn many_terms_show_two_and_a_count() {
        let m = fix_message(&FixOutcome { learned: learned(4), replaced: true, ..Default::default() }).unwrap();
        assert_eq!(text(&m), "Learned hob0 → HAWB0, hob1 → HAWB1 +2 more");
    }

    #[test]
    fn a_plain_replace_or_a_cancel_says_nothing() {
        assert_eq!(fix_message(&FixOutcome { replaced: true, ..Default::default() }), None);
        assert_eq!(fix_message(&FixOutcome::default()), None);
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cmd //c "%TEMP%\\murmur-test.cmd" --bin murmur correction::`
Expected: compile errors, `cannot find struct FixOutcome` / `cannot find function fix_message`.

- [ ] **Step 3: Implement `correction_ui::show`'s card centre**

In `src/correction_ui.rs`:
- add `card: Rc<RefCell<Option<(f32, f32)>>>,` to `FixApp`;
- in `FixApp::close`, before sending `Close`:
```rust
        // where the card was, for the mote that carries the result to the caret
        *self.card.borrow_mut() = Some(caret::physical(|| unsafe {
            let mut r = RECT::default();
            let _ = GetWindowRect(self.hwnd, &mut r);
            ((r.left + r.right) as f32 / 2.0, (r.top + r.bottom) as f32 / 2.0)
        }));
```
- change `show`'s signature and doc:
```rust
/// Shows the fix-last dialog next to where `initial` was dictated. Blocks until the user
/// replaces (Some(edited)) or cancels (None); also returns the card's centre, physical pixels.
pub fn show(initial: &str, heard_at: Option<Instant>, dict: Dictionary, target: HWND) -> (Option<String>, Option<(f32, f32)>) {
```
- create `let card = Rc::new(RefCell::new(None)); let app_card = card.clone();` next to `out`, pass `card: app_card` into `FixApp`, and end with:
```rust
    let result = out.borrow_mut().take();
    let at = card.borrow_mut().take();
    (result, at)
```

- [ ] **Step 4: Implement `FixOutcome`, `fix_message` and the new `fix_last`**

In `src/correction.rs`, add to the imports:
```rust
use crate::correction_ui::{rgb, AMBER, MUTED, SPOKEN, TEXT};
use crate::mote::Message;
```

Add after `bring_to_front`:
```rust
/// What fix-last did, for the mote to say at the caret.
#[derive(Debug, Default, PartialEq)]
pub struct FixOutcome {
    pub nothing_to_fix: bool,
    /// (spoken, written) for each term learned
    pub learned: Vec<(String, String)>,
    /// the correction went to the clipboard for you to paste
    pub copied: bool,
    /// the correction replaced the dictation in place
    pub replaced: bool,
    /// the card's centre when it closed, physical pixels
    pub card: Option<(f32, f32)>,
}

/// Terms named in a "Learned" message; any more are counted.
const NAMED: usize = 2;

/// What the mote says after fix-last, or None when the landing says it all.
pub(crate) fn fix_message(o: &FixOutcome) -> Option<Message> {
    if o.nothing_to_fix {
        return Some(Message::plain("Nothing to fix yet"));
    }
    let (text, muted) = (rgb(TEXT), rgb(MUTED));
    let mut spans = Vec::new();
    if !o.learned.is_empty() {
        spans.push(("Learned ".to_string(), text));
        for (i, (spoken, written)) in o.learned.iter().take(NAMED).enumerate() {
            if i > 0 {
                spans.push((", ".into(), muted));
            }
            spans.push((spoken.clone(), rgb(SPOKEN)));
            spans.push((" → ".into(), muted));
            spans.push((written.clone(), rgb(AMBER)));
        }
        if o.learned.len() > NAMED {
            spans.push((format!(" +{} more", o.learned.len() - NAMED), muted));
        }
    }
    match (spans.is_empty(), o.copied) {
        (true, false) => return None,
        (true, true) => spans.push(("Copied, press Ctrl+V to paste".into(), text)),
        (false, true) => spans.push((" · Copied, press Ctrl+V".into(), text)),
        (false, false) => {}
    }
    Some(Message(spans))
}
```

Replace `fix_last` and `paste_or_copy_correction` with:
```rust
pub fn fix_last(history: &mut History, dict: &Arc<Mutex<Dictionary>>, tray: &Tray) -> FixOutcome {
    let Some(last) = history.last().cloned() else {
        return FixOutcome { nothing_to_fix: true, ..Default::default() };
    };
    let target_hwnd = inject::foreground_hwnd();
    let snapshot = dict.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let heard_at = last.inject.as_ref().map(|r| r.at);
    let (edited, card) = correction_ui::show(&last.cleaned, heard_at, snapshot, HWND(target_hwnd as *mut _));
    let mut out = FixOutcome { card, ..Default::default() };
    let Some(edited) = edited else { return out };
    let edited = edited.trim().to_string();
    if edited == last.cleaned {
        return out;
    }
    {
        let mut d = dict.lock().unwrap_or_else(|e| e.into_inner());
        match Dictionary::load_or_seed() {
            Ok(fresh) => {
                *d = fresh;
                let l = d.learn(&last.cleaned, &edited);
                if !l.is_empty() {
                    if let Err(e) = d.save() {
                        log::error!("save dictionary: {e}");
                    }
                }
                out.learned = l.iter().map(|t| (t.spoken.last().cloned().unwrap_or_default(), t.written.clone())).collect();
            }
            Err(e) => {
                log::error!("reload dictionary: {e}");
                tray.notify("Dictionary", "not loaded — fix dictionary.toml before corrections are saved");
            }
        }
    }
    let replaced = paste_or_copy_correction(history, &last, target_hwnd, edited);
    out.replaced = replaced;
    out.copied = !replaced;
    out
}

/// Replaces the last dictation in place when it's fresh, else puts the correction on the
/// clipboard. Returns whether it replaced.
fn paste_or_copy_correction(history: &mut History, last: &crate::history::Entry, target_hwnd: isize, edited: String) -> bool {
    let fresh = last
        .inject
        .as_ref()
        .map(|r| r.hwnd == target_hwnd && r.at.elapsed() < Duration::from_secs(60) && r.method == InjectMethod::Paste)
        .unwrap_or(false);
    if fresh {
        // the dialog took focus; hand it back to the target before undo+paste
        unsafe { bring_to_front(HWND(target_hwnd as *mut _)) };
        std::thread::sleep(Duration::from_millis(150));
        if inject::foreground_hwnd() == target_hwnd {
            match inject::undo_then_paste(&edited) {
                Ok(_) => {
                    history.replace_last_cleaned(edited);
                    return true;
                }
                Err(e) => log::error!("replace: {e}"),
            }
        }
    }
    let _ = inject::set_clipboard_text(&edited);
    history.replace_last_cleaned(edited);
    false
}
```
`fix_last`'s two callers in `main.rs` won't compile until Task 6. Until then, make them `let _ = correction::fix_last(&mut history, &dict, &tray);` so this task builds on its own.

- [ ] **Step 5: Run to verify they pass**

Run: `cmd //c "%TEMP%\\murmur-test.cmd" --bin murmur`
Expected: all bin tests pass, including the 5 new `correction::tests`.

- [ ] **Step 6: Commit**

```bash
git add src/correction.rs src/correction_ui.rs src/main.rs
git commit -m "refactor(fix-last): report an outcome instead of balloons" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Wire the answers in `main.rs`

**Files:**
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: `Mote::{say, dismiss, is_speaking, launch, dissolve}`, `mote::{Message, Target, caret_point}` (Tasks 3–4); `Overlay::{above_physical, centre_physical}` (Task 4); `correction::{fix_last, fix_message, FixOutcome}` (Task 5).
- Produces: `fn unheard(expect_words: bool, mote_active: bool, caret_pending: bool) -> Option<Unheard>`; `enum Unheard { Here, WaitForCaret, AbovePill }`.

- [ ] **Step 1: Write the failing test** (append to `mod tests` in `src/main.rs`)

```rust
    #[test]
    fn only_a_finished_dictation_with_no_words_is_unheard() {
        assert_eq!(unheard(false, true, false), None, "a cancelled hold (tap, Esc, Shift) says nothing");
        assert_eq!(unheard(true, true, false), Some(Unheard::Here), "the mote is out: it says it where it is");
        assert_eq!(unheard(true, false, true), Some(Unheard::WaitForCaret), "the caret lookup is still running");
        assert_eq!(unheard(true, false, false), Some(Unheard::AbovePill), "no caret found");
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cmd //c "%TEMP%\\murmur-test.cmd" --bin murmur only_a_finished`
Expected: compile error, `cannot find function unheard`.

- [ ] **Step 3: Implement the decision and helpers** (near `reopen_after_loss` in `src/main.rs`)

```rust
const UNHEARD: &str = "Didn't catch that";

/// Where "Didn't catch that" goes when a dictation ends with no words.
#[derive(Debug, PartialEq)]
enum Unheard {
    /// the mote is already out: it says it where it is
    Here,
    /// the caret is still being looked up: say it once it's known
    WaitForCaret,
    AbovePill,
}

/// Only a finished dictation (`expect_words`: a Release sent Stop) is unheard; a cancelled
/// hold ends with no words too, and says nothing.
fn unheard(expect_words: bool, mote_active: bool, caret_pending: bool) -> Option<Unheard> {
    if !expect_words {
        return None;
    }
    Some(if mote_active {
        Unheard::Here
    } else if caret_pending {
        Unheard::WaitForCaret
    } else {
        Unheard::AbovePill
    })
}

/// The mote says `m`, flying from `from` to `to`, or to just above the pill with no caret.
/// Returns the window a caret message is about, for `said_in`.
fn say(mote: &mut Mote, overlay: &Overlay, m: Message, from: (f32, f32), to: Option<(f32, f32)>) -> Option<isize> {
    match to {
        Some(p) => {
            mote.say(m, from, Target::Caret(p));
            Some(inject::foreground_hwnd())
        }
        None => {
            mote.say(m, from, Target::Pill(overlay.above_physical()));
            None
        }
    }
}

/// The caret in the window in front, if it has one.
fn foreground_caret() -> Option<(f32, f32)> {
    match caret::find(HWND(inject::foreground_hwnd() as *mut _)) {
        Some(caret::Anchor::Caret(r)) => Some(mote::caret_point(r)),
        _ => None,
    }
}

/// Answers fix-last at the caret. From the tray with nothing to fix, above the pill instead.
fn after_fix(out: correction::FixOutcome, from_tray: bool, mote: &mut Mote, overlay: &Overlay) -> Option<isize> {
    let to = if out.nothing_to_fix && from_tray { None } else { foreground_caret() };
    let from = out.card.unwrap_or_else(|| overlay.centre_physical());
    match correction::fix_message(&out) {
        Some(m) => say(mote, overlay, m, from, to),
        None => {
            if let (true, Some(to)) = (out.replaced, to) {
                mote.launch(from, to);
                mote.dissolve();
            }
            None
        }
    }
}
```
Update the imports: `use mote::{landing_point, on_done, Landing, Message, Mote, Target};`, and `use overlay::Overlay` if `Overlay` isn't already in scope (check the existing `use overlay::…` line).

- [ ] **Step 4: Run the test**

Run: `cmd //c "%TEMP%\\murmur-test.cmd" --bin murmur only_a_finished`
Expected: PASS.

- [ ] **Step 5: Wire the loop**

After `let mut awaiting: Option<isize> = None;` add:
```rust
    // a Release sent Stop, so the dictation's Done should carry words
    let mut expect_words = false;
    // the caret lookup for the last Release hasn't come back yet
    let mut caret_pending = false;
    // "Didn't catch that" waiting on that lookup
    let mut unheard_pending = false;
    // the window a caret message is about; leaving it closes the message
    let mut said_in: Option<isize> = None;
```

Right after `mote.animate();` at the top of the loop:
```rust
        if said_in.is_some_and(|w| w != inject::foreground_hwnd()) {
            mote.dismiss();
            said_in = None;
        }
```

`TrayEvent::FixLast`:
```rust
                TrayEvent::FixLast => {
                    let out = correction::fix_last(&mut history, &dict, &tray);
                    said_in = after_fix(out, true, &mut mote, &overlay);
                    while hk_rx.try_recv().is_ok() {}
                }
```
`HotkeyEvent::FixLast`:
```rust
                HotkeyEvent::FixLast => {
                    log::info!("fix-last hotkey");
                    let out = correction::fix_last(&mut history, &dict, &tray);
                    said_in = after_fix(out, false, &mut mote, &overlay);
                    while hk_rx.try_recv().is_ok() {}
                }
```

At the start of `HotkeyEvent::Down if !paused => {`:
```rust
                    // a new dictation closes the last message
                    if mote.is_speaking() {
                        mote.dismiss();
                    }
                    said_in = None;
```

In `HotkeyEvent::Cancel`, after `awaiting = None;`:
```rust
                    expect_words = false;
                    unheard_pending = false;
```

In `HotkeyEvent::Release`, inside `if dictating {`, after `awaiting = Some(target);`:
```rust
                        expect_words = true;
                        caret_pending = true;
```

Replace the `caret_rx` loop:
```rust
        while let Ok((target, caret)) = caret_rx.try_recv() {
            caret_pending = false;
            let to = landing_point(awaiting, target, inject::foreground_hwnd(), caret);
            if std::mem::take(&mut unheard_pending) {
                awaiting = None;
                said_in = say(&mut mote, &overlay, Message::plain(UNHEARD), overlay.centre_physical(), to);
            } else if let Some(to) = to {
                mote.launch(overlay.centre_physical(), to);
                overlay.set_quiet(true);
            }
        }
```

Replace the `PipelineMsg::Done(e)` arm:
```rust
                PipelineMsg::Done(e) => {
                    overlay.set(resting(paused));
                    overlay.set_quiet(false);
                    let heard = !e.cleaned.is_empty();
                    let finished = std::mem::take(&mut expect_words);
                    match unheard(finished && !heard, mote.is_active(), caret_pending) {
                        Some(Unheard::WaitForCaret) => unheard_pending = true,
                        Some(Unheard::Here) => {
                            // the mote is at (or flying to) the caret: it says it there, whatever the target
                            awaiting = None;
                            mote.say(Message::plain(UNHEARD), overlay.centre_physical(), Target::Pill(overlay.above_physical()));
                            said_in = Some(inject::foreground_hwnd());
                        }
                        Some(Unheard::AbovePill) => {
                            awaiting = None;
                            said_in = say(&mut mote, &overlay, Message::plain(UNHEARD), overlay.centre_physical(), None);
                        }
                        None => {
                            let same = awaiting.take().is_some_and(|t| t == inject::foreground_hwnd());
                            match on_done(mote.is_active(), heard, same) {
                                Landing::Dissolve => mote.dissolve(),
                                Landing::Fade => mote.fade(),
                                Landing::Pulse => overlay.pulse(),
                                Landing::Nothing => {}
                            }
                        }
                    }
                    if heard {
                        history.push(e);
                    }
                }
```
With `Here`, the mote is already out, so `Flight::say` keeps it where it is and ignores the target. The target only matters if the mote was mid-fade (`Leaving`); then it flies above the pill instead.

In `PipelineMsg::Error(s)`, after `awaiting = None;` add `expect_words = false; unheard_pending = false;`.

- [ ] **Step 6: Build and run the whole suite**

Run: `cmd //c "%TEMP%\\murmur-test.cmd"`
Expected: lib 170, bin all passing (142 plus the tests added in Tasks 2–6), stt 1. No warnings about unused `say`/`dismiss`/`is_speaking`/`above_physical`.

- [ ] **Step 7: Commit**

```bash
git add src/main.rs
git commit -m "feat: answer nothing heard and fix-last at the caret" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: Type the text when the clipboard write fails

**Files:**
- Modify: `src/inject.rs:141-157`

**Interfaces:**
- Produces: `inject::paste` now returns `Ok(… Typed)` when the clipboard write fails.

There's no unit test: forcing `SetClipboardData` to fail needs a seam the spec ruled not worth adding. The Task 9 smoke run covers it where possible.

- [ ] **Step 1: Implement** (replace the `let saved = match Clipboard::open() { … };` block in `paste`)

```rust
    // None: the clipboard can't be used, so type the text in instead
    let saved = match Clipboard::open() {
        Ok(_c) => {
            let saved = read_text_locked();
            match write_text_locked(text) {
                Ok(()) => Some(saved),
                Err(e) => {
                    log::warn!("{e}; falling back to unicode typing");
                    if let Some(prev) = &saved {
                        let _ = write_text_locked(prev);
                    }
                    None
                }
            }
        }
        Err(e) => {
            log::warn!("{e}; falling back to unicode typing");
            None
        }
    };
    let Some(saved) = saved else {
        let hwnd = foreground_hwnd();
        type_unicode(text);
        return Ok(record(hwnd, InjectMethod::Typed));
    };
```
The clipboard guard `_c` is dropped at the end of the `Ok` arm, so typing happens with the clipboard closed. Also update the doc comment above `paste`: `/// Save clipboard text, set ours, Ctrl+V, restore. Types the text in when the clipboard can't be opened or written.`

- [ ] **Step 2: Build and run the suite**

Run: `cmd //c "%TEMP%\\murmur-test.cmd"`
Expected: all green, no new warnings.

- [ ] **Step 3: Commit**

```bash
git add src/inject.rs
git commit -m "fix(paste): type the text when the clipboard can't be written" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: History opens with an empty state

**Files:**
- Modify: `src/history_ui.rs`
- Modify: `src/main.rs` (`TrayEvent::History`)

**Interfaces:**
- Consumes: the `app()`/`frame()` test helpers already in `history_ui.rs`'s tests (from the close-flash fix).
- Produces: `history_ui::show(items: Vec<(String, Instant)>, key: String)`; `HistoryApp` gains `key: String`.

- [ ] **Step 1: Write the failing test** (append to `mod tests` in `src/history_ui.rs`)

```rust
    #[test]
    fn nothing_dictated_says_how_to_start() {
        let ctx = egui::Context::default();
        let mut app = HistoryApp { items: Vec::new(), key: "Right Ctrl".into(), ..app() };
        let (text, _) = frame(&ctx, &mut app, vec![]);
        assert!(text.iter().any(|t| t == "Nothing dictated yet. Hold Right Ctrl and speak."), "{text:?}");
        assert!(!text.iter().any(|t| t.contains("click to copy")), "nothing to copy: {text:?}");
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cmd //c "%TEMP%\\murmur-test.cmd" --bin murmur nothing_dictated`
Expected: compile error, `struct HistoryApp has no field named key`.

- [ ] **Step 3: Implement**

In `src/history_ui.rs`:
- add `/// the talk key's label, for the empty state` and `key: String,` to `HistoryApp`;
- in the test helper `app()`, add `key: "Right Ctrl".into(),`;
- the header label becomes:
```rust
                    let head = if self.items.is_empty() { "History" } else { "History · click to copy" };
                    ui.label(egui::RichText::new(head).size(12.0).color(MUTED));
```
- right after `ui.add_space(6.0);` that follows the header row, before the `ScrollArea`:
```rust
                if self.items.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        ui.label(egui::RichText::new(format!("Nothing dictated yet. Hold {} and speak.", self.key)).size(15.0).color(TEXT));
                    });
                    ui.add_space(6.0);
                }
```
- `show` becomes `pub fn show(items: Vec<(String, Instant)>, key: String)`, and its constructor passes `key`.

In `src/main.rs`, `TrayEvent::History`:
```rust
                TrayEvent::History => {
                    history_ui::show(history.newest_first().map(|e| (e.cleaned.clone(), e.at)).collect(), cfg.ptt_key_label());
                    while hk_rx.try_recv().is_ok() {}
                }
```

- [ ] **Step 4: Run to verify it passes**

Run: `cmd //c "%TEMP%\\murmur-test.cmd" --bin murmur history_ui`
Expected: all `history_ui` tests pass (`ago_buckets`, `the_card_stays_drawn_while_it_closes`, `nothing_dictated_says_how_to_start`).

- [ ] **Step 5: Confirm no user-triggered balloons remain**

Run: `grep -n "tray.notify" src/correction.rs src/main.rs`
Expected: `correction.rs` keeps only "Dictionary … not loaded". `main.rs` no longer has `"History", "nothing dictated yet"`, and every remaining call is a background event or an error the spec keeps as a balloon.

- [ ] **Step 6: Commit**

```bash
git add src/history_ui.rs src/main.rs
git commit -m "feat(history): open with how to start when nothing's dictated" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 9: Frames, critic, ledger, smoke

**Files:**
- Modify: `~/.claude/skills/considered-ux/kits/jeff.md` (ledger row)

- [ ] **Step 1: Self-review.** Re-read `mote.rs`, `canvas.rs`, `correction.rs` and the `main.rs` loop in full (not just the diff). Look for orphaned imports, a `said_in` that can go stale, and the `Flying { then: Say }` → `launch` replacement path.

- [ ] **Step 2: Release build**

Run: `cmd //c "%TEMP%\\murmur-build.cmd"`
Expected: `Finished release`.

- [ ] **Step 3: Frames (needs Jeff).** Ask Jeff to quit the installed Murmur and start a debug build with `MOTION_TIME_SCALE=10`. A Claude session must never launch Murmur. Then capture, with the window title `MurmurMote`:

```bash
powershell.exe -NoProfile -ExecutionPolicy Bypass -File ~/.claude/skills/considered-ux/scripts/frames-native.ps1 -Title "MurmurMote" -Out "$TEMP/frames/unheard"
node ~/.claude/skills/considered-ux/scripts/strip.mjs --dir "$TEMP/frames/unheard" --out "$TEMP/frames/unheard/strip.png"
```
Capture "Didn't catch that" at a Notepad caret, and "Learned hob → HAWB" after fix-last. Then repeat both with Windows animations off (reduced motion). Copy the frames that span each transition into their own folder before stripping. Open every strip and check it against the five questions in `references/editor-pass.md`.

- [ ] **Step 4: Zombie critic.** Dispatch a fresh subagent (model: sonnet) with only the strip paths, the visible text, the kit's beliefs and rules from `jeff.md`, and `references/zombie-tells.md`, using the critic prompt in `references/editor-pass.md`. Fix each finding or write one line on why it stays.

- [ ] **Step 5: Ledger.** Add a row to the `# Ledger` table in `jeff.md`:
```markdown
| 2026-10-02 | murmur | Answers where you acted | The mote speaks: it stretches into a capsule above the caret holding "Didn't catch that" or "Learned hob → HAWB", holds for read time, pulls back to a dot and dissolves | I want to hear and understand you | flight FLIGHT (break) + easing.enter; unfurl duration.enter + easing.enter; hold max(duration.locate, duration.readPerChar × chars); furl duration.exit + easing.exit |
```
Run: `node ~/.claude/skills/considered-ux/scripts/kit.mjs --check ~/.claude/skills/considered-ux/kits/jeff.md`
Expected: the check passes.

- [ ] **Step 6: Smoke (Jeff, on the release build).** One run of each spec Behaviour row:
  1. Hold the key in Notepad, say nothing → "Didn't catch that" above the caret.
  2. The same in Chrome (UIA caret) → at the caret, not above the pill.
  3. Hold the key on the desktop (no caret) → above the pill, centred.
  4. Dictate, fix-last with a learnable edit → "Learned … → …" at the caret, no balloon.
  5. Fix-last more than 60 s after dictating → "Copied, press Ctrl+V to paste".
  6. Shift + talk key with no history → "Nothing to fix yet" at the caret; from the tray → above the pill.
  7. Tray → History with nothing dictated → the window opens with the empty state.
  8. A tap of the talk key (under 150 ms) and an Esc-cancelled hands-free → no message.
  9. With Windows animations off → the tag appears open, holds, disappears.

Report each row's result; a clean build is not a smoke test.
