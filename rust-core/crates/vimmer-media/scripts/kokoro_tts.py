#!/usr/bin/env python3
"""Batch narration with Kokoro (ONNX) for Vimmer.

Usage:
    python -I kokoro_tts.py --models DIR BATCH.json

BATCH.json holds every clip of one render, so the models load once:

    {"items": [
      {"text": "西安钟楼", "lang": "zh", "voice": "zf_001", "speed": 1.0, "out": "/abs/a.wav"},
      ...
    ]}

`lang` is `zh`, `ja` or `en`. Each `out` is written as a 24 kHz mono 16-bit WAV, first to a
temporary name and then renamed, so a killed run never leaves a half-written file behind.

Models (under DIR):
    zh: kokoro-v1.1-zh.onnx + voices-v1.1-zh.bin + config-v1.1-zh.json, misaki ZHG2P(version="1.1")
        (v1.0's Chinese G2P runs syllables together, so it is not used for Chinese)
    ja: kokoro-v1.0.onnx + voices-v1.0.bin, misaki JAG2P
    en: kokoro-v1.0.onnx + voices-v1.0.bin, espeak-ng (bundled with kokoro-onnx)

English words inside Chinese text are phonemized with misaki's espeak fallback, which uses the
espeak-ng library that kokoro-onnx already ships, so nothing extra has to be downloaded.

Output: `warning: ...` lines on stdout for the caller to show, progress on stderr.
Exit status 0 when every clip was written; non-zero with a message on stderr otherwise.
"""

import argparse
import json
import os
import sys
import time
import warnings
from pathlib import Path

# jieba (used by the Chinese G2P) trips SyntaxWarnings on import under Python 3.12.
warnings.filterwarnings("ignore", category=SyntaxWarning)

DEFAULT_VOICE = {"zh": "zf_001", "ja": "jf_alpha", "en": "af_heart"}
V10 = ("kokoro-v1.0.onnx", "voices-v1.0.bin", None)
V11_ZH = ("kokoro-v1.1-zh.onnx", "voices-v1.1-zh.bin", "config-v1.1-zh.json")
MODEL_FOR = {"zh": V11_ZH, "ja": V10, "en": V10}


def log(msg):
    print(msg, file=sys.stderr, flush=True)


def warn(msg):
    print(f"warning: {msg}", flush=True)


class Engines:
    """Loads each model and G2P on first use, once per run."""

    def __init__(self, models: Path):
        self.models = models
        self.kokoro = {}
        self.g2p = {}

    def model(self, lang):
        files = MODEL_FOR[lang]
        if files not in self.kokoro:
            from kokoro_onnx import Kokoro

            onnx, voices, config = files
            t0 = time.perf_counter()
            kwargs = {}
            if config:
                kwargs["vocab_config"] = str(self.models / config)
            self.kokoro[files] = Kokoro(str(self.models / onnx), str(self.models / voices), **kwargs)
            log(f"kokoro: loaded {onnx} in {time.perf_counter() - t0:.1f} s")
        return self.kokoro[files]

    def phonemes(self, lang, text):
        if lang not in self.g2p:
            if lang == "zh":
                from types import SimpleNamespace

                from misaki import zh
                from misaki.espeak import EspeakFallback

                fallback = EspeakFallback(british=False)

                def english(word):
                    ps, _ = fallback(SimpleNamespace(text=word))
                    return ps or ""

                self.g2p[lang] = zh.ZHG2P(version="1.1", en_callable=english)
            elif lang == "ja":
                from misaki import ja

                self.g2p[lang] = ja.JAG2P()
            else:
                self.g2p[lang] = None
        g2p = self.g2p[lang]
        return None if g2p is None else g2p(text)[0]


def synthesize(engines, item):
    lang = item["lang"]
    if lang not in MODEL_FOR:
        raise ValueError(f"unsupported language `{lang}`")
    text = item["text"]
    kokoro = engines.model(lang)
    voice = item.get("voice") or DEFAULT_VOICE[lang]
    if voice not in kokoro.get_voices():
        warn(f"Kokoro voice `{voice}` does not exist for {lang}, using {DEFAULT_VOICE[lang]}")
        voice = DEFAULT_VOICE[lang]
    speed = float(item.get("speed", 1.0))
    phonemes = engines.phonemes(lang, text)
    if phonemes is None:
        espeak_lang = "en-gb" if voice.startswith("b") else "en-us"
        samples, rate = kokoro.create(text, voice=voice, speed=speed, lang=espeak_lang)
    else:
        if not phonemes.strip():
            raise ValueError(f"no phonemes for text: {text!r}")
        samples, rate = kokoro.create(phonemes, voice=voice, speed=speed, is_phonemes=True)
    if len(samples) == 0:
        raise ValueError(f"no audio for text: {text!r}")
    return samples, rate


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--models", required=True, type=Path, help="Kokoro model directory")
    parser.add_argument("batch", help="batch JSON file, or - for stdin")
    args = parser.parse_args()

    raw = sys.stdin.read() if args.batch == "-" else Path(args.batch).read_text(encoding="utf-8")
    items = json.loads(raw)["items"]

    import soundfile as sf

    engines = Engines(args.models)
    for i, item in enumerate(items, 1):
        t0 = time.perf_counter()
        samples, rate = synthesize(engines, item)
        out = Path(item["out"])
        out.parent.mkdir(parents=True, exist_ok=True)
        part = out.with_name(out.name + ".part")
        sf.write(str(part), samples, rate, format="WAV", subtype="PCM_16")
        os.replace(part, out)
        took = time.perf_counter() - t0
        log(f"kokoro: [{i}/{len(items)}] {item['lang']} {len(samples) / rate:.1f} s audio in {took:.1f} s")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as e:  # noqa: BLE001 - one clear line for the caller's warning
        log(f"kokoro_tts: {type(e).__name__}: {e}")
        sys.exit(1)
