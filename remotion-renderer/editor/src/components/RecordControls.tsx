// Recording (录音) UI shared by the scene recorder and the teleprompter: countdown, level meter,
// timer, stop, and after a take: play back, keep, re-record, discard.
import type { Recorder } from "../recorder";
import { meterFill } from "../recording";

const secs = (ms: number) => `${Math.floor(ms / 60000)}:${String(Math.floor((ms % 60000) / 1000)).padStart(2, "0")}.${Math.floor((ms % 1000) / 100)}`;

export const LevelMeter = ({ db }: { db: number }) => {
  const fill = meterFill(db);
  const hot = Number.isFinite(db) && db > -3;
  return (
    <div className="rec-meter" role="meter" aria-label="输入音量 Input level" aria-valuemin={-60} aria-valuemax={0} aria-valuenow={Number.isFinite(db) ? Math.round(db) : -60}>
      <div className={hot ? "rec-meter-fill hot" : "rec-meter-fill"} style={{ width: `${fill * 100}%` }} />
    </div>
  );
};

type Props = {
  rec: Recorder;
  startLabel: string;
  /** Keeps the take; returns an error, or null. */
  onKeep: () => Promise<string | null> | string | null;
  keepLabel: string;
  saving?: boolean;
  /** Shown under a finished take (a length hint). */
  takeNote?: React.ReactNode;
  saveError?: string | null;
};

export const RecordControls = ({ rec, startLabel, onKeep, keepLabel, saving, takeNote, saveError }: Props) => {
  const { phase } = rec;
  return (
    <div className="rec-controls" data-rec-phase={phase}>
      {(phase === "idle" || phase === "error") && (
        <button className="btn primary small" onClick={rec.start} data-rec-start>
          🎙 {startLabel}
        </button>
      )}
      {phase === "asking" && <span className="dim">正在打开麦克风… (第一次会请你允许使用麦克风)</span>}
      {phase === "countdown" && (
        <div className="rec-live">
          <span className="rec-count" aria-live="assertive" data-rec-count>{rec.count}</span>
          <LevelMeter db={rec.peakDb} />
          <button className="btn ghost small" onClick={rec.stop}>取消</button>
        </div>
      )}
      {phase === "recording" && (
        <div className="rec-live">
          <span className="rec-dot" aria-hidden />
          <span className="mono" data-rec-elapsed>{secs(rec.elapsedMs)}</span>
          <LevelMeter db={rec.peakDb} />
          <button className="btn danger small" onClick={rec.stop} data-rec-stop>■ 停止</button>
        </div>
      )}
      {phase === "stopped" && rec.take && (
        <div className="rec-take">
          <audio controls src={rec.take.url} data-rec-playback />
          <span className="dim small-text">这一条 {(rec.take.ms / 1000).toFixed(1)} 秒</span>
          {takeNote}
          <div className="button-row">
            <button className="btn primary small" disabled={saving} onClick={() => void onKeep()} data-rec-keep>
              {saving ? "保存中…" : keepLabel}
            </button>
            <button className="btn ghost small" disabled={saving} onClick={rec.start} data-rec-again>重录</button>
            <button className="btn ghost small" disabled={saving} onClick={rec.discard} data-rec-discard>丢弃</button>
          </div>
        </div>
      )}
      {rec.error && <span className="field-error" role="alert">{rec.error}</span>}
      {saveError && <span className="field-error" role="alert">{saveError}</span>}
    </div>
  );
};
