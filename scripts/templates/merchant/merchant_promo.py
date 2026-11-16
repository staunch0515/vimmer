#!/usr/bin/env python3
"""Prototype: merchant slots JSON -> Vimmer timeline JSON ("merchant_promo_v1").

15 s, 1080x1920, five scenes: hook -> product -> offer -> urgency -> shop + CTA.
The merchant system fills only the slots (see sample-slots.json); every position, size,
color, motion, sound and timing below is locked by the template.

Robustness ("0 废片"): every slot is checked before anything is written. Text is fitted
with a size ladder (shrink, then wrap to the slot's line limit); text that still does not
fit, an expired deadline, a bad price or a missing file is an error that names the slot,
never a silently broken video. Missing optional slots switch to a fixed fallback layout.

Pure Python 3 standard library (PIL is used only if present, to read photo sizes).
No network, no paid API: narration lines are built from the slots by rules.

Usage:
  python3 scripts/templates/merchant/merchant_promo.py SLOTS.json -o TIMELINE.json \
      [--public-dir remotion-renderer/public]
Exit codes: 0 ok (warnings on stderr), 1 slot errors (listed), 2 unreadable input.
"""
import argparse
import datetime as dt
import json
import re
import struct
import sys
from pathlib import Path

W, H = 1080, 1920
# Safe zones (fractions). Bottom 20 % is left to platform UI (TikTok/Reels/Douyin buttons
# and captions); 6 % side margins; the brand logo sits top-left inside 0.05..0.11.
X0, X1 = 0.06, 0.94
TOP, BOTTOM = 0.07, 0.80
COL_W = X1 - X0

# Locked timing (ms). Sum = 15 000. Narration that runs longer stretches its scene in the
# render, so the narration budget below keeps every line inside its scene.
DUR = {"hook": 3000, "product": 2800, "offer": 3600, "urgency": 2800, "shop": 2800}
# vimmer check estimates Kokoro ja at ~200 ms per character plus a lead-in; keep 0.6 s.
NARR_CHARS = {k: int((v - 600) / 200) for k, v in DUR.items()}

LIMITS = {  # slot: (max display units, sizes ladder as fraction of frame height, max lines)
    "hook": (11, [0.056, 0.050, 0.044], 2),
    "shop.name": (12, [0.050, 0.042], 1),
    "offer.title": (10, [0.056, 0.048, 0.040], 2),
    "offer.title_sub": (24, [0.030, 0.026], 1),
    "offer.badge": (7, [0.036, 0.032, 0.028], 1),
    "offer.coupon_code": (14, [0.058, 0.048, 0.040], 1),
    "shop.station": (16, [0.034, 0.030], 1),
    "shop.address": (32, [0.024, 0.021], 2),
    "cta": (14, [0.036, 0.032, 0.028], 2),
    "until": (12, [0.060, 0.052, 0.046], 1),
}
WEEKDAY_JA = "月火水木金土日"
WEEKDAY_ZH = "一二三四五六日"
TEXT = {
    "ja": {"recommend": "おすすめ", "coupon": "期間限定クーポン", "regular": "通常価格",
           "limited": "期間限定", "code": "クーポンコード", "scan": "QRを読み取ってね",
           "off": "{p}%OFF", "until": "{m}/{d}({w})まで", "today": "本日まで！",
           "tomorrow": "明日まで！", "left_pre": "あと", "left_suf": "日",
           "generic": {"hook": "話題のお店！", "product": "今日のおすすめ！", "offer": "今だけお得！",
                       "urgency": "お急ぎください！", "shop": "お待ちしてます！"}},
    "zh": {"recommend": "推荐", "coupon": "限时优惠券", "regular": "原价", "limited": "限时",
           "code": "优惠码", "scan": "扫码领优惠", "off": "{z}折", "until": "{m}/{d}(周{w})截止",
           "today": "今天截止！", "tomorrow": "明天截止！", "left_pre": "还剩", "left_suf": "天",
           "generic": {"hook": "人气好店！", "product": "今日推荐！", "offer": "限时特惠！",
                       "urgency": "抓紧时间！", "shop": "欢迎光临！"}},
}


class SlotError(Exception):
    pass


# ---------- text measuring (same per-class widths as vimmer-check's estimate) ----------

def char_em(c):
    o = ord(c)
    if c == " ":
        return 0.28
    if o >= 0x2E80 or 0xFF00 <= o <= 0xFFEF:
        return 1.0
    if c.isdigit():
        return 0.62
    if c.isupper():
        return 0.68
    if c.isalpha():
        return 0.55
    return 0.28 if c in ",.:;!'|il-()" else 0.6


# Bold Hiragino in Chrome measured ~10 % wider than the Inter-based estimate in the first render
# (pills and an 8-character hook wrapped); fit and box widths divide by this.
SAFETY = 0.88


def chip_w(text, fs, pad):
    """Width (frame fraction) of a text box whose background hugs one line of text."""
    return min(COL_W, (text_px(text, fs) / SAFETY + 2 * pad * fs * H) / W + 0.01)


def text_px(text, fs):
    return sum(char_em(c) for c in text) * fs * H


def wrap_lines(text, fs, width_frac):
    avail, lines = width_frac * W, 0
    for para in text.split("\n"):
        cur, n = 0.0, 1
        for c in para:
            w = char_em(c) * fs * H
            if cur + w > avail and cur > 0:
                n, cur = n + 1, 0.0
            cur += w
        lines += n
    return lines


def fit(slot, text, width_frac, errors, warnings, pad=0.0):
    """Pick the largest ladder size whose wrapped text stays within the slot's lines."""
    max_units, ladder, max_lines = LIMITS[slot]
    units = sum(char_em(c) for c in text.replace("\n", ""))
    if units > max_units * 1.0001:
        errors.append(f"{slot}: too long ({units:.1f} > {max_units} full-width characters): {text!r}")
        return ladder[-1]
    for i, fs in enumerate(ladder):
        avail = width_frac - 2 * pad * fs * H / W
        if wrap_lines(text, fs / SAFETY, avail) <= max_lines:
            if i:
                warnings.append(f"{slot}: shrunk to {fs} to fit {max_lines} line(s)")
            return fs
    errors.append(f"{slot}: does not fit {max_lines} line(s) even at {ladder[-1]}: {text!r}")
    return ladder[-1]


# ---------- numbers ----------

def kanji_number(n):
    """Integer -> Japanese reading in kanji (1200 -> 千二百), so TTS never guesses."""
    digits = "〇一二三四五六七八九"
    if n == 0:
        return "ゼロ"
    out = ""
    for unit_val, unit in ((10**8, "億"), (10**4, "万")):
        if n >= unit_val:
            out += kanji_number(n // unit_val) + unit
            n %= unit_val
    for unit_val, unit in ((1000, "千"), (100, "百"), (10, "十")):
        q, n = divmod(n, unit_val)
        if q:
            out += ("" if q == 1 else digits[q]) + unit
    return out + (digits[n] if n else "")


def spoken_digits(text):
    return re.sub(r"\d+", lambda m: kanji_number(int(m.group())), text)


def yen(n):
    return f"¥{n:,}"


# ---------- files ----------

def image_size(path):
    try:
        from PIL import Image  # optional
        with Image.open(path) as im:
            return im.size
    except ImportError:
        pass
    with open(path, "rb") as f:  # PNG header fallback
        head = f.read(24)
    if head[:8] == b"\x89PNG\r\n\x1a\n":
        return struct.unpack(">II", head[16:24])
    return None


def check_asset(public, rel, slot, errors, kinds=(".png", ".jpg", ".jpeg")):
    if not rel:
        return None
    if rel.startswith("/") or ".." in rel or "://" in rel:
        errors.append(f"{slot}: path must be relative to public/: {rel!r}")
        return None
    if not rel.lower().endswith(kinds):
        errors.append(f"{slot}: must be one of {', '.join(kinds)}: {rel!r}")
        return None
    p = public / rel
    if not p.is_file():
        errors.append(f"{slot}: file not found under public/: {rel!r}")
        return None
    return image_size(p)


# ---------- layer helpers ----------

def text_layer(id, text, y, fs, *, x=X0, w=COL_W, color=None, weight=700, align="center",
               start=0, enter=None, **extra):
    layer = {"id": id, "kind": "text", "text": text, "x": x, "y": y, "width": w,
             "font_size": fs, "weight": weight, "align": align, "start_ms": start}
    if color:
        layer["color"] = color
    if enter:
        layer["enter"] = enter
    layer.update(extra)
    return layer


def rect(id, x, y, w, h, fill, z=-10, **extra):
    return {"id": id, "kind": "shape", "shape": "rect", "x": x, "y": y, "width": w,
            "height": h, "fill": fill, "z": z, **extra}


def photo_card(id, src, size, x, y, w, h, warnings, slot, start=0):
    """A rounded photo card. Cover-crop when the photo's shape is close to the card's;
    otherwise contain it on a dark mat so nothing important is cut off."""
    box_ar = (w * W) / (h * H)
    fit_mode = "cover"
    if size:
        ar = size[0] / size[1]
        kept = min(ar, box_ar) / max(ar, box_ar)  # share of the photo left after cropping
        if kept < 0.55:
            fit_mode = "contain"
            warnings.append(f"{slot}: {src} is {size[0]}x{size[1]}; cropping would keep "
                            f"{kept:.0%}, so it is shown whole on a mat")
        if size[0] < w * W / 2 or size[1] < h * H / 2:
            warnings.append(f"{slot}: {src} is small ({size[0]}x{size[1]}) and will look soft")
    layers = []
    if fit_mode == "contain":
        layers.append(rect(f"{id}-mat", x, y, w, h, "#0b0806", z=-6, corner_radius=0.02,
                           start_ms=start))
    layers.append({"id": id, "kind": "image", "src": src, "fit": fit_mode, "x": x, "y": y,
                   "width": w, "height": h, "corner_radius": 0.02, "z": -5, "start_ms": start,
                   "enter": {"kind": "fade", "duration_ms": 300},
                   "keyframes": [{"at_ms": 0, "scale": 1.0, "ease": "linear"},
                                 {"at_ms": 2800, "scale": 1.05}]})
    return layers


def narration(slots, key, defaults, voice, errors, lang="ja"):
    """The merchant's own line, else the first auto line (longest first) that fits the scene."""
    custom = ((slots.get("narration") or {}).get(key) or "").strip()
    spoken = spoken_digits if lang == "ja" else (lambda t: t)  # zh TTS reads digits itself
    budget = NARR_CHARS[key]
    count = lambda t: len(re.sub(r"[\s、。，！!？?・]", "", t))
    if custom:
        line = spoken(custom)
        if count(line) > budget:
            errors.append(f"narration.{key}: {count(line)} characters, the {DUR[key]} ms scene "
                          f"fits about {budget}: {line!r}")
        return {"voice": voice, "text": line}
    defaults = ([defaults] if isinstance(defaults, str) else defaults) + [TEXT[lang]["generic"][key]]
    for d in defaults:
        line = spoken(d)
        if count(line) <= budget:
            return {"voice": voice, "text": line}
    errors.append(f"narration.{key}: no automatic line fits {budget} characters; give one")
    return {"voice": voice, "text": spoken(defaults[-1])}


# ---------- the template ----------

def build(slots, public):
    errors, warnings = [], []
    lang = slots.get("lang", "ja")
    if lang not in TEXT:
        raise SlotError([f"lang: must be ja or zh, got {lang!r}"])
    T = TEXT[lang]
    voice = slots.get("voice") or ("jf_alpha" if lang == "ja" else "zf_001")
    shop, offer, media = slots.get("shop") or {}, slots.get("offer") or {}, slots.get("media") or {}
    b = slots.get("brand") or {}
    primary, accent = b.get("primary", "#c8102e"), b.get("accent", "#ffd23f")
    for k, v in (("brand.primary", primary), ("brand.accent", accent)):
        if not re.fullmatch(r"#[0-9a-fA-F]{6}", v):
            errors.append(f"{k}: must be #rrggbb, got {v!r}")
    ink, bg, muted = "#fffaf2", "#1a1210", "#e8d9c8"

    def req(obj, key, slot):
        v = (obj.get(key) or "").strip() if isinstance(obj.get(key), str) else obj.get(key)
        if v in (None, ""):
            errors.append(f"{slot}: required")
        return v

    name = req(shop, "name", "shop.name") or ""
    title = req(offer, "title", "offer.title") or ""
    orig, sale = offer.get("original_price"), offer.get("sale_price")
    if not (isinstance(orig, int) and isinstance(sale, int) and 0 < sale < orig <= 9_999_999):
        errors.append(f"offer: need integer prices with 0 < sale_price < original_price, got {orig!r} / {sale!r}")
        orig, sale = 2, 1
    hook = (slots.get("hook") or "").strip() or (f"{name}の{T['recommend']}" if lang == "ja" else f"{name}·{T['recommend']}")
    cta = (slots.get("cta") or "").strip() or ("このクーポンを見せてね" if lang == "ja" else "到店出示优惠券")
    station = (shop.get("station") or "").strip()
    address = (shop.get("address") or "").strip()
    code = (offer.get("coupon_code") or "").strip()
    if code and not re.fullmatch(r"[A-Za-z0-9-]+", code):
        errors.append(f"offer.coupon_code: letters, digits and - only: {code!r}")
    currency = offer.get("currency", "JPY")
    if currency not in ("JPY", "CNY"):
        errors.append(f"offer.currency: JPY or CNY, got {currency!r}")
    unit_zh = "日元" if currency == "JPY" else "元"
    pct = round((1 - sale / orig) * 100)
    badge = (offer.get("badge") or "").strip() or (
        T["off"].format(p=pct) if lang == "ja" else T["off"].format(z=f"{sale / orig * 10:.1f}".rstrip("0").rstrip(".")))

    # deadline -> days left, computed for the render date (re-generate daily for a live countdown)
    try:
        today = dt.date.fromisoformat(slots.get("render_date") or dt.date.today().isoformat())
        deadline = dt.date.fromisoformat(req(offer, "deadline", "offer.deadline") or "")
        days = (deadline - today).days
        if days < 0:
            errors.append(f"offer.deadline: {deadline} is before the render date {today}; the offer has expired")
    except ValueError as e:
        errors.append(f"offer.deadline / render_date: {e}")
        deadline, days = dt.date.today(), 0

    photos = [p for p in (media.get("photos") or []) if p][:3]
    sizes = [check_asset(public, p, f"media.photos[{i}]", errors) for i, p in enumerate(photos)]
    if media.get("clip"):
        warnings.append("media.clip: not used by this prototype (planned: video layer in the hook)")
    logo = shop.get("logo")
    logo_size = check_asset(public, logo, "shop.logo", errors, (".png", ".jpg", ".jpeg")) if logo else None
    qr = offer.get("qr")
    check_asset(public, qr, "offer.qr", errors) if qr else None

    pads = {"cta": 0.45, "offer.coupon_code": 0.5}
    fs = {s: fit(s, t, wf, errors, warnings, pads.get(s, 0.0)) for s, t, wf in [
        ("hook", hook, COL_W), ("shop.name", name, COL_W), ("offer.title", title, COL_W),
        ("offer.badge", badge, 0.23), ("cta", cta, (COL_W - 0.34) if qr else COL_W),
        ("shop.station", station or "-", 0.78), ("shop.address", address or "-", COL_W),
        ("offer.coupon_code", code or "-", 0.80),
        ("offer.title_sub", offer.get("title_sub") or "-", COL_W)]}

    hero = photos[0] if photos else None
    hero_size = sizes[0] if photos else None
    prod, prod_size = (photos[1], sizes[1]) if len(photos) >= 3 else (hero, hero_size)
    place, place_size = (photos[-1], sizes[-1]) if len(photos) >= 2 else (None, None)
    if not photos:
        warnings.append("media.photos: none; typographic layout (brand colors only)")

    pause = "、" if lang == "ja" else "，"
    scenes = []

    # 1. HOOK ---------------------------------------------------------------
    L = []
    if hero:
        hero_ar = hero_size[0] / hero_size[1] if hero_size else 0.56
        if hero_ar > 0.9:  # landscape/square hero: blur-free fallback = card on brand color
            L += [rect("bg", 0, 0, 1, 1, bg)] + photo_card("hero", hero, hero_size, X0, 0.12, COL_W, 0.34, warnings, "media.photos[0]")
        else:
            L.append({"id": "hero", "kind": "image", "src": hero, "fit": "cover", "x": 0, "y": 0,
                      "width": 1, "height": 1, "z": -10,
                      "keyframes": [{"at_ms": 0, "scale": 1.0, "ease": "linear"},
                                    {"at_ms": DUR["hook"], "scale": 1.12}]})
            # stepped scrim (no gradient fill yet): 6 bands from clear to 70 % black
            L += [rect(f"scrim-{i}", 0, 0.34 + 0.03 * i, 1, 0.66 - 0.03 * i, "#00000022", z=-4)
                  for i in range(6)]
    else:
        L += [rect("bg", 0, 0, 1, 1, primary), rect("band", 0, 0.46, 1, 0.30, "#00000040", z=-9)]
    chip_fs = 0.026
    L.append(text_layer("shop-chip", name, 0.53, chip_fs, x=X0, w=chip_w(name, chip_fs, 0.45), weight=800, color=ink,
                        background={"color": primary, "padding": 0.45, "radius": 0.3},
                        enter={"kind": "slide_right", "duration_ms": 350}))
    L.append(text_layer("hook", hook, 0.59, fs["hook"], weight=900, align="left", color=ink,
                        start=150, enter={"kind": "slide_up", "duration_ms": 450, "ease": "back_out"}))
    scenes.append({"id": "hook", "duration_ms": DUR["hook"], "text": None, "layers": L,
                   "narration": narration(slots, "hook", [f"{hook}{pause}{name}。", f"{hook}！", f"{name}。"], voice, errors, lang),
                   "sfx": [{"src": "assets/sfx/whoosh.wav", "at_ms": 0, "volume": 0.5}]})

    # 2. PRODUCT ------------------------------------------------------------
    L = [rect("bg", 0, 0, 1, 1, bg)]
    if prod:
        L += photo_card("photo", prod, prod_size, X0, 0.09, COL_W, 0.47, warnings, "media.photos")
        if prod == hero:  # same picture as the hook: show it closer
            L[-1]["keyframes"] = [{"at_ms": 0, "scale": 1.18, "ease": "linear"}, {"at_ms": 2800, "scale": 1.28}]
        ty = 0.60
    else:
        ty = 0.34
    L.append(text_layer("recommend", T["recommend"], ty, 0.026, color=bg, weight=800, x=0.5 - chip_w(T["recommend"], 0.026, 0.35) / 2, w=chip_w(T["recommend"], 0.026, 0.35),
                        background={"color": accent, "padding": 0.35, "radius": 0.5},
                        enter={"kind": "pop", "duration_ms": 350}))
    L.append(text_layer("title", title, ty + 0.045, fs["offer.title"], weight=900, color=ink,
                        start=200, enter={"kind": "slide_up", "duration_ms": 400}))
    lines = wrap_lines(title, fs["offer.title"], COL_W)
    if offer.get("title_sub"):
        L.append(text_layer("title-sub", offer["title_sub"].strip(),
                            ty + 0.06 + lines * fs["offer.title"] * 1.2, fs["offer.title_sub"],
                            weight=500, color=muted, start=450, enter={"kind": "fade", "duration_ms": 300}))
    scenes.append({"id": "product", "duration_ms": DUR["product"], "text": None, "layers": L,
                   "transition_in": {"kind": "slide_left", "duration_ms": 350},
                   "narration": narration(slots, "product",
                                          [f"おすすめは{pause}{title}。", f"{title}。"] if lang == "ja" else [f"推荐{title}。", f"{title}。"],
                                          voice, errors)})

    # 3. OFFER / PRICE ------------------------------------------------------
    L = [rect("bg", 0, 0, 1, 1, primary),
         {"id": "deco", "kind": "shape", "shape": "ellipse", "x": 0.55, "y": -0.08, "width": 0.9,
          "height": 0.5, "fill": "#ffffff14", "z": -9}]
    L.append(text_layer("label", T["coupon"], 0.13, 0.034, color=bg, weight=900, x=0.5 - chip_w(T["coupon"], 0.034, 0.4) / 2, w=chip_w(T["coupon"], 0.034, 0.4),
                        background={"color": accent, "padding": 0.4, "radius": 0.5},
                        enter={"kind": "pop", "duration_ms": 350}))
    if prod:
        L += photo_card("thumb", prod, prod_size, 0.22, 0.20, 0.56, 0.28, warnings, "media.photos")
    py = 0.51
    L.append(text_layer("regular", T["regular"], py, 0.026, color="#ffffff", weight=600))
    ofs = 0.05
    L.append(text_layer("orig", yen(orig), py + 0.035, ofs, color="#ffffffcc", weight=700))
    ow = (text_px(yen(orig), ofs) + 0.4 * ofs * H) / W
    L.append({"id": "strike", "kind": "shape", "shape": "line", "x": 0.5 - ow / 2,
              "y": py + 0.035 + ofs * 0.62, "width": ow, "height": 0, "stroke": accent,
              "stroke_width": 0.006, "start_ms": 600, "enter": {"kind": "slide_right", "duration_ms": 250}})
    L.append({"id": "price", "kind": "counter", "x": X0, "y": py + 0.10, "width": COL_W,
              "from": orig, "to": sale, "prefix": "¥", "duration_ms": 1300, "start_ms": 700,
              "ease": "ease_out", "font_size": 0.11, "weight": 900, "color": accent, "align": "center"})
    bx, by, bw = 0.70, 0.40, 0.25
    badge_layers = [
        {"id": "badge", "kind": "shape", "shape": "rect", "x": bx, "y": by, "width": bw,
         "height": bw * W / H, "corner_radius": bw * W / H / 2, "fill": accent, "rotation": -12, "z": 1, "start_ms": 1900,
         "enter": {"kind": "pop", "duration_ms": 450, "ease": "elastic_out"}},
        text_layer("badge-text", badge, by, fs["offer.badge"], x=bx, w=bw, color=bg, weight=900,
                   background={"color": accent, "padding": 0.1, "radius": 0.5},
                   height=bw * W / H, rotation=-12, z=2, start=1900,
                   enter={"kind": "pop", "duration_ms": 450, "ease": "elastic_out"})]
    L += badge_layers
    offer_line = (f"今なら{kanji_number(orig)}円が{pause}{kanji_number(sale)}円！" if lang == "ja"
                  else f"原价{orig}{unit_zh}，现在只要{sale}{unit_zh}！")
    scenes.append({"id": "offer", "duration_ms": DUR["offer"], "text": None, "layers": L,
                   "transition_in": {"kind": "slide_up", "duration_ms": 350},
                   "narration": narration(slots, "offer", [offer_line, f"今なら{kanji_number(sale)}円！" if lang == "ja" else f"只要{sale}{unit_zh}！"], voice, errors, lang),
                   "sfx": [{"src": "assets/sfx/ding.wav", "at_ms": 2000, "volume": 0.3},
                           {"src": "assets/sfx/pop.wav", "on": {"layer": "badge", "event": "enter"}, "volume": 0.7}]})

    # 4. URGENCY ------------------------------------------------------------
    L = [rect("bg", 0, 0, 1, 1, "#141010"), rect("stripe", 0, 0.115, 1, 0.006, accent, z=-9)]
    wd = (WEEKDAY_JA if lang == "ja" else WEEKDAY_ZH)[deadline.weekday()]
    until = T["until"].format(m=deadline.month, d=deadline.day, w=wd)
    L.append(text_layer("limited", T["limited"], 0.17, 0.032, color=ink, weight=800, x=0.5 - chip_w(T["limited"], 0.032, 0.4) / 2, w=chip_w(T["limited"], 0.032, 0.4),
                        background={"color": primary, "padding": 0.4, "radius": 0.4},
                        enter={"kind": "pop", "duration_ms": 300}))
    L.append(text_layer("until", until, 0.235, fit("until", until, COL_W, errors, warnings), color=ink, weight=900, start=150,
                        enter={"kind": "slide_up", "duration_ms": 350}))
    if days >= 2:
        L.append({"id": "days", "kind": "counter", "x": X0, "y": 0.33, "width": COL_W,
                  "from": min(days + 9, 99), "to": days, "prefix": T["left_pre"], "suffix": T["left_suf"],
                  "duration_ms": 1200, "start_ms": 300, "ease": "ease_out", "font_size": 0.10,
                  "weight": 900, "color": accent, "align": "center"})
        left_line = f"あと{kanji_number(days)}日。" if lang == "ja" else f"还剩{days}天。"
    else:
        L.append(text_layer("days", T["today"] if days == 0 else T["tomorrow"], 0.34, 0.09,
                            color=accent, weight=900, start=300, enter={"kind": "pop", "duration_ms": 400}))
        left_line = T["today"] if days == 0 else T["tomorrow"]
    if code:
        L.append(text_layer("code-label", T["code"], 0.50, 0.028, color=muted, weight=600, start=700,
                            enter={"kind": "fade", "duration_ms": 300}))
        cfs = fs["offer.coupon_code"]
        cw = chip_w(code, cfs, 0.5) + 0.02
        L.append(text_layer("code", code, 0.545, cfs, x=0.5 - cw / 2, w=cw, color=ink, weight=800, start=800,
                            background={"color": "#ffffff14", "padding": 0.5, "radius": 0.25,
                                        "border": {"color": accent, "width": 4}},
                            enter={"kind": "pop", "duration_ms": 350}))
    urg_line = (f"{kanji_number(deadline.month)}月{kanji_number(deadline.day)}日まで、{left_line}"
                if lang == "ja" else f"{deadline.month}月{deadline.day}日截止，{left_line}")
    scenes.append({"id": "urgency", "duration_ms": DUR["urgency"], "text": None, "layers": L,
                   "transition_in": {"kind": "slide_left", "duration_ms": 300},
                   "narration": narration(slots, "urgency", [urg_line, left_line], voice, errors, lang),
                   "sfx": [{"src": "assets/sfx/click.wav", "at_ms": 1500, "volume": 0.5}]})

    # 5. SHOP + CTA ---------------------------------------------------------
    L = [rect("bg", 0, 0, 1, 1, bg)]
    if place:
        L += photo_card("place", place, place_size, X0, 0.07, COL_W, 0.27, warnings, "media.photos[-1]")
    ly = 0.36 if place else 0.20
    if logo:
        L.append({"id": "logo", "kind": "image", "src": logo, "fit": "contain", "x": 0.5, "y": ly,
                  "width": 0.20, "height": 0.20 * W / H, "anchor": "center", "z": 1, "start_ms": 100,
                  "enter": {"kind": "pop", "duration_ms": 400, "ease": "back_out"}})
    ny = ly + (0.07 if logo else 0.0)
    L.append(text_layer("name", name, ny, fs["shop.name"], color=ink, weight=900, start=200,
                        enter={"kind": "slide_up", "duration_ms": 350}))
    sy = ny + fs["shop.name"] * 1.2 + 0.015
    if station:
        sfs = fs["shop.station"]
        tw = text_px(station, sfs) / W
        icon_w = sfs * H * 1.0 / W
        x_text = 0.5 - (tw + icon_w + 0.01) / 2 + icon_w + 0.01
        L.append({"id": "pin", "kind": "icon", "src": "assets/merchant-demo/map-pin.svg", "color": accent,
                  "x": x_text - icon_w - 0.01, "y": sy, "width": icon_w, "height": sfs, "start_ms": 350,
                  "enter": {"kind": "fade", "duration_ms": 300}})
        L.append(text_layer("station", station, sy, sfs, x=x_text, w=tw + 0.04, align="left",
                            color=ink, weight=700, start=350, enter={"kind": "fade", "duration_ms": 300}))
        sy += sfs * 1.2 + 0.01
    if address:
        L.append(text_layer("address", address, sy, fs["shop.address"], color=muted, weight=500,
                            start=450, enter={"kind": "fade", "duration_ms": 300}))
        sy += wrap_lines(address, fs["shop.address"], COL_W) * fs["shop.address"] * 1.2
    row = max(sy + 0.03, 0.60)
    qr_h = 0.30 * W / H
    if row + qr_h > BOTTOM:
        errors.append("shop: name/station/address leave no room for the QR row above the safe bottom")
    if qr:
        L.append(rect("qr-mat", X0, row, 0.30, qr_h, "#ffffff", z=0, corner_radius=0.01, start_ms=500,
                      enter={"kind": "fade", "duration_ms": 300}))
        L.append({"id": "qr", "kind": "image", "src": qr, "fit": "contain", "x": X0 + 0.015,
                  "y": row + 0.015 * W / H, "width": 0.27, "height": 0.27 * W / H, "z": 1,
                  "start_ms": 500, "enter": {"kind": "fade", "duration_ms": 300}})
        cx, cw = X0 + 0.34, COL_W - 0.34
    else:
        cx, cw = X0, COL_W
    L.append(text_layer("cta", cta, row + 0.02, fs["cta"], x=cx, w=cw, color=ink, weight=900, start=300,
                        background={"color": primary, "padding": 0.45, "radius": 0.35},
                        enter={"kind": "pop", "duration_ms": 400, "ease": "back_out"},
                        keyframes=[{"at_ms": 1000, "scale": 1.0}, {"at_ms": 1450, "scale": 1.06},
                                   {"at_ms": 1900, "scale": 1.0}, {"at_ms": 2350, "scale": 1.06}]))
    if qr:
        cta_lines = wrap_lines(cta, fs["cta"] / SAFETY, cw - 2 * 0.45 * fs["cta"] * H / W)
        L.append(text_layer("scan", T["scan"], row + 0.02 + cta_lines * fs["cta"] * 1.2 + 2 * 0.45 * fs["cta"] + 0.02, 0.022, x=cx, w=cw, color=muted, weight=500,
                            start=400, enter={"kind": "fade", "duration_ms": 300}))
    where = (station.replace(" ", pause) + "。") if station else ""
    scenes.append({"id": "shop", "duration_ms": DUR["shop"], "text": None, "layers": L,
                   "transition_in": {"kind": "fade", "duration_ms": 300},
                   "narration": narration(slots, "shop", [f"{where}{cta}！", f"{cta}！"], voice, errors, lang),
                   "sfx": [{"src": "assets/sfx/pop.wav", "on": {"layer": "cta", "event": "enter"}, "volume": 0.6}]})

    if errors:
        raise SlotError(errors)

    brand = {"colors": {"primary": primary, "accent": accent, "text": ink, "muted_text": muted,
                        "background": bg},
             "fonts": {"heading": {"family": "Hiragino Sans"}, "body": {"family": "Hiragino Sans"}},
             "text_style": {"outline": {"color": "#000000", "width": 0}, "shadow": "soft"}}
    if logo:
        brand["logo"] = {"src": logo, "corner": "top_left", "size": 0.05, "margin": 0.05,
                         "opacity": 0.95, "hide_on": ["shop"]}
    timeline = {"fps": 30, "resolution": {"width": W, "height": H}, "brand": brand,
                "music": {"pick": {"mood": "upbeat", "energy": 0.7}, "volume": 0.3,
                          "fade_in_ms": 300, "fade_out_ms": 900},
                "subtitles": [], "scenes": scenes}
    return timeline, warnings


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[1])
    ap.add_argument("slots")
    ap.add_argument("-o", "--output", required=True)
    ap.add_argument("--public-dir", default=str(Path(__file__).resolve().parents[3] / "remotion-renderer/public"))
    a = ap.parse_args()
    try:
        slots = json.loads(Path(a.slots).read_text(encoding="utf-8"))
    except (OSError, ValueError) as e:
        print(f"cannot read slots: {e}", file=sys.stderr)
        return 2
    if slots.get("template") != "merchant_promo_v1":
        print("template: must be merchant_promo_v1", file=sys.stderr)
        return 1
    try:
        timeline, warnings = build(slots, Path(a.public_dir))
    except SlotError as e:
        for msg in e.args[0]:
            print(f"error: {msg}", file=sys.stderr)
        return 1
    for w in warnings:
        print(f"warning: {w}", file=sys.stderr)
    Path(a.output).write_text(json.dumps(timeline, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"{len(timeline['scenes'])} scenes, {sum(s['duration_ms'] for s in timeline['scenes'])} ms -> {a.output}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
