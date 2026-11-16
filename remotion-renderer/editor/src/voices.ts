// Narration voices in the editor: the Kokoro voices to pick from (by language and gender, with
// Chinese labels), old macOS `say` voices, and 「换成 Kokoro」 (a sensible Kokoro voice for an old
// `say` voice). Pure (type imports only), so it is unit-tested with Node (test/editor-voices.test.ts).
// Mirrors `looks_like_kokoro_voice` / `is_say_voice` in vimmer-media/src/kokoro.rs.

import type { VideoTimeline } from "../../src/types/timeline";

export type VoiceLang = "zh" | "en" | "ja";
export type KokoroVoice = { id: string; lang: VoiceLang; gender: "f" | "m"; label: string };

/** A curated set of the installed Kokoro voices (v1.1-zh for Chinese, v1.0 for English and Japanese). */
export const KOKORO_VOICES: KokoroVoice[] = [
  { id: "zf_001", lang: "zh", gender: "f", label: "女声 1（默认）" },
  { id: "zf_002", lang: "zh", gender: "f", label: "女声 2" },
  { id: "zf_003", lang: "zh", gender: "f", label: "女声 3" },
  { id: "zf_004", lang: "zh", gender: "f", label: "女声 4" },
  { id: "zf_005", lang: "zh", gender: "f", label: "女声 5" },
  { id: "zm_009", lang: "zh", gender: "m", label: "男声 1" },
  { id: "zm_010", lang: "zh", gender: "m", label: "男声 2" },
  { id: "zm_011", lang: "zh", gender: "m", label: "男声 3" },
  { id: "zm_012", lang: "zh", gender: "m", label: "男声 4" },
  { id: "zm_013", lang: "zh", gender: "m", label: "男声 5" },
  { id: "af_heart", lang: "en", gender: "f", label: "美式女声 Heart（默认）" },
  { id: "af_bella", lang: "en", gender: "f", label: "美式女声 Bella" },
  { id: "af_nicole", lang: "en", gender: "f", label: "美式女声 Nicole（轻声）" },
  { id: "af_sarah", lang: "en", gender: "f", label: "美式女声 Sarah" },
  { id: "am_michael", lang: "en", gender: "m", label: "美式男声 Michael" },
  { id: "am_adam", lang: "en", gender: "m", label: "美式男声 Adam" },
  { id: "am_eric", lang: "en", gender: "m", label: "美式男声 Eric" },
  { id: "bf_emma", lang: "en", gender: "f", label: "英式女声 Emma" },
  { id: "bf_isabella", lang: "en", gender: "f", label: "英式女声 Isabella" },
  { id: "bm_george", lang: "en", gender: "m", label: "英式男声 George" },
  { id: "bm_lewis", lang: "en", gender: "m", label: "英式男声 Lewis" },
  { id: "jf_alpha", lang: "ja", gender: "f", label: "日语女声 Alpha（默认）" },
  { id: "jf_gongitsune", lang: "ja", gender: "f", label: "日语女声 Gongitsune" },
  { id: "jf_nezumi", lang: "ja", gender: "f", label: "日语女声 Nezumi" },
  { id: "jf_tebukuro", lang: "ja", gender: "f", label: "日语女声 Tebukuro" },
  { id: "jm_kumo", lang: "ja", gender: "m", label: "日语男声 Kumo" },
];

export const LANG_LABELS: Record<VoiceLang, string> = { zh: "中文", en: "英语 English", ja: "日语 日本語" };

/** Kokoro's voice when a scene names none (mirrors `KokoroLang::default_voice`). */
export const DEFAULT_KOKORO: Record<VoiceLang, string> = { zh: "zf_001", en: "af_heart", ja: "jf_alpha" };

/** A language letter, `f` or `m`, `_`, then lowercase letters or digits (`zf_001`, `af_heart`). */
export const looksLikeKokoroVoice = (v: string): boolean => /^[a-z][fm]_[a-z0-9]+$/.test(v);

/** A voice name that is a macOS `say` voice: set, and not shaped like a Kokoro voice. */
export const isSayVoice = (v: string | null | undefined): v is string => !!v && !!v.trim() && !looksLikeKokoroVoice(v.trim());

/** The language a text is read in (mirrors `guess_language`: kana → ja, other CJK → zh, else en). */
export const textLang = (text: string | null | undefined): VoiceLang => {
  const t = text ?? "";
  if (/[぀-ヿ]/.test(t)) return "ja";
  if (/[一-鿿㐀-䶿]/.test(t)) return "zh";
  return "en";
};

// Known macOS `say` voices: language, gender, and British English.
const SAY_VOICES: Record<string, { lang: VoiceLang; gender: "f" | "m"; british?: boolean }> = {
  tingting: { lang: "zh", gender: "f" }, "ting-ting": { lang: "zh", gender: "f" }, meijia: { lang: "zh", gender: "f" },
  "mei-jia": { lang: "zh", gender: "f" }, sinji: { lang: "zh", gender: "f" }, "sin-ji": { lang: "zh", gender: "f" },
  lili: { lang: "zh", gender: "f" }, yushu: { lang: "zh", gender: "f" }, "yu-shu": { lang: "zh", gender: "f" },
  limu: { lang: "zh", gender: "m" }, "li-mu": { lang: "zh", gender: "m" }, binbin: { lang: "zh", gender: "m" },
  kyoko: { lang: "ja", gender: "f" }, otoya: { lang: "ja", gender: "m" }, hattori: { lang: "ja", gender: "m" }, "o-ren": { lang: "ja", gender: "f" },
  samantha: { lang: "en", gender: "f" }, allison: { lang: "en", gender: "f" }, ava: { lang: "en", gender: "f" },
  susan: { lang: "en", gender: "f" }, victoria: { lang: "en", gender: "f" }, karen: { lang: "en", gender: "f" },
  moira: { lang: "en", gender: "f" }, tessa: { lang: "en", gender: "f" }, zoe: { lang: "en", gender: "f" },
  alex: { lang: "en", gender: "m" }, fred: { lang: "en", gender: "m" }, tom: { lang: "en", gender: "m" },
  aaron: { lang: "en", gender: "m" }, evan: { lang: "en", gender: "m" }, nathan: { lang: "en", gender: "m" },
  daniel: { lang: "en", gender: "m", british: true }, oliver: { lang: "en", gender: "m", british: true },
  serena: { lang: "en", gender: "f", british: true }, kate: { lang: "en", gender: "f", british: true },
  stephanie: { lang: "en", gender: "f", british: true }, arthur: { lang: "en", gender: "m", british: true },
};

/**
 * 「换成 Kokoro」: the Kokoro voice for an old `say` voice. A known voice keeps its language and
 * gender (Tingting → zf_001, Kyoko → jf_alpha, Samantha → af_heart, Alex → am_michael, Daniel →
 * bm_george); an unknown one gets the default voice of the language the scene speaks.
 */
export const sayToKokoro = (sayVoice: string, spoken: string | null | undefined): string => {
  const known = SAY_VOICES[sayVoice.trim().toLowerCase().replace(/\s*\(.*\)$/, "")];
  if (!known) return DEFAULT_KOKORO[textLang(spoken)];
  if (known.lang === "zh") return known.gender === "f" ? "zf_001" : "zm_009";
  if (known.lang === "ja") return known.gender === "f" ? "jf_alpha" : "jm_kumo";
  if (known.british) return known.gender === "f" ? "bf_emma" : "bm_george";
  return known.gender === "f" ? "af_heart" : "am_michael";
};

export const kokoroVoice = (id: string | null | undefined): KokoroVoice | undefined => KOKORO_VOICES.find((v) => v.id === id);

/** A short sentence in the voice's language, for ▶ 试听这个声音. */
export const SAMPLE_TEXT: Record<VoiceLang, string> = {
  zh: "你好，这是我的声音。欢迎收看这个视频。",
  en: "Hello, this is my voice. Welcome to this video.",
  ja: "こんにちは、これは私の声です。",
};

/** A one-scene timeline that makes the voice's sample through the render's narration code. */
export const sampleTimeline = (voice: KokoroVoice): VideoTimeline => ({
  fps: 30,
  resolution: { width: 1920, height: 1080 },
  scenes: [
    {
      id: `voice-sample-${voice.id.replace(/_/g, "-")}`,
      duration_ms: 1000,
      text: SAMPLE_TEXT[voice.lang],
      highlights: [],
      image: null,
      motion: "static",
      audio: null,
      narration: { voice: voice.id },
    },
  ],
  subtitles: [],
});

/** Scenes that name a `say` voice (shown 「这个声音已不推荐」). */
export const sayVoiceScenes = (t: VideoTimeline): number[] =>
  t.scenes.flatMap((s, i) => (isSayVoice(s.narration?.voice) ? [i] : []));
