// 「按旁白对齐场景」: listens to a voice-over clip (local whisper), shows the proposed scene lengths
// old → new, and applies them as one undo step. Scenes outside the clip are never changed.
import { useState, type Dispatch } from "react";
import type { VideoTimeline } from "../../../src/types/timeline";
import type { Action } from "../state";
import { alignScenes, type AlignResult } from "../align";
import { clipsOf } from "../voiceover";

const s = (ms: number) => `${(ms / 1000).toFixed(2)} 秒`;

export const AlignScenes = ({ timeline, dispatch, clipId }: { timeline: VideoTimeline; dispatch: Dispatch<Action>; clipId: string | null }) => {
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<(AlignResult & { ok: true }) | null>(null);
  const [error, setError] = useState<string | null>(null);
  const clips = clipsOf(timeline);
  const clip = clips.find((c) => c.id === clipId) ?? clips[0];
  if (!clip) return null;
  const run = async () => {
    setBusy(true);
    setError(null);
    setResult(null);
    const got = await alignScenes(JSON.stringify(timeline), clip.id);
    setBusy(false);
    if (got.ok) setResult(got);
    else setError(got.error);
  };
  const ids = timeline.scenes.map((sc) => sc.id);
  const stale = result !== null && (result.durations.length !== ids.length || result.changes.some((c) => ids[c.index] !== c.id));
  const changed = result?.changes.filter((c) => c.new_ms !== c.old_ms) ?? [];
  return (
    <div className="align-box" data-align>
      <button className="btn small" disabled={busy} onClick={() => void run()} data-align-run>
        {busy ? "正在听旁白…（可能要一分钟）" : `按旁白对齐场景（${clip.id}）`}
      </button>
      <span className="dim small-text">按旁白里每句话的位置，建议各场景的长度，让切换落在句子之间。</span>
      {error && <span className="field-error" role="alert">{error}</span>}
      {result && (
        <div className="align-result">
          <table className="align-table">
            <thead>
              <tr><th>场景</th><th>现在</th><th>建议</th><th>这句话</th></tr>
            </thead>
            <tbody>
              {result.changes.map((c) => (
                <tr key={c.index} className={c.new_ms === c.old_ms ? "dim" : ""} data-align-row={c.id}>
                  <td>{c.id}</td>
                  <td className="mono">{s(c.old_ms)}</td>
                  <td className="mono">{c.new_ms === c.old_ms ? "不变" : s(c.new_ms)}</td>
                  <td className="mono dim">{c.said_from_ms === null ? "没听到" : `${s(c.said_from_ms)}–${s(c.said_to_ms ?? c.said_from_ms)}`}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {result.notes.map((n) => <span key={n} className="dim small-text">{n}</span>)}
          {stale && <span className="field-error">场景已经改过了，请重新对齐。</span>}
          <div className="button-row">
            <button
              className="btn primary small"
              disabled={stale || changed.length === 0}
              onClick={() => {
                dispatch({ type: "setSceneDurations", durations: result.durations, ids });
                setResult(null);
              }}
              data-align-apply
            >
              {changed.length === 0 ? "已经对齐了" : `应用（改 ${changed.length} 个场景，可撤销）`}
            </button>
            <button className="btn ghost small" onClick={() => setResult(null)}>取消</button>
          </div>
        </div>
      )}
    </div>
  );
};
