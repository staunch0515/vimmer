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
    digits   ASCII digits
    minor    , ; : 、 ， ； ： and dashes  (short pause)
    major    . ! ? 。 ！ ？ … and line breaks   (sentence pause; a run counts once)
    const    1 per clip (lead-in / tail silence)
"""

import csv
import sys
from pathlib import Path

FEATURES = ["han", "kana", "letters", "words", "digits", "minor", "major", "const"]
# Which features each language's model may use (others are fixed at 0). Few points per
# language, so few features.
USED = {
    "zh": ["han", "letters", "words", "digits", "minor", "major", "const"],
    "ja": ["han", "kana", "letters", "words", "digits", "minor", "major", "const"],
    "en": ["letters", "words", "digits", "minor", "major", "const"],
}
MINOR = set(",;:、，；：—–")
MAJOR = set(".!?。！？…\n")


def is_han(c):
    return "一" <= c <= "鿿" or "㐀" <= c <= "䶿" or c == "々"


def is_kana(c):
    return "ぁ" <= c <= "ゖ" or "ァ" <= c <= "ヺ" or c == "ー"


def features(text):
    f = dict.fromkeys(FEATURES, 0)
    f["const"] = 1
    prev = ""
    in_word = False
    for i, c in enumerate(text):
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
                f["digits"] += 1
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


def main():
    path = Path(sys.argv[1] if len(sys.argv) > 1 else Path(__file__).with_name("measured.csv"))
    data = {}
    with open(path, newline="", encoding="utf-8") as fh:
        for r in csv.DictReader(fh):
            data.setdefault((r["engine"], r["lang"]), []).append((r["text"], int(r["ms"])))

    table, arms = [], []
    for (engine, lang), items in sorted(data.items()):
        rows = [(features(t), y) for t, y in items]
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

    order = {"Say": 0, "Kokoro": 1, "Zh": 0, "Ja": 1, "En": 2}
    arms.sort(key=lambda a: [order[w.strip("(,)")] for w in a.split()[:2]])
    print("Rates for speech_rates() in src/estimate.rs, r(han, kana, letters, words, digits, minor, major, base):")
    print("\n".join(arms))
    print()
    print("| engine | lang | n | old storyboard MAPE / worst / bias | old check MAPE / worst / bias | new MAPE / worst / bias | new leave-one-out MAPE / worst |")
    print("|---|---|---|---|---|---|---|")
    for engine, lang, n, sb, ck, new, loo in table:
        fmt = lambda e: f"{e[0]:.1f}% / {e[1]:.0f}% / {e[2]:+.0f}%"
        print(f"| {engine} | {lang} | {n} | {fmt(sb)} | {fmt(ck)} | {fmt(new)} | {loo[0]:.1f}% / {loo[1]:.0f}% |")


if __name__ == "__main__":
    main()
