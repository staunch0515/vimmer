import type { FitTime, VideoClip } from "../../../src/types/timeline";
import { MAX_CLIP_SPEED, MIN_CLIP_SPEED } from "../../../src/motion/video";
import { Field, MediaInput, NumField } from "./Inspector";

const FIT_TIMES: { value: FitTime; label: string }[] = [
  { value: "freeze", label: "freeze (hold last frame)" },
  { value: "loop", label: "loop" },
  { value: "stretch", label: "stretch (fit the speed)" },
];

/**
 * The fields of a video clip, for a scene background or a video layer. `patch` sets keys; a key set
 * to undefined goes back to its default (and is left out of the JSON).
 */
export const ClipFields = ({ clip, patch }: { clip: VideoClip; patch: (p: Record<string, unknown>) => void }) => {
  const stretch = clip.fit_time === "stretch";
  return (
    <>
      <Field label="Video" hint=".mp4 .mov .m4v .webm under public/">
        <MediaInput kind="video" value={clip.src} placeholder="assets/clips/counter-6s.mp4" onChange={(v) => patch({ src: v })} />
      </Field>
      <div className="two-col">
        <NumField
          label="Trim start"
          suffix="ms"
          value={clip.trim_start_ms ?? 0}
          step={100}
          validate={(v) => (Number.isInteger(v) && v >= 0 ? null : "0 ms or more")}
          onCommit={(v) => patch({ trim_start_ms: v || undefined })}
        />
        <Field label="Trim end" suffix="ms" hint="blank = file end">
          <input
            type="number"
            step={100}
            value={clip.trim_end_ms ?? ""}
            placeholder="end"
            data-clip-field="trim_end_ms"
            onChange={(e) => {
              const v = e.target.value.trim();
              patch({ trim_end_ms: v === "" ? undefined : Math.max(0, Math.round(Number(v))) });
            }}
          />
        </Field>
      </div>
      <div className="two-col">
        <NumField
          label="Speed"
          suffix="×"
          hint={stretch ? "set by stretch" : undefined}
          value={clip.speed ?? 1}
          step={0.25}
          validate={(v) => (v >= MIN_CLIP_SPEED && v <= MAX_CLIP_SPEED ? null : `${MIN_CLIP_SPEED} to ${MAX_CLIP_SPEED}`)}
          onCommit={(v) => patch({ speed: v === 1 ? undefined : v })}
        />
        <Field label="When shorter">
          <select value={clip.fit_time ?? "freeze"} onChange={(e) => patch({ fit_time: e.target.value === "freeze" ? undefined : e.target.value })}>
            {FIT_TIMES.map((f) => (
              <option key={f.value} value={f.value}>{f.label}</option>
            ))}
          </select>
        </Field>
      </div>
      <div className="two-col">
        <Field label="Fit">
          <select value={clip.fit ?? "cover"} onChange={(e) => patch({ fit: e.target.value === "cover" ? undefined : e.target.value })}>
            <option value="cover">cover (crop)</option>
            <option value="contain">contain</option>
          </select>
        </Field>
        <NumField
          label="Volume"
          hint="0 = muted"
          value={clip.volume ?? 0}
          step={0.1}
          validate={(v) => (v >= 0 && v <= 1 ? null : "0 to 1")}
          onCommit={(v) => patch({ volume: v || undefined })}
        />
      </div>
    </>
  );
};
