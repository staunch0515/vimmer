import { useCallback, useEffect, useState } from "react";
import { errorText, useStudioApi } from "./api";
import { formatBytes, formatDuration, formatWhen } from "./format";
import { Modal } from "./Modal";
import type { ProjectSummary, RenderRecord } from "./types";

type Props = {
  dir: string;
  /** Bumped by the owner when a render finished. */
  refreshKey?: number;
  /** The project's facts changed (render count, thumbnail). */
  onProjectChanged?: (project: ProjectSummary) => void;
};

/** `Kokoro · auto-subtitles on`, from the options the render was started with. */
export const optionsLabel = (options: RenderRecord["options"]): string => {
  if (!options) return "";
  const parts: string[] = [];
  if (typeof options.tts === "string") parts.push(options.tts === "kokoro" ? "Kokoro" : options.tts);
  if (typeof options.autoSubtitles === "string" && options.autoSubtitles !== "off") parts.push(`auto-subtitles ${options.autoSubtitles}`);
  if (options.wordSubtitles === false) parts.push("no word timing");
  return parts.join(" · ");
};

const clock = (ms: number): string => {
  const d = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
};

export const RendersPanel = ({ dir, refreshKey = 0, onProjectChanged }: Props) => {
  const api = useStudioApi();
  const [renders, setRenders] = useState<RenderRecord[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [playing, setPlaying] = useState<RenderRecord | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    try {
      setRenders(await api.listRenders(dir));
    } catch (e) {
      setError(errorText(e));
      setRenders((r) => r ?? []);
    }
  }, [api, dir]);
  useEffect(() => {
    setRenders(null);
    setError(null);
    void load();
  }, [load, refreshKey]);

  const run = async (work: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await work();
    } catch (e) {
      setError(errorText(e));
    }
    setBusy(false);
  };
  const remove = (r: RenderRecord) =>
    run(async () => {
      const inProject = !r.file.startsWith("/");
      const yes = await api.confirm(
        r.missing
          ? "Remove this entry from the history? Its file is already gone."
          : inProject
            ? `Move this render (${formatBytes(r.bytes)}) to the Trash?`
            : `Remove this render from the history? The file stays at ${r.path}.`,
        r.missing || !inProject ? "Remove" : "Move to Trash",
      );
      if (!yes) return;
      const changed = await api.trashRender(dir, r.id);
      setRenders(changed.renders);
      onProjectChanged?.(changed.project);
    });
  const addExisting = () =>
    run(async () => {
      const file = await api.pickVideoFile();
      if (!file) return;
      const changed = await api.addExistingRender(dir, file);
      setRenders(changed.renders);
      onProjectChanged?.(changed.project);
    });

  const now = Date.now();
  return (
    <div className="st-renders">
      <div className="st-panel-head">
        <span className="section-label">Renders</span>
        <span className="dim">{renders ? renders.length : ""}</span>
        <span className="spacer" />
        <button className="btn small" disabled={busy} onClick={() => void addExisting()} title="Copy a video you already have into this project's history">
          Add video…
        </button>
      </div>
      {error && (
        <div className="st-inline bad" role="alert">
          {error}
        </div>
      )}
      <div className="st-asset-scroll">
        {renders === null ? (
          <p className="dim st-pad">Loading…</p>
        ) : renders.length === 0 ? (
          <div className="st-drop-hint">
            <p>No renders yet</p>
            <p className="dim">Videos you render from this project are kept here, newest first.</p>
          </div>
        ) : (
          <ul className="st-render-list">
            {renders.map((r, i) => (
              <li key={r.id} className={`st-render${r.missing ? " missing" : ""}`}>
                <button
                  className="st-render-play"
                  disabled={r.missing}
                  onClick={() => setPlaying(r)}
                  aria-label={`Play the render of ${clock(r.finishedMs)}`}
                  title={r.missing ? "The file is no longer there" : "Play"}
                >
                  ▶
                </button>
                <div className="st-render-text">
                  <div className="st-render-when" title={clock(r.finishedMs)}>
                    {formatWhen(r.finishedMs, now)}
                    {i === 0 && <span className="st-tag ok">Latest</span>}
                    {r.imported && <span className="st-tag">Added</span>}
                    {r.missing && <span className="st-tag bad">File missing</span>}
                  </div>
                  <div className="dim mono">
                    {formatDuration(r.durationMs)} · {formatBytes(r.bytes)}
                  </div>
                  {optionsLabel(r.options) && <div className="dim">{optionsLabel(r.options)}</div>}
                  <div className="st-actions">
                    <button className="btn ghost small" disabled={r.missing} onClick={() => void run(() => api.reveal(r.path))}>
                      Show in Finder
                    </button>
                    <button className="btn ghost small danger" disabled={busy} onClick={() => void remove(r)}>
                      Delete
                    </button>
                  </div>
                </div>
              </li>
            ))}
          </ul>
        )}
      </div>

      {playing && (
        <Modal title={`Render of ${clock(playing.finishedMs)}`} onClose={() => setPlaying(null)} wide>
          <video className="st-player" src={api.fileUrl(playing.path, playing.finishedMs)} controls autoPlay />
          <div className="st-modal-foot">
            <span className="dim mono">
              {formatDuration(playing.durationMs)} · {formatBytes(playing.bytes)} · {playing.file}
            </span>
            <span className="spacer" />
            <button className="btn" onClick={() => void run(() => api.reveal(playing.path))}>
              Show in Finder
            </button>
          </div>
        </Modal>
      )}
    </div>
  );
};
