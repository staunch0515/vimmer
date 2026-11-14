import type { Dispatch } from "react";
import type { BeatSync, Music, Scene, SoundEffect } from "../../../src/types/timeline";
import { DEFAULT_SFX_VOLUME } from "../../../src/motion/sfx";
import { BUNDLED_SFX, type DetectedBeats } from "../beats";
import type { Action } from "../state";
import { Field, MediaInput } from "./Inspector";

export type BeatStatus = { busy: boolean; error: string | null };

const MAX_SFX = 16;

/** Beat sync settings and the "Detect beats" button, in the music tab. */
export const BeatSyncFields = ({
  music,
  patch,
  detected,
  status,
  onDetect,
}: {
  music: Music;
  patch: (p: Partial<Music>) => void;
  detected: DetectedBeats | null;
  status: BeatStatus;
  onDetect: () => void;
}) => {
  const sync = music.beat_sync ?? null;
  const on = !!sync && sync.enabled !== false;
  const setSync = (p: Partial<BeatSync>) => patch({ beat_sync: { ...(sync ?? {}), enabled: true, ...p } });
  const current = detected && detected.source === music.source ? detected : null;
  return (
    <>
      <div className="section-title">Beat sync</div>
      <label className="check">
        <input
          type="checkbox"
          data-beat-sync
          checked={on}
          onChange={(e) => (e.target.checked ? setSync({}) : patch({ beat_sync: undefined }))}
        />
        <span>Snap scene cuts to the beat (at render time, after narration)</span>
      </label>
      {on && (
        <div className="two-col">
          <Field label="Step" hint="beats between cuts">
            <select value={sync?.every ?? 1} onChange={(e) => setSync({ every: Number(e.target.value) as 1 | 2 | 4 })}>
              <option value={1}>every beat</option>
              <option value={2}>every 2nd beat</option>
              <option value={4}>bar starts</option>
            </select>
          </Field>
          <Field label="Move" hint="which way a cut may go">
            <select value={sync?.mode ?? "nearest"} onChange={(e) => setSync({ mode: e.target.value as "nearest" | "later" })}>
              <option value="nearest">nearest beat</option>
              <option value="later">only later</option>
            </select>
          </Field>
          <Field label="Max shift" suffix="ms">
            <input
              type="number"
              min={1}
              max={1000}
              step={50}
              value={sync?.max_shift_ms ?? 250}
              onChange={(e) => setSync({ max_shift_ms: Math.min(1000, Math.max(1, Number(e.target.value) || 250)) })}
            />
          </Field>
        </div>
      )}
      <div className="button-row">
        <button className="btn small" data-detect-beats disabled={status.busy || !music.source} onClick={onDetect}>
          {status.busy ? "Detecting…" : "Detect beats"}
        </button>
      </div>
      {status.error && <span className="field-error">{status.error}</span>}
      {current && (
        <div className={current.low_confidence ? "field-error" : "dim"} data-beat-result>
          {current.bpm.toFixed(1)} BPM · confidence {current.confidence.toFixed(2)} · {current.beats_ms.length} beats
          {current.low_confidence ? " — the beat is unclear; set the tempo by hand below for beat sync." : " (markers on the ruler)"}
        </div>
      )}
      <label className="check">
        <input
          type="checkbox"
          checked={!!music.beats}
          onChange={(e) =>
            patch({
              beats: e.target.checked
                ? { bpm: Math.round((current?.bpm ?? 120) * 10) / 10, offset_ms: Math.round(current?.beats_ms[0] ?? 0) }
                : undefined,
            })
          }
        />
        <span>Set the tempo by hand (skips detection)</span>
      </label>
      {music.beats && (
        <div className="two-col">
          <Field label="Tempo" suffix="BPM">
            <input
              type="number"
              min={30}
              max={300}
              step={0.1}
              value={music.beats.bpm}
              onChange={(e) => patch({ beats: { ...music.beats!, bpm: Math.min(300, Math.max(30, Number(e.target.value) || 120)) } })}
            />
          </Field>
          <Field label="First beat" suffix="ms">
            <input
              type="number"
              min={0}
              step={10}
              value={music.beats.offset_ms ?? 0}
              onChange={(e) => patch({ beats: { ...music.beats!, offset_ms: Math.max(0, Number(e.target.value) || 0) } })}
            />
          </Field>
        </div>
      )}
    </>
  );
};

const describe = (src: string) => BUNDLED_SFX.find((s) => s.src === src)?.label ?? src;

/** The scene's sound effects: what, when (a time or a layer event), how loud. */
export const SfxList = ({ scene, index, dispatch }: { scene: Scene; index: number; dispatch: Dispatch<Action> }) => {
  const list = scene.sfx ?? [];
  const layers = scene.layers ?? [];
  const set = (next: SoundEffect[]) => dispatch({ type: "patchScene", index, patch: { sfx: next.length ? next : undefined } });
  const patchAt = (k: number, p: Partial<SoundEffect>) =>
    set(list.map((fx, i) => (i === k ? (Object.fromEntries(Object.entries({ ...fx, ...p }).filter(([, v]) => v !== undefined)) as SoundEffect) : fx)));
  return (
    <div className="sfx-list" data-sfx-list>
      {list.length === 0 && <div className="dim">No sound effects. They play on top of the music at a moment of this scene.</div>}
      {list.map((fx, k) => {
        const when = fx.on ? fx.on.event : "time";
        const badLayer = fx.on && !layers.some((l) => l.id === fx.on!.layer);
        return (
          <div key={k} className="sfx-row" data-sfx-row={k}>
            <div className="sfx-head">
              <span className="sfx-name">♪ {describe(fx.src)}</span>
              <button className="btn ghost small" aria-label={`Remove sound ${k + 1}`} onClick={() => set(list.filter((_, i) => i !== k))}>
                Remove
              </button>
            </div>
            <Field label="Sound">
              <MediaInput kind="audio" value={fx.src} placeholder="assets/sfx/pop.wav" onChange={(v) => patchAt(k, { src: v })} />
            </Field>
            <div className="two-col">
              <Field label="When">
                <select
                  data-sfx-when
                  value={when}
                  onChange={(e) => {
                    const v = e.target.value;
                    if (v === "time") patchAt(k, { on: undefined, at_ms: 0 });
                    else patchAt(k, { at_ms: undefined, on: { layer: fx.on?.layer ?? layers[0]?.id ?? "", event: v as "enter" | "exit" } });
                  }}
                >
                  <option value="time">at a time</option>
                  <option value="enter" disabled={!layers.length}>a layer enters</option>
                  <option value="exit" disabled={!layers.length}>a layer exits</option>
                </select>
              </Field>
              {fx.on ? (
                <Field label="Layer">
                  <select data-sfx-layer value={fx.on.layer} onChange={(e) => patchAt(k, { on: { ...fx.on!, layer: e.target.value } })}>
                    {badLayer && <option value={fx.on.layer}>{fx.on.layer} (missing)</option>}
                    {layers.map((l) => (
                      <option key={l.id} value={l.id}>{l.id}</option>
                    ))}
                  </select>
                </Field>
              ) : (
                <Field label="At" suffix="ms" hint={`0–${scene.duration_ms - 1}`}>
                  <input
                    type="number"
                    min={0}
                    max={scene.duration_ms - 1}
                    step={50}
                    value={fx.at_ms ?? 0}
                    onChange={(e) => patchAt(k, { at_ms: Math.min(scene.duration_ms - 1, Math.max(0, Math.round(Number(e.target.value) || 0))) })}
                  />
                </Field>
              )}
            </div>
            <Field label="Volume" suffix={`${Math.round((fx.volume ?? DEFAULT_SFX_VOLUME) * 100)} %`}>
              <input
                type="range"
                min={0}
                max={1}
                step={0.05}
                value={fx.volume ?? DEFAULT_SFX_VOLUME}
                onChange={(e) => patchAt(k, { volume: Number(e.target.value) })}
              />
            </Field>
            <label className="check">
              <input type="checkbox" checked={!!fx.duck_music} onChange={(e) => patchAt(k, { duck_music: e.target.checked || undefined })} />
              <span>Lower the music under it</span>
            </label>
          </div>
        );
      })}
      <Field label="Add a sound">
        <select
          data-add-sfx
          value=""
          disabled={list.length >= MAX_SFX}
          onChange={(e) => {
            if (!e.target.value) return;
            set([...list, { src: e.target.value, at_ms: 0 }]);
          }}
        >
          <option value="">{list.length >= MAX_SFX ? `at most ${MAX_SFX} per scene` : "choose…"}</option>
          {BUNDLED_SFX.map((s) => (
            <option key={s.src} value={s.src}>{s.label}</option>
          ))}
        </select>
      </Field>
    </div>
  );
};
