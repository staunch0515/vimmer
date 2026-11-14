#!/usr/bin/env python3
"""Fits the narration-length model to calibration/measured.csv (stdlib only).

    python3 -I calibration/fit.py [calibration/measured.csv]

For each engine x language, finds non-negative per-feature costs (ms) that minimise the
squared *relative* error, prints them as Rust constants for `src/estimate.rs`, and prints the
error table: mean absolute % error and worst case, for the old estimators (storyboard and
`vimmer check`, re-implemented below) and the new model, in-sample and leave-one-out.

Features (must match `features()` in src/estimate.rs):
    han      CJK ideographs
    kana     hiragana / katakana (incl. the long mark)
    letters  ASCII letters
    words    runs of ASCII letters (and apostrophes) - a word in English, a Latin name in CJK
    number   spoken length of ASCII numbers (ja moras / zh syllables / en words), see number_units
    minor    , ; : 、 ， ； ： and dashes  (short pause)
    major    . ! ? 。 ！ ？ … and line breaks   (sentence pause; a run counts once)
    const    1 per clip (lead-in / tail silence)
"""

import csv
import sys
from pathlib import Path

FEATURES = ["han", "kana", "letters", "words", "number", "minor", "major", "const"]
# Which features each language's model may use (others are fixed at 0). Few points per
# language, so few features.
USED = {
    "zh": ["han", "letters", "words", "number", "minor", "major", "const"],
    "ja": ["han", "kana", "letters", "words", "number", "minor", "major", "const"],
    "en": ["letters", "words", "number", "minor", "major", "const"],
}
MINOR = set(",;:、，；：—–")
MAJOR = set(".!?。！？…\n")


def is_han(c):
    return "一" <= c <= "鿿" or "㐀" <= c <= "䶿" or c == "々"


def is_kana(c):
    return "ぁ" <= c <= "ゖ" or "ァ" <= c <= "ヺ" or c == "ー"


JA_DIGIT_MORAS = [2, 2, 1, 2, 2, 1, 2, 2, 2, 2]


def number_units(int_part, frac, year, lang):
    """Spoken length of one number; must match `number_units` in src/estimate.rs."""
    d = [int(c) for c in int_part]
    n = len(d)

    def single(x):
        return JA_DIGIT_MORAS[x] if lang == "ja" else 1

    def by_digit(ds):
        return sum(single(x) for x in ds)

    value = int(int_part) if n <= 16 else 0

    def words(v):
        return 0 if v == 0 else 1 if v < 20 else 1 + (v % 10 != 0)

    if n > 16 or (n > 1 and d[0] == 0) or (lang == "zh" and year and n == 4):
        units = by_digit(d)
    elif lang == "en":
        if 1100 <= value <= 1999 or 2010 <= value <= 2099:
            pair = lambda v: 1 if v == 0 else 2 if v < 10 else words(v)
            units = pair(value // 100) + pair(value % 100)
        elif value == 0:
            units = 1
        else:
            units, rest, scale = 0, value, 0
            while rest > 0:
                g = rest % 1000
                if g > 0:
                    units += (2 if g >= 100 else 0) + words(g % 100) + (scale > 0)
                rest //= 1000
                scale += 1
    elif value == 0:
        units = single(0)
    else:
        units, pending_zero, started, group_nonzero = 0, False, False, False
        ja = lang == "ja"
        for i, x in enumerate(d):
            r = n - 1 - i
            k, g = r % 4, r // 4
            if x == 0:
                pending_zero = started
            else:
                if lang == "zh" and pending_zero:
                    units += 1
                pending_zero, started, group_nonzero = False, True, True
                drop_one = x == 1 and ((k > 0) if ja else (k == 1 and i == 0))
                if not drop_one:
                    units += single(x)
                if k > 0:
                    units += 2 if ja else 1
            if k == 0:
                if g > 0 and group_nonzero:
                    units += 2 if ja else 1
                group_nonzero = False
    if frac:
        units += (2 if lang == "ja" else 1) + by_digit([int(c) for c in frac])
    return units


def features(text, lang):
    f = dict.fromkeys(FEATURES, 0)
    f["const"] = 1
    prev = ""
    in_word = False
    skip_to = 0
    for i, c in enumerate(text):
        if i < skip_to:
            prev = c
            continue
        nxt = text[i + 1] if i + 1 < len(text) else ""
        if c.isascii() and c.isalpha():
            f["letters"] += 1
            if not in_word:
                f["words"] += 1
            in_word = True
        elif c == "'" and in_word:
            pass
        else:
            in_word = False
            if c.isascii() and c.isdigit():
                isd = lambda j: j < len(text) and text[j].isascii() and text[j].isdigit()
                j, ip, fp = i, "", ""
                while isd(j):
                    ip += text[j]
                    j += 1
                    if j < len(text) and text[j] == "," and all(isd(j + k) for k in (1, 2, 3)) and not isd(j + 4):
                        j += 1
                if j < len(text) and text[j] == "." and isd(j + 1):
                    j += 1
                    while isd(j):
                        fp += text[j]
                        j += 1
                f["number"] += number_units(ip, fp, text[j : j + 1] == "年", lang)
                if lang == "en" and i > 0 and text[i - 1] == "$":
                    f["number"] += 1
                if text[j : j + 1] == "%":
                    f["number"] += {"en": 1, "zh": 3, "ja": 5}[lang]
                skip_to = j
            elif is_han(c):
                f["han"] += 1
            elif is_kana(c):
                f["kana"] += 1
            elif c in MAJOR:
                # 8,848.86 / 1.2: a dot between digits is read, not a pause.
                if c == "." and prev.isdigit() and nxt.isdigit():
                    pass
                elif prev not in MAJOR:
                    f["major"] += 1
            elif c in MINOR:
                if c == "," and prev.isdigit() and nxt.isdigit():
                    pass
                else:
                    f["minor"] += 1
        prev = c
    return f


# --- old estimators --------------------------------------------------------------------------
def cjk_units(text):
    units, run = 0, False
    for c in text:
        if c.isascii() and c.isalnum():
            if not run:
                units += 2
                run = True
        else:
            run = False
            if c.isalnum():
                units += 1
    return units


def words(text):
    return sum(1 for w in text.split() if any(ch.isalnum() for ch in w))


def old_storyboard(text, lang):
    """Old storyboard estimate minus its 600 ms padding (comparable with the file length)."""
    return {"en": 400 * words(text), "zh": 230 * cjk_units(text), "ja": 200 * cjk_units(text)}[lang]


def old_check(text, lang):
    """Old `vimmer check` speech estimate (without the 300 ms tail)."""
    return {"en": 250 * words(text), "zh": 230 * cjk_units(text), "ja": 165 * cjk_units(text)}[lang]


# --- fitting ---------------------------------------------------------------------------------
def solve(a, b):
    """Gaussian elimination with partial pivoting; a is n x n."""
    n = len(a)
    m = [row[:] + [b[i]] for i, row in enumerate(a)]
    for col in range(n):
        piv = max(range(col, n), key=lambda r: abs(m[r][col]))
        if abs(m[piv][col]) < 1e-12:
            raise ValueError("singular")
        m[col], m[piv] = m[piv], m[col]
        for r in range(n):
            if r != col:
                k = m[r][col] / m[col][col]
                for c in range(col, n + 1):
                    m[r][c] -= k * m[col][c]
    return [m[i][n] / m[i][i] for i in range(n)]


def fit(rows, used, ridge=1e-3):
    """Non-negative least squares on relative error: minimise sum(((x.w - y) / y)^2).
    Negative coefficients are dropped and the fit repeated (active-set NNLS, good enough
    for a handful of features). A light ridge (relative to each feature's scale)
    keeps rare features from blowing up."""
    active = list(used)
    while True:
        n = len(active)
        ata = [[0.0] * n for _ in range(n)]
        aty = [0.0] * n
        for f, y in rows:
            x = [f[k] / y for k in active]
            for i in range(n):
                aty[i] += x[i]
                for j in range(n):
                    ata[i][j] += x[i] * x[j]
        # Ridge relative to each feature's own scale: a light pull towards 0 for rare features.
        for i in range(n):
            ata[i][i] += ridge * (ata[i][i] or 1.0)
        try:
            w = solve(ata, aty)
        except ValueError:
            # A feature never occurs: drop the rarest.
            counts = {k: sum(f[k] for f, _ in rows) for k in active}
            active.remove(min(active, key=counts.get))
            continue
        neg = [k for k, v in zip(active, w) if v < 0]
        if not neg:
            coef = dict.fromkeys(FEATURES, 0.0)
            coef.update(zip(active, w))
            return coef
        active.remove(min(neg, key=lambda k: w[active.index(k)]))


def predict(coef, f):
    return sum(coef[k] * f[k] for k in FEATURES)


def errors(pairs):
    errs = [abs(p - y) / y * 100 for p, y in pairs]
    signed = [(p - y) / y * 100 for p, y in pairs]
    return sum(errs) / len(errs), max(errs), sum(signed) / len(signed)


def digit_features(text, lang):
    """Old model: `number` = count of ASCII digits (a flat cost per digit)."""
    f = features(text, lang)
    f["number"] = sum(c.isascii() and c.isdigit() for c in text)
    return f


def print_number_table(data):
    lines = []
    for (engine, lang), items in sorted(data.items()):
        out = []
        for feat in (digit_features, features):
            rows = [(feat(t, lang), y) for t, y in items]
            coef = {k: round(v) for k, v in fit(rows, USED[lang]).items()}
            sel = [i for i, (t, _) in enumerate(items) if any(c.isascii() and c.isdigit() for c in t)]
            pairs = lambda idx: [(predict(coef, rows[i][0]), rows[i][1]) for i in idx]
            rest = [i for i in range(len(items)) if i not in sel]
            out.append((errors(pairs(range(len(items)))), errors(pairs(sel)), errors(pairs(rest))))
        lines.append((engine, lang, len(sel), out))
    print("| engine | lang | n with numbers | flat digits: all / numbers / others (MAPE, worst) | number reading: all / numbers / others (MAPE, worst) |")
    print("|---|---|---|---|---|")
    fmt = lambda e: f"{e[0]:.1f}% ({e[1]:.0f}%)"
    for engine, lang, n, (o, nw) in lines:
        print(f"| {engine} | {lang} | {n} | {fmt(o[0])} / {fmt(o[1])} / {fmt(o[2])} | {fmt(nw[0])} / {fmt(nw[1])} / {fmt(nw[2])} |")
    print()


def main():
    path = Path(sys.argv[1] if len(sys.argv) > 1 else Path(__file__).with_name("measured.csv"))
    data = {}
    with open(path, newline="", encoding="utf-8") as fh:
        for r in csv.DictReader(fh):
            data.setdefault((r["engine"], r["lang"]), []).append((r["text"], int(r["ms"])))

    table, arms = [], []
    for (engine, lang), items in sorted(data.items()):
        rows = [(features(t, lang), y) for t, y in items]
        coef = fit(rows, USED[lang])
        rounded = {k: round(v) for k, v in coef.items()}
        args = ", ".join(str(rounded[k]) for k in FEATURES)
        arms.append(f"        ({engine.capitalize()}, {lang.capitalize()}) => r({args}),")
        new = [(predict(rounded, f), y) for f, y in rows]
        loo = []
        for i in range(len(rows)):
            c = fit(rows[:i] + rows[i + 1 :], USED[lang])
            loo.append((predict(c, rows[i][0]), rows[i][1]))
        sb = [(old_storyboard(t, lang), y) for t, y in items]
        ck = [(old_check(t, lang), y) for t, y in items]
        table.append((engine, lang, len(items), errors(sb), errors(ck), errors(new), errors(loo)))

    # Number sentences (any ASCII digit) alone: the old flat per-digit model, refitted on this
    # corpus, vs the number-reading model.
    print_number_table(data)

    order = {"Say": 0, "Kokoro": 1, "Zh": 0, "Ja": 1, "En": 2}
    arms.sort(key=lambda a: [order[w.strip("(,)")] for w in a.split()[:2]])
    print("Rates for speech_rates() in src/estimate.rs, r(han, kana, letters, words, number, minor, major, base):")
    print("\n".join(arms))
    print()
    print("| engine | lang | n | old storyboard MAPE / worst / bias | old check MAPE / worst / bias | new MAPE / worst / bias | new leave-one-out MAPE / worst |")
    print("|---|---|---|---|---|---|---|")
    for engine, lang, n, sb, ck, new, loo in table:
        fmt = lambda e: f"{e[0]:.1f}% / {e[1]:.0f}% / {e[2]:+.0f}%"
        print(f"| {engine} | {lang} | {n} | {fmt(sb)} | {fmt(ck)} | {fmt(new)} | {loo[0]:.1f}% / {loo[1]:.0f}% |")


if __name__ == "__main__":
    main()
