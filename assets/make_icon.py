"""Generate assets/murmur.ico: a white microphone on a teal disc, with two sound arcs.
Run once (python assets/make_icon.py); the .ico is committed and embedded by build.rs."""
from PIL import Image, ImageDraw
from pathlib import Path

S = 1024  # draw large, downsample for crisp edges
img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
d = ImageDraw.Draw(img)

BG = (23, 120, 128)      # teal disc
FG = (255, 255, 255)
ACCENT = (255, 214, 102)  # warm arcs

d.ellipse((0, 0, S - 1, S - 1), fill=BG)

# mic capsule
cx = S * 0.44
cap_w, cap_top, cap_bot = S * 0.20, S * 0.20, S * 0.56
d.rounded_rectangle((cx - cap_w / 2, cap_top, cx + cap_w / 2, cap_bot), radius=cap_w / 2, fill=FG)
# grille lines on the capsule
for y in (0.32, 0.40, 0.48):
    d.line((cx - cap_w * 0.28, S * y, cx + cap_w * 0.28, S * y), fill=BG, width=int(S * 0.018))

# cradle arc
lw = int(S * 0.055)
d.arc((cx - cap_w * 0.95, cap_top + cap_w * 0.9, cx + cap_w * 0.95, cap_bot + cap_w * 0.55), start=0, end=180, fill=FG, width=lw)
# stem + base
stem_bot = S * 0.78
d.line((cx, cap_bot + cap_w * 0.55, cx, stem_bot), fill=FG, width=lw)
d.line((cx - cap_w * 0.55, stem_bot, cx + cap_w * 0.55, stem_bot), fill=FG, width=lw)

# sound arcs to the right of the mic
ax, ay = cx + cap_w * 0.9, S * 0.38
for r in (0.10, 0.19):
    box = (ax - S * r, ay - S * r, ax + S * r, ay + S * r)
    d.arc(box, start=-40, end=40, fill=ACCENT, width=lw)

out = Path(__file__).with_name("murmur.ico")
sizes = [16, 24, 32, 48, 64, 128, 256]
img.save(out, format="ICO", sizes=[(s, s) for s in sizes])
img.resize((256, 256), Image.LANCZOS).save(out.with_suffix(".png"))
print("wrote", out)
