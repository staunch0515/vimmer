#!/usr/bin/env python3
"""Draw the fictional ramen shop's sample pictures locally (PIL only, no downloads).

Writes into remotion-renderer/public/assets/merchant-demo/:
  ramen.png       1080x1350 portrait "photo" of a bowl (the product)
  shop.png        1600x1000 landscape shop front (tests the landscape rule)
  logo.png        600x600 transparent round logo
  qr-sample.png   600x600 QR-looking placeholder (NOT a scannable code)
  map-pin.svg     the template's location icon (hand-written SVG path)

Everything is drawn from shapes; the only font is the system Hiragino (macOS).
Usage: python3 scripts/templates/merchant/draw_sample_assets.py
"""
import math
import random
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageFont

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / "remotion-renderer/public/assets/merchant-demo"
FONT_BOLD = "/System/Library/Fonts/ヒラギノ角ゴシック W6.ttc"
FONT_HEAVY = "/System/Library/Fonts/ヒラギノ角ゴシック W8.ttc"


def font(path, size):
    try:
        return ImageFont.truetype(path, size)
    except OSError:
        return ImageFont.load_default()


def vgrad(w, h, top, bottom):
    img = Image.new("RGB", (w, h))
    d = ImageDraw.Draw(img)
    for y in range(h):
        t = y / max(1, h - 1)
        d.line([(0, y), (w, y)], fill=tuple(int(a + (b - a) * t) for a, b in zip(top, bottom)))
    return img


def ramen():
    w, h = 1080, 1350
    img = vgrad(w, h, (58, 36, 24), (24, 14, 10))
    d = ImageDraw.Draw(img)
    rnd = random.Random(7)
    for _ in range(60):  # wood grain
        y = rnd.randint(0, h)
        d.arc([-200, y - 400, w + 200, y + 400], 200, 340, fill=(70, 44, 30), width=2)
    cx, cy, r = w // 2, int(h * 0.55), 430
    sh = Image.new("L", (w, h), 0)
    ImageDraw.Draw(sh).ellipse([cx - r - 10, cy - r + 40, cx + r + 30, cy + r + 70], fill=170)
    img.paste((0, 0, 0), mask=sh.filter(ImageFilter.GaussianBlur(30)))
    d.ellipse([cx - r, cy - r, cx + r, cy + r], fill=(30, 30, 34))  # bowl rim
    d.ellipse([cx - r + 14, cy - r + 14, cx + r - 14, cy + r - 14], fill=(170, 40, 34))
    for i in range(0, 360, 30):  # rim pattern
        a = math.radians(i)
        x, y = cx + (r - 7) * math.cos(a), cy + (r - 7) * math.sin(a)
        d.ellipse([x - 5, y - 5, x + 5, y + 5], fill=(240, 200, 120))
    br = r - 40
    broth = Image.new("RGB", (2 * br, 2 * br))
    bd = ImageDraw.Draw(broth)
    for k in range(br, 0, -2):
        t = k / br
        bd.ellipse([br - k, br - k, br + k, br + k],
                   fill=(int(236 - 40 * t), int(178 - 50 * t), int(96 - 40 * t)))
    m = Image.new("L", (2 * br, 2 * br), 0)
    ImageDraw.Draw(m).ellipse([0, 0, 2 * br, 2 * br], fill=255)
    img.paste(broth, (cx - br, cy - br), m)
    for k in range(26):  # noodles
        y0 = cy - 60 + k * 9
        d.arc([cx - 300 + (k % 3) * 20, y0 - 60, cx + 120, y0 + 90], 200, 340,
              fill=(250, 226, 150), width=7)
    for (dx, dy) in [(-210, -170), (-60, -230)]:  # chashu
        d.ellipse([cx + dx - 110, cy + dy - 80, cx + dx + 110, cy + dy + 80], fill=(150, 80, 52))
        d.ellipse([cx + dx - 85, cy + dy - 58, cx + dx + 85, cy + dy + 58], fill=(214, 150, 112))
        d.ellipse([cx + dx - 40, cy + dy - 25, cx + dx + 40, cy + dy + 25], fill=(240, 196, 160))
    for ex in (150, 255):  # ajitama halves
        d.ellipse([cx + ex - 75, cy - 30 - 95, cx + ex + 75, cy - 30 + 95], fill=(250, 246, 232))
        d.ellipse([cx + ex - 42, cy - 30 - 48, cx + ex + 42, cy - 30 + 48], fill=(244, 150, 30))
    d.polygon([(cx + 170, cy + 120), (cx + 330, cy + 40), (cx + 360, cy + 260), (cx + 210, cy + 320)],
              fill=(22, 40, 26))  # nori
    for _ in range(70):  # negi
        a, rr = rnd.uniform(0, 6.28), rnd.uniform(0, 150)
        x, y = cx - 120 + rr * math.cos(a), cy + 160 + rr * 0.6 * math.sin(a)
        d.ellipse([x - 9, y - 7, x + 9, y + 7], fill=(110, 190, 70), outline=(200, 240, 170), width=2)
    steam = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    sd = ImageDraw.Draw(steam)
    for s in range(3):
        x0 = cx - 200 + s * 200
        pts = [(x0 + 40 * math.sin(t / 30 + s), cy - r - t) for t in range(0, 380, 6)]
        sd.line(pts, fill=(255, 255, 255, 90), width=26)
    img = Image.alpha_composite(img.convert("RGBA"), steam.filter(ImageFilter.GaussianBlur(14)))
    img.convert("RGB").save(OUT / "ramen.png", optimize=True)


def shop():
    w, h = 1600, 1000
    img = vgrad(w, h, (36, 46, 84), (14, 16, 30))  # dusk sky
    d = ImageDraw.Draw(img)
    d.rectangle([0, 760, w, h], fill=(48, 46, 50))  # street
    d.rectangle([220, 180, 1380, 780], fill=(92, 62, 40))  # facade
    for x in range(220, 1380, 36):
        d.line([(x, 180), (x, 780)], fill=(78, 52, 34), width=4)
    d.rectangle([180, 140, 1420, 220], fill=(24, 22, 24))  # roof
    d.rectangle([520, 250, 1080, 360], fill=(26, 24, 26))  # sign board
    d.text((800, 305), "麺屋さくら", font=font(FONT_HEAVY, 84), fill=(250, 236, 210), anchor="mm")
    d.rectangle([420, 420, 1180, 780], fill=(250, 214, 140))  # lit doorway
    for i in range(5):  # noren
        x0 = 440 + i * 146
        d.rectangle([x0, 400, x0 + 136, 600], fill=(30, 50, 110))
    d.text((800, 500), "ら ー め ん", font=font(FONT_BOLD, 70), fill=(250, 250, 250), anchor="mm")
    glow = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    gd = ImageDraw.Draw(glow)
    for lx in (330, 1270):  # lanterns
        gd.ellipse([lx - 110, 370, lx + 110, 610], fill=(255, 120, 60, 120))
    img = Image.alpha_composite(img.convert("RGBA"), glow.filter(ImageFilter.GaussianBlur(40)))
    d = ImageDraw.Draw(img)
    for lx in (330, 1270):
        d.ellipse([lx - 55, 410, lx + 55, 570], fill=(226, 54, 40))
        d.text((lx, 490), "麺", font=font(FONT_HEAVY, 60), fill=(255, 240, 220), anchor="mm")
    img.convert("RGB").save(OUT / "shop.png", optimize=True)


def logo():
    s = 600
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    d.ellipse([10, 10, s - 10, s - 10], fill=(200, 38, 38))
    d.ellipse([40, 40, s - 40, s - 40], outline=(255, 236, 200), width=10)
    for i in range(5):  # sakura petals
        a = math.radians(-90 + i * 72)
        x, y = s / 2 + 70 * math.cos(a), 190 + 70 * math.sin(a)
        d.ellipse([x - 42, y - 42, x + 42, y + 42], fill=(255, 196, 214))
    d.ellipse([s / 2 - 26, 164, s / 2 + 26, 216], fill=(255, 236, 200))
    d.text((s / 2, 400), "麺屋", font=font(FONT_HEAVY, 118), fill=(255, 248, 236), anchor="mm")
    img.save(OUT / "logo.png", optimize=True)


def qr_placeholder():
    n, cell, quiet = 25, 20, 50
    s = n * cell + 2 * quiet
    img = Image.new("RGB", (s, s), "white")
    d = ImageDraw.Draw(img)
    rnd = random.Random(42)

    def finder(cx, cy):
        x, y = quiet + cx * cell, quiet + cy * cell
        d.rectangle([x, y, x + 7 * cell - 1, y + 7 * cell - 1], fill="black")
        d.rectangle([x + cell, y + cell, x + 6 * cell - 1, y + 6 * cell - 1], fill="white")
        d.rectangle([x + 2 * cell, y + 2 * cell, x + 5 * cell - 1, y + 5 * cell - 1], fill="black")

    for yy in range(n):
        for xx in range(n):
            in_finder = (xx < 8 and yy < 8) or (xx > n - 9 and yy < 8) or (xx < 8 and yy > n - 9)
            if not in_finder and rnd.random() < 0.48:
                x, y = quiet + xx * cell, quiet + yy * cell
                d.rectangle([x, y, x + cell - 1, y + cell - 1], fill="black")
    for fx, fy in [(0, 0), (n - 7, 0), (0, n - 7)]:
        finder(fx, fy)
    img.save(OUT / "qr-sample.png", optimize=True)


PIN_SVG = (
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">'
    '<path d="M12 1.5c-4.4 0-8 3.4-8 7.8 0 5.6 6.6 12.2 7.3 12.9a1 1 0 0 0 1.4 0'
    'c.7-.7 7.3-7.3 7.3-12.9 0-4.4-3.6-7.8-8-7.8zm0 11a3.2 3.2 0 1 1 0-6.4 3.2 3.2 0 0 1 0 6.4z"/>'
    "</svg>\n"
)


if __name__ == "__main__":
    OUT.mkdir(parents=True, exist_ok=True)
    ramen(); shop(); logo(); qr_placeholder()
    (OUT / "map-pin.svg").write_text(PIN_SVG)
    for p in sorted(OUT.iterdir()):
        print(p.relative_to(ROOT), p.stat().st_size)
