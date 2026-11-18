// The narration voice of a scene: Kokoro voices by language and gender (Chinese labels), a ▶
// sample of the chosen voice (made locally by the render's own narration code, cached), and for
// an old project that names a macOS `say` voice: 「这个声音已不推荐」 with 「换成 Kokoro」.
import { useEffect, useRef, useState } from "react";
import type { Narration } from "../../../src/types/timeline";
import { listenNarration } from "../listen";
import { withVoice } from "../narration";
import {
  DEFAULT_KOKORO, KOKORO_VOICES, LANG_LABELS, isSayVoice, kokoroVoice, sampleTimeline, sayToKokoro, textLang,
  type VoiceLang,
} from "../voices";
import { AudioClip } from "../audioClip";

type Props = {
  narration: Narration;
  /** What the scene says (its spoken line, else its text): the automatic voice follows its language. */
  spoken: string | null;
  onChange: (n: Narration) => void;
  /** URL of a public-relative file (the sample). Missing: no ▶ button. */
  audioUrl?: (path: string) => string;
};

// Samples already made in this session, by voice.
const samples = new Map<string, string>();

export const VoicePicker = ({ narration, spoken, onChange, audioUrl }: Props) => {
  const voice = narration.voice ?? "";
  const old = isSayVoice(voice);
  const lang: VoiceLang = textLang(spoken);
  // The voice the ▶ button plays: the chosen Kokoro voice, else the automatic one.
  const sampleVoice = kokoroVoice(voice) ?? kokoroVoice(DEFAULT_KOKORO[lang])!;
  const [status, setStatus] = useState<{ busy: boolean; error: string | null }>({ busy: false, error: null });
  const audio = useRef<AudioClip | null>(null);
  useEffect(() => () => audio.current?.pause(), []);

  const play = async () => {
    if (!audioUrl) return;
    audio.current?.pause();
    setStatus({ busy: true, error: null });
    let source = samples.get(sampleVoice.id);
    if (!source) {
      const got = await listenNarration(JSON.stringify(sampleTimeline(sampleVoice)), [], "kokoro");
      if (!got.ok) {
        setStatus({ busy: false, error: got.error });
        return;
      }
      source = got.clips[0]?.source;
      if (!source) {
        setStatus({ busy: false, error: "No sample was made." });
        return;
      }
      samples.set(sampleVoice.id, source);
    }
    const a = new AudioClip(audioUrl(source));
    audio.current = a;
    setStatus({ busy: false, error: null });
    void a.play().catch((e) => setStatus({ busy: false, error: String(e) }));
  };

  return (
    <div className="voice-picker" data-voice-picker>
      <div className="voice-row">
        <select value={voice} onChange={(e) => onChange(withVoice(narration, e.target.value || null))} data-voice-select>
          <option value="">自动（按文字语言 · Kokoro {DEFAULT_KOKORO[lang]}）</option>
          {(Object.keys(LANG_LABELS) as VoiceLang[]).map((l) => (
            <optgroup key={l} label={LANG_LABELS[l]}>
              {KOKORO_VOICES.filter((v) => v.lang === l).map((v) => (
                <option key={v.id} value={v.id}>{v.label} · {v.id}</option>
              ))}
            </optgroup>
          ))}
          {voice && !kokoroVoice(voice) && (
            <option value={voice}>{voice}{old ? "（旧 macOS say 声音）" : ""}</option>
          )}
        </select>
        {audioUrl && (
          <button
            type="button"
            className="btn ghost small"
            disabled={status.busy}
            title={`试听 ${sampleVoice.label}（本机合成，第一次要几十秒）`}
            onClick={() => void play()}
            data-voice-sample
          >
            {status.busy ? "合成中…" : "▶ 试听声音"}
          </button>
        )}
      </div>
      {old && (
        <div className="voice-old" role="status" data-voice-old>
          <span>这个声音已不推荐（macOS 自带的 say，听起来比较机械）。</span>
          <button
            type="button"
            className="btn small"
            onClick={() => onChange(withVoice(narration, sayToKokoro(voice, spoken)))}
            data-voice-to-kokoro
          >
            换成 Kokoro（{kokoroVoice(sayToKokoro(voice, spoken))?.label ?? sayToKokoro(voice, spoken)}）
          </button>
        </div>
      )}
      {status.error && <span className="field-error" role="alert">{status.error}</span>}
    </div>
  );
};
