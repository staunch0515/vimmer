import { useState } from "react";
import { fraction, type RenderView } from "../render";
import {
  AUTO_SUBTITLE_MODES,
  TTS_ENGINES,
  blocksRender,
  sortFindings,
  summaryText,
  type CheckFinding,
  type CheckView,
  type RenderDefaults,
  type RenderOptions,
} from "../renderOptions";

type Props = {
  view: RenderView;
  options: RenderOptions;
  defaults: RenderDefaults;
  onOptions: (o: RenderOptions) => void;
  /** The check for the current timeline and engine (a stale one shows as running). */
  check: CheckView;
  onRecheck: () => void;
  onFinding: (f: CheckFinding) => void;
  onRender: () => void;
  onCancel: () => void;
  onReveal: (path: string) => void;
  onDismiss: () => void;
};

const SEVERITY_LABEL = { error: "Error", warning: "Warning", info: "Info" } as const;

/**
 * A number field that may be left empty (= the default). `min` and the stored value are in the
 * option's unit (ms for the chunk length); the field shows `value / scale`. What is typed stays as
 * typed until it is valid, so a half-typed number is never clamped.
 */
const NumberField = ({ label, value, placeholder, min, scale = 1, onChange }: {
  label: string;
  value: number | null;
  placeholder: number;
  min: number;
  scale?: number;
  onChange: (v: number | null) => void;
}) => {
  const [draft, setDraft] = useState<string | null>(null);
  const parsed = (text: string): number | null | undefined => {
    if (text.trim() === "") return null;
    const n = Math.round(Number(text) * scale);
    return Number.isFinite(n) && n >= min ? n : undefined;
  };
  const invalid = draft !== null && parsed(draft) === undefined;
  return (
    <label className="field">
      <span className="field-label">{label}</span>
      <input
        type="number"
        min={min / scale}
        value={draft ?? (value === null ? "" : String(value / scale))}
        placeholder={String(placeholder / scale)}
        aria-invalid={invalid}
        onChange={(e) => {
          setDraft(e.target.value);
          const v = parsed(e.target.value);
          if (v !== undefined) onChange(v);
        }}
        onBlur={() => setDraft(null)}
      />
      {invalid && <span className="field-error">At least {min / scale}</span>}
    </label>
  );
};

const Findings = ({ check, onRecheck, onFinding }: Pick<Props, "check" | "onRecheck" | "onFinding">) => {
  if (check.status === "idle" || check.status === "running")
    return <div className="render-check dim">Checking the timeline…</div>;
  if (check.status === "failed")
    return (
      <div className="render-check">
        <span className="bad">The check could not run: {check.error}</span>
        <button className="btn small ghost" onClick={onRecheck}>Retry</button>
      </div>
    );
  const findings = sortFindings(check.report.findings);
  const { errors } = check.report.summary;
  return (
    <div className="render-check">
      <div className="render-check-head">
        <strong className={errors ? "bad" : findings.length ? "warn" : "ok"}>{summaryText(check.report)}</strong>
        <span className="spacer" />
        <button className="btn small ghost" onClick={onRecheck}>Check again</button>
      </div>
      {findings.length > 0 && (
        <ul className="findings">
          {findings.map((f, i) => (
            <li key={i}>
              <button className={`finding ${f.severity}`} onClick={() => onFinding(f)}>
                <span className="finding-top">
                  <span className={`sev ${f.severity}`}>{SEVERITY_LABEL[f.severity]}</span>
                  <span className="mono dim">{f.code}</span>
                  {f.scene_id && <span className="dim">scene {f.scene_id}</span>}
                </span>
                <span className="finding-msg">{f.message}</span>
                {f.suggestion.text && <span className="finding-fix">Fix: {f.suggestion.text}</span>}
                {f.suggestion.path && <span className="mono dim finding-path">{f.suggestion.path}</span>}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
};

const Setup = (p: Props) => {
  const { options: o, defaults: d, onOptions } = p;
  const set = (patch: Partial<RenderOptions>) => onOptions({ ...o, ...patch });
  const engine = o.tts ?? d.tts;
  const ready = p.check.status === "done" || p.check.status === "failed";
  const blocked = p.check.status === "done" && blocksRender(p.check.report);
  const mode = AUTO_SUBTITLE_MODES.find((m) => m.value === o.autoSubtitles);

  return (
    <>
      <div className="render-head">
        <div className="render-title">Render</div>
        <span className="spacer" />
        <button className="btn small ghost" onClick={p.onDismiss} aria-label="Close">✕</button>
      </div>

      <div className="render-body">
        <label className="field">
          <span className="field-label">Voice engine</span>
          <select value={engine} onChange={(e) => set({ tts: e.target.value as RenderOptions["tts"] })}>
            {TTS_ENGINES.map((t) => <option key={t.value} value={t.value}>{t.label}</option>)}
          </select>
          <span className="hint dim">
            Default: {d.tts}{d.ttsFromEnv ? " (from $VIMMER_TTS)" : ""}. Kokoro falls back to say when it cannot run.
          </span>
        </label>

        <div className="field">
          <span className="field-label">Auto-subtitles</span>
          <div className="segmented" role="radiogroup" aria-label="Auto-subtitles">
            {AUTO_SUBTITLE_MODES.map((m) => (
              <button
                key={m.value}
                role="radio"
                aria-checked={o.autoSubtitles === m.value}
                className={`seg${o.autoSubtitles === m.value ? " active" : ""}`}
                onClick={() => set({ autoSubtitles: m.value })}
              >
                {m.label}
              </button>
            ))}
          </div>
          <span className="hint dim">{mode?.help}</span>
        </div>

        <label className="check">
          <input type="checkbox" checked={o.wordSubtitles} onChange={(e) => set({ wordSubtitles: e.target.checked })} />
          Word-by-word subtitle timing (whisper.cpp)
        </label>

        <details className="render-advanced">
          <summary>Advanced</summary>
          <div className="render-grid">
            <NumberField label="Chunk length (s)" value={o.chunkMs} placeholder={d.chunkMs} min={1000} scale={1000} onChange={(v) => set({ chunkMs: v })} />
            <NumberField label="Render jobs" value={o.renderJobs} placeholder={d.renderJobs} min={1} onChange={(v) => set({ renderJobs: v })} />
            <NumberField label="Image jobs" value={o.preprocessJobs} placeholder={d.preprocessJobs} min={1} onChange={(v) => set({ preprocessJobs: v })} />
            <label className="field">
              <span className="field-label">Recognition language</span>
              <input value={o.whisperLanguage} placeholder="guess" onChange={(e) => set({ whisperLanguage: e.target.value })} />
            </label>
          </div>
        </details>

        <Findings check={p.check} onRecheck={p.onRecheck} onFinding={p.onFinding} />
      </div>

      <div className="render-foot">
        <span className="spacer" />
        <button className="btn small ghost" onClick={p.onDismiss}>Cancel</button>
        {blocked ? (
          <button className="btn small danger" onClick={p.onRender}>Render anyway</button>
        ) : (
          <button className="btn small primary" disabled={!ready} onClick={p.onRender}>Render…</button>
        )}
      </div>
    </>
  );
};

// Floating card in the corner: options and the pre-render check, then progress, then the result.
export const RenderPanel = (props: Props) => {
  const { view, onCancel, onReveal, onDismiss } = props;
  if (view.status === "idle") return null;
  const pct = Math.round(fraction(view) * 100);

  return (
    <div className={`render-card glass${view.status === "setup" ? " setup" : ""}`} role="status" aria-live="polite">
      {view.status === "setup" && <Setup {...props} />}
      {view.status === "running" && (
        <>
          <div className="render-title">Rendering</div>
          <div className="render-stage dim">{view.stage}{view.chunks && view.chunks.total > 0 ? ` · segment ${Math.min(view.chunks.done + 1, view.chunks.total)} of ${view.chunks.total}` : ""}</div>
          <div className="render-bar" aria-label={`${pct} percent`}>
            <span style={{ width: `${pct}%` }} />
          </div>
          <div className="render-foot">
            <span className="mono dim">{pct}%</span>
            <span className="spacer" />
            <button className="btn small danger" onClick={onCancel}>Cancel</button>
          </div>
        </>
      )}
      {view.status === "done" && (
        <>
          <div className="render-title ok">Render complete</div>
          <div className="render-stage mono dim">{view.output}</div>
          <div className="render-foot">
            <button className="btn small" onClick={() => onReveal(view.output)}>Show in Finder</button>
            <span className="spacer" />
            <button className="btn small ghost" onClick={onDismiss}>Close</button>
          </div>
        </>
      )}
      {view.status === "cancelled" && (
        <>
          <div className="render-title">Render cancelled</div>
          <div className="render-foot">
            <span className="spacer" />
            <button className="btn small ghost" onClick={onDismiss}>Close</button>
          </div>
        </>
      )}
      {view.status === "failed" && (
        <>
          <div className="render-title bad">Render failed</div>
          <div className="render-stage">{view.error}</div>
          <div className="render-foot">
            <span className="spacer" />
            <button className="btn small ghost" onClick={onDismiss}>Close</button>
          </div>
        </>
      )}
    </div>
  );
};
