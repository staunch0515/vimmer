// 提词器 (teleprompter): recording the whole voice-over in one take. Each scene's script (its
// spoken line, else its text) is shown large, in order; while recording, the preview's playhead
// follows the take and the scene being read is highlighted. Keeping the take puts it on the
// voice-over track at the chosen start, one undo step.
import { useEffect, useMemo, useRef, useState, type Dispatch } from "react";
import type { VideoTimeline } from "../../../src/types/timeline";
import type { Action } from "../state";
import { timecode } from "../model";
import { useRecorder, saveTake } from "../recorder";
import { currentLine, prompterLines } from "../recording";
import * as vo from "../voiceover";
import { RecordControls } from "./RecordControls";

type Props = {
  timeline: VideoTimeline;
  playheadMs: number;
  lengths: vo.Lengths;
  dispatch: Dispatch<Action>;
  onSeekMs: (ms: number) => void;
  onClose: () => void;
};

export const Teleprompter = ({ timeline, playheadMs, lengths, dispatch, onSeekMs, onClose }: Props) => {
  const [fromPlayhead, setFromPlayhead] = useState(playheadMs > 0);
  // Fixed when a take starts: the playhead moves with the take, the start must not.
  const [fixedStart, setFixedStart] = useState<number | null>(null);
  const startMs = fixedStart ?? (fromPlayhead ? playheadMs : 0);
  const [elapsed, setElapsed] = useState(0);
  const recorder = useRecorder((ms) => {
    setElapsed(ms);
    onSeekMs(startMs + ms);
  });
  const rec = {
    ...recorder,
    start: () => {
      setFixedStart(startMs);
      onSeekMs(startMs);
      recorder.start();
    },
    discard: () => {
      setFixedStart(null);
      recorder.discard();
    },
  };
  const lines = useMemo(() => prompterLines(timeline, startMs), [timeline, startMs]);
  const live = rec.phase === "recording" || rec.phase === "stopped";
  const current = live ? currentLine(lines, startMs, rec.phase === "stopped" ? 0 : elapsed) : 0;
  const listRef = useRef<HTMLOListElement>(null);
  useEffect(() => {
    listRef.current?.querySelector<HTMLElement>(`[data-line="${current}"]`)?.scrollIntoView({ block: "center", behavior: "smooth" });
  }, [current]);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [replace, setReplace] = useState(false);
  const take = rec.take;
  const clashes = take ? vo.clipsOf(timeline).filter((c) => c.start_ms < startMs + take.ms && vo.endMs(c, lengths) > startMs) : [];
  const total = timeline.scenes.reduce((n, s) => n + s.duration_ms, 0);

  const keep = async () => {
    if (!take) return null;
    if (clashes.length && !replace) {
      const msg = "这一条和旁白音轨上已有的片段重叠。勾选「替换重叠的片段」，或换一个开始位置。";
      setSaveError(msg);
      return msg;
    }
    setSaving(true);
    setSaveError(null);
    const saved = await saveTake("voiceover", take.wav);
    setSaving(false);
    if ("error" in saved) {
      setSaveError(saved.error);
      return saved.error;
    }
    const placed = vo.placeTake(timeline, saved.path, startMs, take.ms, { ...lengths, [saved.path]: take.ms }, replace);
    if ("error" in placed) {
      setSaveError(placed.error);
      return placed.error;
    }
    dispatch({ type: "commitTimeline", timeline: placed.timeline, selection: { kind: "voiceover", index: placed.index } });
    rec.discard();
    onClose();
    return null;
  };

  return (
    <div className="prompter-layer" role="dialog" aria-label="提词器 Teleprompter" data-teleprompter>
      <div className="prompter">
        <div className="prompter-head">
          <strong>🎙 录制整段旁白 · 提词器</strong>
          <span className="spacer" />
          <button className="btn ghost small" onClick={() => { rec.discard(); onClose(); }} data-prompter-close>关闭</button>
        </div>
        <div className="prompter-settings">
          <label className="check">
            <input type="radio" name="from" checked={!fromPlayhead} disabled={rec.phase !== "idle"} onChange={() => setFromPlayhead(false)} />
            <span>从头开始 0:00</span>
          </label>
          <label className="check">
            <input type="radio" name="from" checked={fromPlayhead} disabled={rec.phase !== "idle" || playheadMs <= 0} onChange={() => setFromPlayhead(true)} />
            <span>从播放头 {timecode(fixedStart ?? playheadMs)}</span>
          </label>
          <span className="dim small-text">视频共 {(total / 1000).toFixed(1)} 秒。录音时预览会跟着走（不放声音，以免录进去）。</span>
        </div>
        <ol className="prompter-lines" ref={listRef}>
          {lines.map((l, k) => (
            <li key={l.index} data-line={k} className={live && k === current ? "prompter-line current" : k < current && live ? "prompter-line done" : "prompter-line"}>
              <span className="prompter-time mono">{timecode(Math.max(l.startMs, startMs))}</span>
              <span className="prompter-scene dim">{l.id}</span>
              <span className="prompter-text">{l.text || <span className="dim">（这一幕没有文字）</span>}</span>
            </li>
          ))}
        </ol>
        <RecordControls
          rec={rec}
          startLabel={`开始录音（从 ${timecode(startMs)}，倒数 3 秒）`}
          keepLabel="保留到旁白音轨"
          onKeep={keep}
          saving={saving}
          saveError={saveError}
          takeNote={
            take && clashes.length > 0 ? (
              <label className="check small-text">
                <input type="checkbox" checked={replace} onChange={(e) => setReplace(e.target.checked)} data-prompter-replace />
                <span>替换重叠的片段：{clashes.map((c) => c.id).join("、")}</span>
              </label>
            ) : take && startMs + take.ms > total ? (
              <span className="dim small-text">比视频长 {((startMs + take.ms - total) / 1000).toFixed(1)} 秒，结尾会被截掉（可以拉长最后一幕或剪掉结尾）。</span>
            ) : null
          }
        />
      </div>
    </div>
  );
};
