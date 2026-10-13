// The hero: words as they're heard scatter over a window, break into a murmuration of fireflies,
// pour into Murmur's pill, and one mote flies to the caret and bursts into the clean sentence.
// Hold your push-to-talk key (or press and hold the pill) to drive it yourself.
(() => {
  // A reload starts at the top. Browsers otherwise go back to where you were, or to the section a
  // Download link last put in the address, which on this short page is the very bottom.
  if (performance.getEntriesByType?.("navigation")[0]?.type === "reload") {
    history.scrollRestoration = "manual";
    if (location.hash) history.replaceState(null, "", location.pathname + location.search);
    const top = () => scrollTo({ top: 0, behavior: "instant" });
    top();
    addEventListener("load", top, { once: true });
    // back and forward still return you to where you were
    addEventListener("pagehide", () => { history.scrollRestoration = "auto"; }, { once: true });
  }

  const $ = (id) => document.getElementById(id);
  const stage = $("stage"), cv = $("fx"), ctx = cv.getContext("2d");
  const field = $("field"), statusEl = $("status"), pill = $("pill"), live = $("said-live");
  const bars = [...pill.querySelectorAll("i")];

  // The key the real app uses on this machine, and this machine's download first.
  const platform = navigator.userAgentData?.platform || navigator.platform || navigator.userAgent || "";
  const isMac = /mac/i.test(platform) && !/iphone|ipad/i.test(navigator.userAgent);
  const KEY = isMac ? { codes: ["AltRight", "AltLeft"], name: "Right Option" } : { codes: ["ControlRight", "ControlLeft"], name: "Right Ctrl" };
  $("key-name").textContent = KEY.name;
  $(isMac ? "plat-macos" : "plat-windows").classList.add("you");
  if (isMac) { $("cta-label").textContent = "Download the Mac preview"; $("cta-alt").textContent = "Windows"; }
  if (matchMedia("(pointer: coarse)").matches) $("try").textContent = "Press and hold the pill to try it. A scripted demo, no microphone.";

  const FILLERS = new Set(["um", "uh", "er", "hmm", "mm"]);
  const PHRASES = [
    { raw: "um pick up milk and uh coffee on the way home", out: "Pick up milk and coffee on the way home.", tag: "Fillers dropped, punctuation added" },
    { raw: "uh deploy to cooper netties", wrong: ["cooper", "netties"], out: "Deploy to Kubernetes.", tag: "Dictionary: “cooper netties” → Kubernetes" },
    { raw: "send the notes to grain by friday um", wrong: ["grain"], out: "Send the notes to Grainne by Friday.", tag: "Learned from one fix: grain → Grainne" },
  ];
  const BIRDS_PER_LETTER = 5;
  const GLOW = "226,242,120", LETTER = "rgb(196,206,232)", WRONG = "#ff9a85";
  const MONO = '"JetBrains Mono", ui-monospace, monospace';

  // The mess is brief and the sentence holds longest. Pressing your key shortens the flock.
  function timings(manual) {
    const T = { mess: 1400, dissolve: 550, swirl: manual ? 800 : 1500, stream: 650, gather: 250, flight: 550, dwell: 150, burst: 550, hold: 5000 };
    const E = { morph: T.mess + T.dissolve + T.swirl + T.stream };
    E.gather = E.morph + T.gather; E.flight = E.gather + T.flight; E.dwell = E.flight + T.dwell; E.burst = E.dwell + T.burst; E.all = E.burst + T.hold;
    return { T, E };
  }

  let W = 0, H = 0, raf = 0, running = false, pausedAt = 0, run = null, sparks = [];

  // seeded, so each phrase's mess looks the same every loop
  function rng(seed) { return () => { seed |= 0; seed = (seed + 0x6d2b79f5) | 0; let t = Math.imul(seed ^ (seed >>> 15), 1 | seed); t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t; return ((t ^ (t >>> 14)) >>> 0) / 4294967296; }; }
  const clamp = (v, a = 0, b = 1) => Math.min(b, Math.max(a, v));
  const smooth = (v) => { v = clamp(v); return v * v * (3 - 2 * v); };
  const inOut = (v) => { v = clamp(v); return v < 0.5 ? 4 * v * v * v : 1 - Math.pow(-2 * v + 2, 3) / 2; };
  const lerp = (a, b, t) => a + (b - a) * t;

  // one glow, drawn once and stamped for every mote
  const sprite = document.createElement("canvas");
  sprite.width = sprite.height = 64;
  {
    const g = sprite.getContext("2d"), grd = g.createRadialGradient(32, 32, 0, 32, 32, 32);
    grd.addColorStop(0, "rgba(255,253,225,1)"); grd.addColorStop(0.14, `rgba(${GLOW},0.95)`);
    grd.addColorStop(0.38, `rgba(${GLOW},0.28)`); grd.addColorStop(1, `rgba(${GLOW},0)`);
    g.fillStyle = grd; g.fillRect(0, 0, 64, 64);
  }
  const dot = (x, y, r, a) => { if (a <= 0.01) return; ctx.globalAlpha = Math.min(1, a); ctx.drawImage(sprite, x - r * 4, y - r * 4, r * 8, r * 8); };

  function resize() {
    const r = stage.getBoundingClientRect(), dpr = Math.min(2, devicePixelRatio || 1);
    W = r.width; H = r.height;
    cv.width = Math.round(W * dpr); cv.height = Math.round(H * dpr);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  }
  function rel(el) {
    const s = stage.getBoundingClientRect(), r = el.getBoundingClientRect();
    return { x: r.left - s.left + r.width / 2, y: r.top - s.top + r.height / 2, h: r.height };
  }

  function build(ph, seed) {
    const rand = rng(seed), parts = [], birds = [];
    // four clumps over the top of the window: the words as heard, out of order
    const clumps = [[0.14, 0.24], [0.44, 0.1], [0.3, 0.44], [0.58, 0.32]];
    ph.raw.split(" ").forEach((w, wi) => {
      const c = clumps[(wi * 3) % 4];
      const filler = FILLERS.has(w), wrong = (ph.wrong || []).includes(w);
      for (const ch of w) parts.push({
        ch, filler, wrong,
        nx: clamp(c[0] + (rand() - 0.5) * 0.3, 0.04, 0.94), ny: clamp(c[1] + (rand() - 0.5) * 0.28, 0.05, 0.62),
        rot: (rand() - 0.5) * 1.4, size: 15 + rand() * 17, ph: rand() * 6.283,
        fx: (rand() - 0.5) * 2, fy: -0.5 - rand(), d: 0, at: 0,
      });
    });
    const n = parts.length;
    parts.forEach((p, k) => { p.d = (k / n) * 0.32 + rand() * 0.06; p.at = (k / n) * 650; });
    // each spoken letter breaks into a few fireflies, each with a place in the flock
    parts.forEach((p, k) => {
      if (p.filler) return;
      for (let j = 0; j < BIRDS_PER_LETTER; j++) {
        const a = rand() * 6.283;
        birds.push({ p: k, a, r: Math.sqrt(rand()), z: rand() * 2 - 1, ox: (rand() - 0.5) * 2, oy: (rand() - 0.5) * 2, ph: rand() * 6.283, d: (a / 6.283) * 0.6 });
      }
    });
    return { parts, birds };
  }

  // the window's lines; the caret sits at the end of the last one
  const lines = () => [...field.querySelectorAll(".line")];
  const hasText = (l) => !!l.querySelector(".w, .t");
  function tidy() {
    const caret = $("caret"), all = lines(), full = all.filter(hasText);
    all.forEach((l) => { if (!hasText(l)) l.remove(); });
    if (full.length) full[full.length - 1].appendChild(caret);
    else { const l = document.createElement("div"); l.className = "line"; l.appendChild(caret); field.appendChild(l); }
  }
  function clearField(fade) {
    const reset = () => { field.innerHTML = '<div class="line"><span class="caret" id="caret"></span></div>'; field.classList.remove("clearing"); };
    if (!fade) return reset();
    field.classList.add("clearing");
    setTimeout(reset, 520);
  }
  function newLine() {
    const caret = $("caret"), l = document.createElement("div");
    l.className = "line";
    if (!lines().some(hasText)) field.innerHTML = "";
    field.appendChild(l); l.appendChild(caret);
    return l;
  }
  function typeWords(line, text, from) {
    const caret = $("caret");
    const spans = text.split(" ").map((w, i) => {
      const s = document.createElement("span");
      s.className = "w"; s.textContent = (i ? " " : "") + w; s.style.transitionDelay = `${i * 45}ms`;
      line.insertBefore(s, caret);
      return s;
    });
    // each word starts at the point where the mote landed
    for (const s of spans) { const r = rel(s); s.style.setProperty("--dx", `${(from.x - r.x).toFixed(1)}px`); s.style.setProperty("--dy", `${(from.y - r.y).toFixed(1)}px`); }
    requestAnimationFrame(() => requestAnimationFrame(() => spans.forEach((s) => s.classList.add("in"))));
  }
  function showTag(ph, announce) {
    statusEl.textContent = ph.tag; statusEl.classList.add("show");
    // only dictations you start are read out; the loop stays quiet
    if (announce) live.textContent = `Murmur typed: ${ph.out}`;
  }

  function startPhrase(i, now, manual) {
    tidy();
    let delay = 0;
    if (lines().filter(hasText).length >= PHRASES.length) { clearField(true); delay = 650; }
    run = { i, ph: PHRASES[i], ...build(PHRASES[i], 7 + i * 13), ...timings(manual), t0: now + delay, fired: {}, manual, held: false, line: null };
    sparks = [];
    statusEl.classList.remove("show");
  }

  function setBars(level, now) {
    bars.forEach((b, i) => {
      const n = 0.5 + 0.5 * Math.sin(now / 90 + i * 1.7) * Math.sin(now / 237 + i);
      b.style.transform = `scaleY(${(0.18 + level * (0.25 + 0.75 * n)).toFixed(3)})`;
    });
  }

  const letterAt = (p, now) => ({ x: p.nx * W + Math.sin(now / 700 + p.ph) * 7, y: p.ny * H + Math.cos(now / 820 + p.ph) * 6 });
  // letters arrive one after another, as they're heard
  const appear = (p, t) => smooth((t - p.at) / 220);
  function drawLetter(p, at, a, now) {
    if (a <= 0.01) return;
    ctx.save();
    ctx.globalCompositeOperation = "source-over";
    ctx.translate(at.x, at.y); ctx.rotate(p.rot + Math.sin(now / 600 + p.ph) * 0.12);
    ctx.globalAlpha = a * (p.filler ? 0.5 : 0.85);
    ctx.fillStyle = p.wrong ? WRONG : LETTER;
    ctx.font = `${p.filler ? "italic " : ""}500 ${p.size}px ${MONO}`;
    ctx.textAlign = "center"; ctx.textBaseline = "middle";
    ctx.fillText(p.ch, 0, 0);
    ctx.restore();
  }

  // The flock folds, stretches and banks like starlings at dusk, drifting over the window.
  function flockAt(b, s) {
    const cx = W * (0.46 + 0.15 * Math.sin(s * 1.15 + 0.4)), cy = H * (0.36 + 0.06 * Math.sin(s * 1.9));
    let x = b.r * Math.cos(b.a), y = b.r * Math.sin(b.a) * 0.34;
    x += 0.24 * Math.sin(y * 5 + s * 1.6);
    y += 0.13 * Math.sin(x * 3.2 + s * 1.1);
    const sx = 1 + 0.45 * Math.sin(s * 0.9), sy = 1 - 0.3 * Math.sin(s * 0.9 + 0.6);
    const psi = 0.5 * Math.sin(s * 0.7) + 0.22 * Math.sin(s * 1.3), c = Math.cos(psi), sn = Math.sin(psi);
    const depth = 1 / (1.2 - 0.3 * b.z * Math.cos(s * 0.8 + b.a));
    const R = Math.min(W, 640) * 0.22 * depth;
    return {
      x: cx + (c * x * sx - sn * y * sy) * R + Math.sin(s * 3 + b.ph) * 1.5,
      y: cy + (sn * x * sx + c * y * sy) * R + Math.cos(s * 2.6 + b.ph) * 1.5,
      depth,
    };
  }
  function drawFlock(t, now, pc) {
    const { T } = run, m = t - T.mess, s = t / 1000;
    const kDis = m / T.dissolve, sProg = (m - T.dissolve - T.swirl) / T.stream;
    for (const p of run.parts) {
      const h = letterAt(p, now);
      drawLetter(p, h, appear(p, t) * (1 - smooth((m - p.d * 300) / 400)), now);
      if (p.filler && m > 0) {
        // fillers fade and drift off instead of joining the flock
        const u = clamp(kDis);
        if (u < 1) dot(h.x + p.fx * 60 * u, h.y + p.fy * 70 * u, 2, smooth(u / 0.3) * (1 - smooth(u / 0.8)) * 0.6);
      }
    }
    if (m <= 0) return;
    for (const b of run.birds) {
      const f = flockAt(b, s);
      let x = f.x, y = f.y;
      if (kDis < 1) {
        // each letter bursts into its fireflies, which find their place in the flock
        const p = run.parts[b.p], h = letterAt(p, now), k = smooth((m - p.d * 250) / T.dissolve);
        if (k <= 0) continue;
        const bx = h.x + b.ox * 18 * smooth(k * 3), by = h.y + b.oy * 14 * smooth(k * 3);
        x = lerp(bx, f.x, inOut(k)); y = lerp(by, f.y, inOut(k));
      } else if (sProg > 0) {
        // then it pours into the pill as a ribbon
        const u = clamp((sProg - b.d) / 0.4);
        if (u >= 1) continue;
        const e = u * u;
        x = lerp(f.x, pc.x, e); y = lerp(f.y, pc.y, e) - Math.sin(u * Math.PI) * 18;
      }
      const tw = 0.8 + 0.2 * Math.sin(now / 110 + b.ph);
      dot(x, y, 1.05 + 0.85 * f.depth, (0.45 + 0.4 * f.depth) * tw * clamp(kDis * 3));
    }
  }

  function frame(now) {
    raf = requestAnimationFrame(frame);
    if (!run) return;
    let t = now - run.t0;
    // while your key is held, Murmur keeps hearing
    if (run.held && t > run.T.mess - 16) { run.t0 = now - (run.T.mess - 16); t = run.T.mess - 16; }
    if (t >= run.E.all) { startPhrase((run.i + 1) % PHRASES.length, now, false); t = now - run.t0; }
    ctx.clearRect(0, 0, W, H);
    if (t < 0) { setBars(0, now); return; }
    const { T, E, ph } = run, pc = rel(pill);

    setBars(t < E.gather ? smooth(t / 300) : 0, now);
    pill.classList.toggle("pulse", t > E.morph - 120 && t < E.gather + 80);

    ctx.save();
    ctx.globalCompositeOperation = "lighter";
    if (t < E.morph) drawFlock(t, now, pc);

    // the light gathers on top of the pill
    if (t >= E.morph && t < E.gather) {
      const k = smooth((t - E.morph) / T.gather);
      dot(pc.x, pc.y - pc.h / 2, 1.5 + 2.7 * k, 0.4 + 0.6 * k);
    }
    // one mote leaves the pill on an upward arc to the caret, as in the app
    if (t >= E.gather && !run.fired.line) { run.line = newLine(); run.fired.line = true; }
    if (t >= E.gather && t < E.burst) {
      const c = rel($("caret")), from = { x: pc.x, y: pc.y - pc.h / 2 };
      const arc = (k) => {
        const dx = c.x - from.x, dy = c.y - from.y, len = Math.max(1, Math.hypot(dx, dy));
        let px = -dy / len, py = dx / len;
        if (py > 0) { px = -px; py = -py; }
        const bx = (from.x + c.x) / 2 + px * len * 0.22, by = (from.y + c.y) / 2 + py * len * 0.22, v = 1 - k;
        return { x: v * v * from.x + 2 * v * k * bx + k * k * c.x, y: v * v * from.y + 2 * v * k * by + k * k * c.y };
      };
      if (t < E.flight) {
        const k = inOut((t - E.gather) / T.flight);
        for (let j = 6; j >= 1; j--) { const q = arc(clamp(k - j * 0.035)); dot(q.x, q.y, 3.2 - j * 0.35, 0.5 - j * 0.07); }
        const q = arc(k); dot(q.x, q.y, 4.2, 1);
      } else if (t < E.dwell) {
        dot(c.x, c.y, 4.2 + 2 * smooth((t - E.flight) / T.dwell), 1);
      } else {
        // it bursts into the words
        if (!run.fired.burst) {
          run.fired.burst = true;
          typeWords(run.line, ph.out, c);
          for (let j = 0; j < 18; j++) { const a = (j / 18) * 6.283 + Math.random() * 0.3; sparks.push({ x: c.x, y: c.y, vx: Math.cos(a) * (50 + Math.random() * 90), vy: Math.sin(a) * (40 + Math.random() * 70) }); }
        }
        const k = (t - E.dwell) / T.burst;
        dot(c.x, c.y, 6.2 * (1 - smooth(k * 2.2)) + 0.1, 1 - smooth(k * 2));
      }
    }
    if (sparks.length) {
      const k = clamp((t - E.dwell) / T.burst);
      for (const sp of sparks) dot(sp.x + sp.vx * k * 0.6, sp.y + sp.vy * k * 0.6 + 20 * k * k, 1.6 * (1 - k) + 0.2, (1 - k) * 0.9);
      if (k >= 1) sparks = [];
    }
    if (!run.fired.tag && t >= E.burst) { showTag(ph, run.manual); run.fired.tag = true; }
    ctx.restore();
  }

  function play() {
    if (running) return;
    running = true;
    if (pausedAt && run) run.t0 += performance.now() - pausedAt;
    pausedAt = 0;
    raf = requestAnimationFrame(frame);
  }
  function pause() { if (!running) return; running = false; cancelAnimationFrame(raf); pausedAt = performance.now(); }

  // Reduced motion: the organized end state as a still, and your key types the next sentence.
  let stillShown = false, stillNext = 0;
  function still() {
    stillShown = true;
    pause(); run = null; ctx.clearRect(0, 0, W, H); setBars(0, 0); pill.classList.remove("pulse");
    field.innerHTML = PHRASES.map((p, i) => `<div class="line"><span class="t">${p.out}</span>${i === PHRASES.length - 1 ? '<span class="caret" id="caret"></span>' : ""}</div>`).join("");
    showTag(PHRASES[0], false);
  }

  const reduce = matchMedia("(prefers-reduced-motion: reduce)");
  let visible = true;
  function sync() {
    if (reduce.matches) { if (!stillShown) still(); return; }
    if (!run) { stillShown = false; clearField(false); pausedAt = 0; startPhrase(0, performance.now(), false); }
    if (visible && !document.hidden) play(); else pause();
  }

  // Your key takes over: hold it and Murmur listens; let go and the words land.
  function hold() {
    pill.classList.add("held");
    if (reduce.matches) return;
    const i = run && !run.fired.burst ? run.i : ((run ? run.i : -1) + 1) % PHRASES.length;
    startPhrase(i, performance.now(), true);
    run.held = true;
    play();
  }
  function release(cancel) {
    if (!pill.classList.contains("held")) return;
    pill.classList.remove("held");
    if (reduce.matches) {
      if (cancel) return;
      const ph = PHRASES[stillNext++ % PHRASES.length];
      if (lines().filter(hasText).length >= PHRASES.length) clearField(false);
      tidy();
      const l = newLine(), s = document.createElement("span");
      s.className = "t"; s.textContent = ph.out; l.insertBefore(s, $("caret"));
      showTag(ph, true);
      return;
    }
    if (!run || !run.held) return;
    run.held = false;
    const now = performance.now();
    if (cancel) { startPhrase(run.i, now, false); return; }
    if (now - run.t0 < run.T.mess - 400) run.t0 = now - (run.T.mess - 400);
  }
  addEventListener("keydown", (e) => {
    if (KEY.codes.includes(e.code)) { if (!e.repeat) hold(); return; }
    // any other key while it's held is a shortcut, not a dictation
    if (pill.classList.contains("held") && !(e.target === pill && (e.key === " " || e.key === "Enter"))) release(true);
  });
  addEventListener("keyup", (e) => { if (KEY.codes.includes(e.code)) release(false); });
  addEventListener("blur", () => release(true));
  pill.addEventListener("pointerdown", (e) => { pill.setPointerCapture(e.pointerId); hold(); });
  pill.addEventListener("pointerup", () => release(false));
  pill.addEventListener("pointercancel", () => release(true));
  pill.addEventListener("keydown", (e) => { if ((e.key === " " || e.key === "Enter") && !e.repeat) { e.preventDefault(); hold(); } });
  pill.addEventListener("keyup", (e) => { if (e.key === " " || e.key === "Enter") { e.preventDefault(); release(false); } });
  pill.addEventListener("click", (e) => e.preventDefault());

  new IntersectionObserver(([e]) => { visible = e.isIntersecting; sync(); }).observe(stage);
  document.addEventListener("visibilitychange", sync);
  reduce.addEventListener?.("change", () => { run = null; stillShown = false; sync(); });
  addEventListener("resize", resize);

  resize(); sync();
  document.fonts?.ready.then(resize);
})();
