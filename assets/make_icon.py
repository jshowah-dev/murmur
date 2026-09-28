"""Generate assets/murmur.ico: a white microphone on a teal disc, with two sound arcs.
Run once (python assets/make_icon.py); the .ico is committed and embedded by build.rs.

`python assets/make_icon.py tray` instead writes tray-live.ico and tray-paused.ico: the same
disc with a bolder mic and no arcs or grille, which stay legible at 16 px; grey when paused."""
from PIL import Image, ImageDraw
from pathlib import Path
import sys

S = 1024  # draw large, downsample for crisp edges


def tray_icon(bg):
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    d.ellipse((0, 0, S - 1, S - 1), fill=bg)
    cx, cap_w, cap_top, cap_bot = S * 0.5, S * 0.30, S * 0.14, S * 0.56
    d.rounded_rectangle((cx - cap_w / 2, cap_top, cx + cap_w / 2, cap_bot), radius=cap_w / 2, fill=(255, 255, 255))
    lw = int(S * 0.09)
    d.arc((cx - cap_w * 0.95, cap_top + cap_w * 0.9, cx + cap_w * 0.95, cap_bot + cap_w * 0.55), start=0, end=180, fill=(255, 255, 255), width=lw)
    stem_bot = S * 0.84
    d.line((cx, cap_bot + cap_w * 0.55, cx, stem_bot), fill=(255, 255, 255), width=lw)
    d.line((cx - cap_w * 0.55, stem_bot, cx + cap_w * 0.55, stem_bot), fill=(255, 255, 255), width=lw)
    return img


if sys.argv[1:] == ["tray"]:
    # 16 px at 100% scaling up to 32 px at 200%; the tray picks the size for the display's DPI
    sizes = [(s, s) for s in (16, 20, 24, 32, 40, 48)]
    for name, bg in (("tray-live.ico", (23, 120, 128)), ("tray-paused.ico", (130, 130, 130))):
        out = Path(__file__).with_name(name)
        tray_icon(bg).save(out, format="ICO", sizes=sizes)
        print("wrote", out)
    sys.exit()
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
