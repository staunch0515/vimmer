import { useEffect, useRef, useState } from "react";
import type { Scene } from "../../../src/types/timeline";
import { clipFor, lengthHint, type HeardClips, type ListenClip, type TtsChoice } from "../narration";
import { listenNarration } from "../listen";

/** What the scene inspector needs to let the user hear a scene's narration. */
export type ListenProps = {
  /** The engine the render panel has chosen (or the render's default). */
  engine: TtsChoice;
  heard: HeardClips;
  onHeard: (engine: TtsChoice, clips: ListenClip[]) => void;
  /** The whole timeline as the editor holds it (the render expands templates first). */
  timelineJson: () => string;
  /** URL of a public-relative file. */
  audioUrl: (path: string) => string;
};

type Status =
  | { kind: "idle" }
  | { kind: "busy" }
  | { kind: "playing" }
  | { kind: "error"; message: string };

/**
 * 试听 Listen: synthesizes this scene's spoken text with the chosen voice and engine (the render's
 * code and cache) and plays it, with how long the speech is against the scene. Mounted per scene:
 * switching scenes unmounts it, which stops the sound.
 */
export const ListenButton = ({ scene, listen }: { scene: Scene; listen: ListenProps }) => {
  const [status, setStatus] = useState<Status>({ kind: "idle" });
  const [notes, setNotes] = useState<string[]>([]);
  const audio = useRef<HTMLAudioElement | null>(null);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      audio.current?.pause();
      audio.current = null;
    };
  }, []);
  const clip = clipFor(listen.heard, scene, listen.engine);
  // Editing the text or voice gives a new clip: stop the old one.
  const key = clip?.source ?? null;
  useEffect(() => {
    if (audio.current && audio.current.dataset.source !== key) {
      audio.current.pause();
      audio.current = null;
      setStatus((s) => (s.kind === "playing" ? { kind: "idle" } : s));
    }
  }, [key]);

  const stop = () => {
    audio.current?.pause();
    audio.current = null;
    setStatus({ kind: "idle" });
  };

  const play = (c: ListenClip) => {
    audio.current?.pause();
    const el = new Audio(listen.audioUrl(c.source));
    el.dataset.source = c.source;
    el.dataset.listen = scene.id;
    el.onended = () => alive.current && audio.current === el && setStatus({ kind: "idle" });
    el.onerror = () =>
      alive.current && audio.current === el && setStatus({ kind: "error", message: `Could not play ${c.source}.` });
    audio.current = el;
    setStatus({ kind: "playing" });
    void el.play().catch((e: unknown) => {
      if (alive.current && audio.current === el) setStatus({ kind: "error", message: `Could not play: ${String(e)}` });
    });
  };

  const onClick = async () => {
    if (status.kind === "playing") return stop();
    if (status.kind === "busy") return;
    setStatus({ kind: "busy" });
    const engine = listen.engine;
    const result = await listenNarration(listen.timelineJson(), [scene.id], engine);
    if (!alive.current) return;
    if (!result.ok) {
      setStatus({ kind: "error", message: result.error });
      return;
    }
    listen.onHeard(engine, result.clips);
    setNotes(result.warnings);
    const made = result.clips.find((c) => c.scene_id === scene.id);
    if (made) play(made);
    else setStatus({ kind: "error", message: "No narration came back for this scene." });
  };

  const hint = clip ? lengthHint(scene.duration_ms, clip.speech_ms) : null;
  return (
    <div className="listen" data-listen-scene={scene.id}>
      <div className="button-row">
        <button
          className="btn small"
          data-listen
          disabled={!!scene.audio}
          title={scene.audio ? "Manual audio is used instead of narration" : `Synthesize with ${listen.engine} and play`}
          onClick={() => void onClick()}
        >
          {status.kind === "busy" ? (
            <>
              <span className="spinner" aria-hidden /> 合成中…
            </>
          ) : status.kind === "playing" ? (
            "■ 停止 Stop"
          ) : (
            "▶ 试听 Listen"
          )}
        </button>
        <span className="dim small">{listen.engine === "kokoro" ? "Kokoro" : "say"}</span>
      </div>
      {hint && (
        <div className={hint.stretched ? "listen-hint stretched" : "listen-hint"} data-listen-hint role="status">
          {hint.text}
        </div>
      )}
      {status.kind === "error" && (
        <div className="listen-hint error" role="alert">
          {status.message}
        </div>
      )}
      {notes.map((n) => (
        <div key={n} className="listen-hint note">
          {n}
        </div>
      ))}
    </div>
  );
};
