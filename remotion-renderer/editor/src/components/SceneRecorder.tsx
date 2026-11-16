// 🎙 Recording one scene (录这一幕): the scene's script to read, a take, and keeping it as the
// scene's own audio (本幕音频). Keeping is one undo step; a take longer than the scene can lengthen
// the scene in the same step, so nothing is cut at the scene end.
import { useState, type Dispatch } from "react";
import type { Scene } from "../../../src/types/timeline";
import type { Action } from "../state";
import { useRecorder, saveTake } from "../recorder";
import { scriptOf } from "../recording";
import { RecordControls } from "./RecordControls";

/** Pause after the take before the scene ends (the narration step's 300 ms tail). */
const TAIL_MS = 300;

export const SceneRecorder = ({ scene, index, dispatch }: { scene: Scene; index: number; dispatch: Dispatch<Action> }) => {
  const [open, setOpen] = useState(false);
  const rec = useRecorder();
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [stretch, setStretch] = useState(true);
  const script = scriptOf(scene);
  if (!open) {
    return (
      <button className="btn small" onClick={() => setOpen(true)} data-scene-record>
        🎙 录这一幕 Record this scene
      </button>
    );
  }
  const take = rec.take;
  const needed = take ? take.ms + TAIL_MS : 0;
  const longer = take !== null && needed > scene.duration_ms;
  const keep = async () => {
    if (!take) return null;
    setSaving(true);
    setSaveError(null);
    const saved = await saveTake(scene.id, take.wav);
    setSaving(false);
    if ("error" in saved) {
      setSaveError(saved.error);
      return saved.error;
    }
    dispatch({
      type: "patchScene",
      index,
      patch: { audio: { source: saved.path, offset_ms: 0 }, ...(longer && stretch ? { duration_ms: needed } : {}) },
    });
    rec.discard();
    setOpen(false);
    return null;
  };
  return (
    <div className="rec-panel" data-scene-recorder>
      <div className="rec-head">
        <strong>🎙 录这一幕</strong>
        <button className="btn ghost small" onClick={() => { rec.discard(); setOpen(false); }}>关闭</button>
      </div>
      <p className={script ? "rec-script" : "rec-script dim"}>{script || "这一幕没有文字。可以直接说，或者先写好台词。"}</p>
      <RecordControls
        rec={rec}
        startLabel="开始录音（倒数 3 秒）"
        keepLabel="保留，作为本幕音频"
        onKeep={keep}
        saving={saving}
        saveError={saveError}
        takeNote={
          longer ? (
            <label className="check small-text">
              <input type="checkbox" checked={stretch} onChange={(e) => setStretch(e.target.checked)} />
              <span>
                比这一幕长（{(scene.duration_ms / 1000).toFixed(1)} 秒），会被截断；同时把这一幕拉长到 {(needed / 1000).toFixed(1)} 秒
              </span>
            </label>
          ) : null
        }
      />
      {scene.audio && <p className="dim small-text">保留后会替换现在的本幕音频（可撤销）。</p>}
    </div>
  );
};
