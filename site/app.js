(() => {
  const $ = (id) => document.getElementById(id);
  const stage = $("stage"), keycap = $("keycap"), notes = $("notes"), pill = $("pill"),
    typed = $("typed"), placeholder = $("placeholder"), hintEl = $("hint"), askLine = $("ask-line");
  const bars = [...pill.querySelectorAll(".bar")];

  const reduced = matchMedia("(prefers-reduced-motion: reduce)");
  const css = getComputedStyle(document.documentElement);
  const ms = (name) => parseFloat(css.getPropertyValue(name));
  const tok = (name) => css.getPropertyValue(name).trim();
  const T = {
    enter: ms("--motion-duration-enter"), exit: ms("--motion-duration-exit"), emphasis: ms("--motion-duration-emphasis"),
    bump: ms("--motion-duration-bump"), locate: ms("--motion-duration-locate"), bumpScale: parseFloat(tok("--motion-scale-bump")),
    easeEnter: tok("--motion-easing-enter"), easeExit: tok("--motion-easing-exit"),
  };

  // The key the real app uses on this machine.
  const ua = navigator.userAgentData && navigator.userAgentData.platform || navigator.platform || navigator.userAgent || "";
  const isMac = /mac|iphone|ipad/i.test(ua) && !/iphone|ipad/i.test(navigator.userAgent);
  const isWindows = /win/i.test(ua);
  const coarse = matchMedia("(pointer: coarse)").matches;
  const KEY = isMac
    ? { code: "AltRight", twin: "AltLeft", name: "Right Option", glyph: "⌥" }
    : { code: "ControlRight", twin: "ControlLeft", name: "Right Ctrl", glyph: "ctrl" };

  $("key-name").textContent = KEY.name;
  $("key-glyph").textContent = KEY.glyph;
  $("key-cap-name").textContent = KEY.name;
  if (coarse) {
    askLine.innerHTML = "Hold the key and talk.";
    $("key-help").textContent = "Press and hold the key.";
  }
  const youEl = isMac ? $("plat-macos") : isWindows ? $("plat-windows") : null;
  if (youEl) {
    youEl.classList.add("you");
    youEl.querySelector("h3").insertAdjacentHTML("beforeend", ' <span class="badge you">your machine</span>');
  }

  const PHRASES = [
    "Pick up milk and coffee on the way home.",
    "Deploy to Kubernetes.",
    "Send it to Grainne.",
  ];
  let next = 0, lines = [];
  let held = null;       // { at, via, raf }
  let pending = null;    // a landing in progress: { words, mote, anims, timers }
  let hintTimer = 0;
  let learned = false;

  const mote = document.createElement("div");
  mote.className = "mote";
  mote.setAttribute("aria-hidden", "true");
  stage.appendChild(mote);

  function hint(text) {
    hintEl.textContent = text;
    clearTimeout(hintTimer);
    if (text) hintTimer = setTimeout(() => (hintEl.textContent = ""), 4000);
  }

  function center(el) {
    const r = el.getBoundingClientRect(), s = stage.getBoundingClientRect();
    return { x: r.left - s.left + r.width / 2, y: r.top - s.top + r.height / 2 };
  }

  // ribbon: voice-shaped bars while the key is down
  function ribbon(t0) {
    const loop = (now) => {
      if (!held) return;
      const t = now - t0;
      bars.forEach((b, i) => {
        const env = (0.5 + 0.5 * Math.sin(t * 0.012 + i * 1.3)) * (0.55 + 0.45 * Math.sin(t * 0.0071 + i * 2.1));
        b.style.transform = `scaleY(${0.2 + 0.8 * env})`;
      });
      held.raf = requestAnimationFrame(loop);
    };
    held.raf = requestAnimationFrame(loop);
  }

  function start(via) {
    if (held) return;
    finishPending();                 // a new hold redirects from wherever the last landing is
    hint("");
    held = { at: performance.now(), via, raf: 0 };
    keycap.classList.add("down");
    pill.classList.add("on");
    if (reduced.matches) bars.forEach((b) => (b.style.transform = "scaleY(0.6)"));
    else ribbon(held.at);
  }

  function stop(cancel) {
    if (!held) return;
    const h = held;
    held = null;
    cancelAnimationFrame(h.raf);
    keycap.classList.remove("down");
    pill.classList.remove("on");
    bars.forEach((b) => (b.style.transform = "scaleY(0.15)"));
    if (cancel) return;
    if (performance.now() - h.at < 250) {
      hint(`Hold ${KEY.name} a little longer. It listens while the key is down.`);
      return;
    }
    land();
  }

  // Put the next phrase in the field; fly the mote there, then dissolve into the words.
  function land() {
    if (lines.length === PHRASES.length) { lines = []; typed.textContent = ""; }
    const text = PHRASES[next++ % PHRASES.length];
    lines.push(text);
    placeholder.hidden = true;

    const line = document.createElement("span");
    line.className = "line";
    const words = text.split(" ").map((w, i) => {
      const s = document.createElement("span");
      s.className = "word";
      s.textContent = w;
      line.append(s, i < text.split(" ").length - 1 ? " " : "");
      return s;
    });
    if (typed.firstChild) typed.appendChild(document.createElement("br"));
    typed.appendChild(line);

    const p = (pending = { words, line, anims: [], timers: [] });
    p.done = () => {
      words.forEach((w) => w.classList.add("in"));
      mote.style.opacity = "0";
      p.anims.forEach((a) => a.cancel());
      p.timers.forEach(clearTimeout);
      learnedCue();
      if (pending === p) pending = null;
    };

    if (reduced.matches) {
      // No movement: the words are simply there, and the field answers with a color change.
      p.done();
      notes.classList.add("landed");
      p.timers.push(setTimeout(() => notes.classList.remove("landed"), T.locate));
      return;
    }

    const from = center(pill), to = center(words[0]);
    const dx = to.x - from.x, dy = to.y - from.y, len = Math.hypot(dx, dy);
    const lift = Math.max(24, 0.2 * len);
    mote.style.opacity = "1";
    const fly = mote.animate([
      { transform: `translate(${from.x}px, ${from.y}px) scale(1)` },
      { transform: `translate(${from.x + dx / 2}px, ${from.y + dy / 2 - lift}px) scale(1)`, offset: 0.5 },
      { transform: `translate(${to.x - 14}px, ${to.y}px) scale(1)` },
    ], { duration: T.emphasis, easing: T.easeEnter, fill: "forwards" });
    p.anims.push(fly);

    fly.onfinish = () => {
      // breathe at the caret while the words are "heard"
      const breath = mote.animate([
        { transform: `translate(${to.x - 14}px, ${to.y}px) scale(1)` },
        { transform: `translate(${to.x - 14}px, ${to.y}px) scale(${T.bumpScale})` },
        { transform: `translate(${to.x - 14}px, ${to.y}px) scale(1)` },
      ], { duration: T.bump, easing: "ease-in-out", fill: "forwards" });
      p.anims.push(breath);
      breath.onfinish = () => {
        // dissolve into the words
        mote.animate([{ opacity: 1 }, { opacity: 0 }], { duration: T.exit, easing: T.easeExit, fill: "forwards" });
        words.forEach((w, i) => p.timers.push(setTimeout(() => w.classList.add("in"), i * 45)));
        p.timers.push(setTimeout(() => { learnedCue(); if (pending === p) pending = null; }, words.length * 45 + T.enter));
      };
    };
  }

  function finishPending() { if (pending) pending.done(); }

  function learnedCue() {
    if (learned) return;
    learned = true;
    askLine.innerHTML = "That's all there is to it. Hold <strong>" + KEY.name + "</strong> again.";
  }

  // Real key
  addEventListener("keydown", (e) => {
    if (e.target.closest && e.target.closest("video, input, textarea, select, [contenteditable]")) return;
    if (e.code === KEY.code) {
      if (!e.repeat) start("key");
    } else if (e.code === KEY.twin) {
      if (!held) hint(`That's the left one. Try ${KEY.name}.`);
    } else if (held && held.via === "key") {
      stop(true);                   // a chord, not a dictation
    } else if (e.code === "Space" && document.activeElement === keycap && !e.repeat) {
      e.preventDefault();
      start("space");
    }
  });
  addEventListener("keyup", (e) => {
    if (e.code === KEY.code && held && held.via === "key") stop(false);
    if (e.code === "Space" && held && held.via === "space") stop(false);
  });
  addEventListener("blur", () => stop(true));
  document.addEventListener("visibilitychange", () => { if (document.hidden) stop(true); });

  // Pointer on the keycap (touch, or no suitable key)
  keycap.addEventListener("pointerdown", (e) => {
    keycap.setPointerCapture(e.pointerId);
    start("pointer");
  });
  const release = (cancel) => () => { if (held && held.via === "pointer") stop(cancel); };
  keycap.addEventListener("pointerup", release(false));
  keycap.addEventListener("pointercancel", release(true));
  keycap.addEventListener("contextmenu", (e) => e.preventDefault());
})();
