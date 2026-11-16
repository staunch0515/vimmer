import type { Scene } from "../../../src/types/timeline";
import { timecode } from "../model";
import { secondsPair, type NarratedPreview } from "../narration";

type Props = {
  /** The narrated preview while it is on, else null. */
  narrated: NarratedPreview<{ scenes: Scene[] }> | null;
  /** Length of the saved timeline. */
  savedMs: number;
  /** Some scene has narration to hear. */
  narratable: boolean;
  status: { busy: boolean; error: string | null; warnings: string[] };
  onListenAll: () => void;
  onOff: () => void;
};

/**
 * 试听全部 Listen to all, under the preview: synthesizes every narrated scene, then the Player
 * plays the video with the narration, each scene as long as the render will make it.
 */
export const NarrationBar = ({ narrated, savedMs, narratable, status, onListenAll, onOff }: Props) => {
  const previewMs = narrated ? narrated.timeline.scenes.reduce((sum, s) => sum + s.duration_ms, 0) : savedMs;
  const shown = narrated?.stretched.slice(0, 3).map((s) => {
      const [from, to] = secondsPair(s.from, s.to);
      return `${s.id} ${from}→${to} 秒`;
    }).join("，") ?? "";
  const more = narrated && narrated.stretched.length > 3 ? ` 等 ${narrated.stretched.length} 个` : "";
  return (
    <div className={narrated ? "narration-bar on" : "narration-bar"} data-narration-bar role="status">
      <button
        className="btn small"
        data-listen-all
        disabled={status.busy || !narratable}
        title={narratable ? "Synthesize every narrated scene and play the preview with it" : "No scene has narration"}
        onClick={onListenAll}
      >
        {status.busy ? (
          <>
            <span className="spinner" aria-hidden /> 正在合成旁白…
          </>
        ) : narrated ? (
          "🔊 重新试听全部"
        ) : (
          "🔊 试听全部 Listen to all"
        )}
      </button>
      {narrated && (
        <button className="btn ghost small" data-listen-off onClick={onOff}>
          关闭旁白 Narration off
        </button>
      )}
      {!narrated && !status.busy && !status.error && (
        <span className="dim">旁白平时只在渲染时合成；点「试听全部」可边看边听</span>
      )}
      {narrated &&
        (narrated.stretched.length > 0 ? (
          <span className="warn" data-narration-stretched>
            预览已按渲染时长播放：{shown}
            {more} 会被拉长以念完旁白，总长 {timecode(previewMs)}（时间线上 {timecode(savedMs)}）
          </span>
        ) : (
          <span>预览含旁白，时长与渲染一致（{timecode(previewMs)}）</span>
        ))}
      {narrated && narrated.missing.length > 0 && (
        <span className="warn" data-narration-missing>
          {narrated.missing.length} 个场景的旁白改过或还没合成（{narrated.missing.slice(0, 3).join("，")}），预览里它们暂时无声；再点「试听全部」更新
        </span>
      )}
      {status.error && <span className="error">{status.error}</span>}
      {status.warnings.map((w) => (
        <span key={w} className="warn">
          {w}
        </span>
      ))}
    </div>
  );
};
