// The 「旁白」 tab: the voice-over track (旁白音轨). Add a clip from a file, record the whole
// voice-over with the teleprompter, align the scene cuts to it, and edit the selected clip.
import { useState, type Dispatch, type ReactNode } from "react";
import type { VideoTimeline } from "../../../src/types/timeline";
import type { Selection } from "../model";
import { timecode } from "../model";
import type { Action } from "../state";
import * as vo from "../voiceover";
import { CommitNumField, Field, MediaInput } from "./Inspector";

export type VoicePanelProps = {
  lengths: vo.Lengths;
  /** Adds a clip of `source` at the playhead; returns an error message, or null. */
  onAddClip: (source: string) => string | null;
  /** Opens the teleprompter to record the whole voice-over (录制整段旁白). */
  onRecordAll?: () => void;
  /** 按旁白对齐场景: proposes scene lengths from the voice-over. */
  alignControls?: ReactNode;
  /** 移到旁白音轨: the scene's own audio becomes a voice-over clip; returns an error, or null. */
  onMoveSceneAudio: (sceneIndex: number) => string | null;
  /** Per-scene recording (🎙) in the scene inspector. */
  sceneRecorder?: (sceneIndex: number) => ReactNode;
};

type Props = {
  timeline: VideoTimeline;
  selection: Selection;
  playheadMs: number;
  dispatch: Dispatch<Action>;
  onSelect: (s: Selection) => void;
  onSeekMs?: (ms: number) => void;
  voice: VoicePanelProps;
};

export const VoiceoverPanel = ({ timeline, selection, playheadMs, dispatch, onSelect, onSeekMs, voice }: Props) => {
  const clips = vo.clipsOf(timeline);
  const [source, setSource] = useState("");
  const [error, setError] = useState<string | null>(null);
  const selected = selection.kind === "voiceover" ? clips[selection.index] : undefined;
  const index = selection.kind === "voiceover" ? selection.index : -1;
  const { lengths } = voice;

  const add = () => {
    const err = voice.onAddClip(source.trim());
    setError(err);
    if (!err) setSource("");
  };

  return (
    <div className="form" data-voiceover-panel>
      <div className="section-title">旁白音轨 Voice-over</div>
      <p className="hint-block dim">
        旁白按整段视频的时间播放，不会在场景切换时被截断；背景音乐会在旁白时自动压低。
        A voice-over plays on the video's clock across scene cuts; the music ducks under it.
      </p>
      {voice.onRecordAll && (
        <button className="btn primary small" onClick={voice.onRecordAll} data-record-all>
          🎙 录制整段旁白（提词器）
        </button>
      )}
      <Field label="添加音频 Add a file" hint={`在播放头 ${timecode(playheadMs)} 处加入`}>
        <MediaInput kind="audio" value={source} placeholder="assets/recordings/voice.wav" onChange={setSource} />
      </Field>
      <div className="button-row">
        <button className="btn small" disabled={!source.trim()} onClick={add} data-add-voiceover>
          ＋ 加到旁白音轨
        </button>
      </div>
      {error && <span className="field-error" role="alert">{error}</span>}
      {voice.alignControls}

      <div className="section-title">片段 Clips <span className="count">{clips.length}</span></div>
      {clips.length === 0 ? (
        <p className="dim">还没有旁白。录一段，或从素材里拖一个音频到时间线的「旁白」轨道。</p>
      ) : (
        <ul className="vo-list">
          {clips.map((c, i) => (
            <li key={c.id}>
              <button
                className={i === index ? "vo-row active" : "vo-row"}
                onClick={() => {
                  onSelect({ kind: "voiceover", index: i });
                  onSeekMs?.(c.start_ms);
                }}
              >
                <span className="mono">{timecode(c.start_ms)}</span>
                <span className="vo-name">{c.source.split("/").pop()}</span>
                <span className="dim mono">{(vo.playedMs(c, lengths) / 1000).toFixed(1)} s{lengths[c.source] === undefined ? "?" : ""}</span>
              </button>
            </li>
          ))}
        </ul>
      )}

      {selected && (
        <>
          <div className="section-title">片段 {selected.id}</div>
          <Field label="文件 File">
            <span className="mono small-text">{selected.source}</span>
          </Field>
          <CommitNumField
            label="开始 Start"
            suffix="ms"
            hint={timecode(selected.start_ms)}
            value={selected.start_ms}
            step={100}
            min={0}
            validate={(v) => (Number.isInteger(v) && v >= 0 ? null : "A whole number, 0 or more.")}
            onCommit={(v) => dispatch({ type: "moveVoiceover", index, startMs: v, lengths })}
          />
          <CommitNumField
            label="剪掉开头 Trim start"
            suffix="ms"
            value={selected.trim_start_ms ?? 0}
            step={50}
            min={0}
            validate={(v) => (Number.isInteger(v) && v >= 0 ? null : "A whole number, 0 or more.")}
            onCommit={(v) => dispatch({ type: "trimVoiceover", index, edge: "start", trimMs: v, lengths })}
          />
          <CommitNumField
            label="剪掉结尾 Trim end"
            suffix="ms"
            value={selected.trim_end_ms ?? 0}
            step={50}
            min={0}
            validate={(v) => (Number.isInteger(v) && v >= 0 ? null : "A whole number, 0 or more.")}
            onCommit={(v) => dispatch({ type: "trimVoiceover", index, edge: "end", trimMs: v, lengths })}
          />
          <Field label="音量 Gain" hint={`×${(selected.gain ?? 1).toFixed(2)}`}>
            <input
              type="range"
              min={0}
              max={vo.MAX_VOICEOVER_GAIN}
              step={0.05}
              value={selected.gain ?? 1}
              onChange={(e) => dispatch({ type: "setVoiceoverGain", index, gain: Number(e.target.value) })}
              data-voiceover-gain
            />
          </Field>
          <div className="button-row">
            <button className="btn ghost small" onClick={() => onSeekMs?.(selected.start_ms)}>⏮ 跳到开头</button>
            <button className="btn ghost small danger" onClick={() => dispatch({ type: "deleteVoiceover", index })} data-delete-voiceover>
              删除片段
            </button>
          </div>
          <p className="dim small-text">
            时长 {(vo.playedMs(selected, lengths) / 1000).toFixed(2)} 秒（文件 {lengths[selected.source] !== undefined ? `${(lengths[selected.source] / 1000).toFixed(2)} 秒` : "长度未知"}），
            覆盖场景：{vo.scenesUnder(timeline, selected, lengths).map((i) => timeline.scenes[i].id).join("、") || "无"}
          </p>
        </>
      )}
    </div>
  );
};
