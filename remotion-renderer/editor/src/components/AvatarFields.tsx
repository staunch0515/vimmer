// 数字人讲解人 controls: the photo and look in the Brand tab, 「生成数字人」 with its status and
// progress, and the per-scene settings in the Scene tab. Logic in avatarUi.ts, desktop calls in
// avatar.ts; `useAvatar` (one per editor window) holds the engine state, the status and the job.
import { useCallback, useEffect, useRef, useState, type Dispatch } from "react";
import type { Avatar, LogoCorner, SceneAvatar, VideoTimeline } from "../../../src/types/timeline";
import type { Action } from "../state";
import type { CheckFinding } from "../renderOptions";
import {
  AVATAR_DEFAULTS,
  CORNERS,
  CORNER_LABELS,
  aboutTime,
  anySceneAvatar,
  defaultAvatar,
  needsGenerating,
  overTextFix,
  parseProgress,
  progressFraction,
  sceneAvatar,
  statusLines,
  type AvatarEnv,
  type AvatarProgress,
  type AvatarStatus,
} from "../avatarUi";
import { avatarEnv, avatarStatus, cancelAvatarGenerate, listenAvatar, probeFace, startAvatarGenerate, type ProbeResult } from "../avatar";
import { Field, MediaInput } from "./Inspector";

export type AvatarJob =
  | { status: "idle" }
  | { status: "running"; progress: AvatarProgress | null }
  | { status: "done" }
  | { status: "cancelled" }
  | { status: "failed"; error: string };

export type AvatarController = {
  env: AvatarEnv | null;
  status: AvatarStatus | null;
  job: AvatarJob;
  generate: () => void;
  cancel: () => void;
  refresh: () => void;
};

/**
 * The engine state (once), the clip status (after each edit, debounced) and the generation job.
 * `options` are the render panel's choices (the voice decides the speech). `onGenerated` runs
 * once after a successful generation (「先生成再渲染」 renders then).
 */
export const useAvatar = (timeline: VideoTimeline, text: string, options: () => unknown, onGenerated: () => void): AvatarController => {
  const [env, setEnv] = useState<AvatarEnv | null>(null);
  const [status, setStatus] = useState<AvatarStatus | null>(null);
  const [job, setJob] = useState<AvatarJob>({ status: "idle" });
  const [tick, setTick] = useState(0);
  const generatedRef = useRef(onGenerated);
  generatedRef.current = onGenerated;
  const has = anySceneAvatar(timeline);

  useEffect(() => void avatarEnv().then(setEnv), []);
  useEffect(() => {
    if (!has) {
      setStatus(null);
      return;
    }
    let live = true;
    const handle = window.setTimeout(() => void avatarStatus(text).then((s) => live && setStatus(s)), 400);
    return () => {
      live = false;
      window.clearTimeout(handle);
    };
  }, [text, has, tick]);
  useEffect(() => {
    let off: (() => void) | null = null;
    let live = true;
    void listenAvatar(
      (p) => p.kind === "stage" && setJob((j) => (j.status === "running" ? { status: "running", progress: parseProgress(j.progress, p.name) } : j)),
      (f) => {
        setJob(f.ok ? { status: "done" } : f.cancelled ? { status: "cancelled" } : { status: "failed", error: f.error ?? "unknown error" });
        setTick((t) => t + 1);
        if (f.ok) generatedRef.current();
      },
    ).then((u) => (live ? (off = u) : u()));
    return () => {
      live = false;
      off?.();
    };
  }, []);

  const generate = useCallback(() => {
    setJob({ status: "running", progress: null });
    startAvatarGenerate(text, options()).catch((e) => setJob({ status: "failed", error: String(e) }));
  }, [text, options]);
  const cancel = useCallback(() => void cancelAvatarGenerate(), []);
  const refresh = useCallback(() => setTick((t) => t + 1), []);
  return { env, status, job, generate, cancel, refresh };
};

const CornerPicker = ({ value, onChange, name, inherit }: { value: LogoCorner | undefined; onChange: (c: LogoCorner | undefined) => void; name: string; inherit?: string }) => (
  <div className="segmented" role="radiogroup" aria-label={name} data-avatar-corner={name}>
    {inherit !== undefined && (
      <button type="button" role="radio" aria-checked={value === undefined} className={`seg${value === undefined ? " active" : ""}`} onClick={() => onChange(undefined)}>
        {inherit}
      </button>
    )}
    {CORNERS.map((c) => (
      <button key={c} type="button" role="radio" aria-checked={value === c} className={`seg${value === c ? " active" : ""}`} data-corner={c} onClick={() => onChange(c)}>
        {CORNER_LABELS[c]}
      </button>
    ))}
  </div>
);

/** Face check of the chosen photo, run in the background whenever the photo changes. */
const useProbe = (photo: string): ProbeResult | "checking" | null => {
  const [result, setResult] = useState<ProbeResult | "checking" | null>(null);
  useEffect(() => {
    if (!photo) {
      setResult(null);
      return;
    }
    let live = true;
    setResult("checking");
    const handle = window.setTimeout(() => void probeFace(photo).then((r) => live && setResult(r)), 300);
    return () => {
      live = false;
      window.clearTimeout(handle);
    };
  }, [photo]);
  return result;
};

const ProbeLine = ({ photo }: { photo: string }) => {
  const r = useProbe(photo);
  if (!r) return null;
  if (r === "checking") return <div className="dim" data-avatar-probe="checking">正在检查照片里的人脸…</div>;
  if (!r.ok) return <div className="field-error" data-avatar-probe="error">检查人脸没有完成：{r.error}</div>;
  const cls = r.state === "found" ? "ok" : r.state === "no_face" || r.state === "missing" ? "bad" : "dim";
  return <div className={cls} data-avatar-probe={r.state}>{r.state === "no_face" ? "✗ 没找到清晰的正脸：请换一张" : r.zh}</div>;
};

/** 「生成数字人」: status per run, the button (or why it is off), progress and 取消. */
export const AvatarGenerate = ({ av, timeline }: { av: AvatarController; timeline: VideoTimeline }) => {
  if (!anySceneAvatar(timeline)) return null;
  const lines = statusLines(av.status);
  const ready = av.env?.state === "ready";
  const running = av.job.status === "running";
  const p = av.job.status === "running" ? av.job.progress : null;
  const pct = Math.round(progressFraction(p) * 100);
  const left = p?.text.match(/约 [^)）]+/)?.[0];
  const total = av.status?.ok ? av.status.missing_estimate_s : 0;
  return (
    <div className="subform" data-avatar-generate>
      {av.status && !av.status.ok && <div className="field-error">读不到数字人状态：{av.status.error}</div>}
      {lines.length > 0 && (
        <ul className="avatar-status" data-avatar-status>
          {lines.map((l, i) => (
            <li key={i} className={l.ready ? "ok" : "warn"}>
              {l.label}
              {l.scenes.length > 0 && <span className="dim mono"> {l.scenes.join(", ")}</span>}
            </li>
          ))}
        </ul>
      )}
      {running ? (
        <div data-avatar-progress>
          <div className="render-stage dim">{p ? `第 ${Math.max(p.run, 1)}/${Math.max(p.runs, 1)} 段 · ${p.total ? `${p.done}/${p.total} 帧` : p.text}` : "准备中…"}</div>
          <div className="render-bar" aria-label={`${pct} percent`}>
            <span style={{ width: `${pct}%` }} />
          </div>
          <div className="row">
            <span className="mono dim">{pct}% · {left ? `还要${left}` : total ? `共${aboutTime(total)}` : ""}</span>
            <span className="spacer" />
            <button className="btn small danger" onClick={av.cancel} data-avatar-cancel>取消</button>
          </div>
        </div>
      ) : (
        <>
          <button
            className="btn small primary"
            disabled={!ready || !needsGenerating(av.status)}
            title={ready ? undefined : av.env?.zh}
            onClick={av.generate}
            data-avatar-start
          >
            生成数字人{needsGenerating(av.status) ? `（${aboutTime(total)}）` : ""}
          </button>
          {!ready && av.env && <div className="hint dim" data-avatar-env={av.env.state}>{av.env.zh}</div>}
          {ready && !needsGenerating(av.status) && av.status?.ok && <div className="hint dim">都已生成：渲染会直接用。</div>}
        </>
      )}
      {av.job.status === "cancelled" && <div className="dim">已取消，没有留下半成品。</div>}
      {av.job.status === "failed" && <div className="field-error">生成失败：{av.job.error}</div>}
      {av.job.status === "done" && <div className="ok">✓ 数字人生成好了</div>}
    </div>
  );
};

/** The Brand tab's 讲解人 section: photo, look, generation. */
export const AvatarSection = ({ timeline, dispatch, av }: { timeline: VideoTimeline; dispatch: Dispatch<Action>; av: AvatarController }) => {
  const a = defaultAvatar(timeline);
  const set = (patch: Partial<Avatar> | null) => dispatch({ type: "setAvatar", patch });
  const fromBrand = !timeline.avatar && !!timeline.brand?.avatar;
  return (
    <div className="subform" data-avatar-section>
      <div className="section-title">数字人讲解人 Narrator</div>
      <div className="dim">角落里一个会说话的人像，嘴型跟着旁白动。{fromBrand ? "现在用的是品牌里的设置；改动会存到这个视频里。" : ""}</div>
      <Field label="讲解人照片" hint="PNG / JPEG">
        <MediaInput kind="image" value={a?.photo ?? ""} placeholder="assets/me.png" onChange={(v) => set(v ? { photo: v } : null)} />
      </Field>
      <div className="hint dim">一张清晰的正脸：一个人，头和肩膀，看镜头，不戴墨镜。</div>
      {a?.photo && <ProbeLine photo={a.photo} />}
      {a?.photo && (
        <>
          <Field label="位置 Corner">
            <CornerPicker name="default" value={a.corner ?? AVATAR_DEFAULTS.corner} onChange={(c) => set({ corner: c })} />
          </Field>
          <Field label="大小 Size" hint={`${Math.round((a.size ?? AVATAR_DEFAULTS.size) * 100)}% 画面高度`}>
            <input type="range" data-avatar-size min={0.1} max={0.6} step={0.01} value={a.size ?? AVATAR_DEFAULTS.size} onChange={(e) => set({ size: Number(e.target.value) })} />
          </Field>
          <Field label="形状 Shape">
            <div className="segmented" role="radiogroup" aria-label="Shape">
              {(["circle", "rounded"] as const).map((s) => (
                <button key={s} type="button" role="radio" aria-checked={(a.shape ?? "circle") === s} className={`seg${(a.shape ?? "circle") === s ? " active" : ""}`} data-avatar-shape={s} onClick={() => set({ shape: s })}>
                  {s === "circle" ? "圆形" : "圆角方形"}
                </button>
              ))}
            </div>
          </Field>
          <label className="check">
            <input type="checkbox" data-avatar-border checked={!!a.border} onChange={(e) => set({ border: e.target.checked ? { color: AVATAR_DEFAULTS.borderColor, width: AVATAR_DEFAULTS.borderWidth } : undefined })} />
            边框 Border
            {a.border && (
              <input type="color" aria-label="Border color" value={(a.border.color ?? AVATAR_DEFAULTS.borderColor).slice(0, 7).toLowerCase()} onChange={(e) => set({ border: { ...a.border, color: e.target.value.toUpperCase() } })} />
            )}
          </label>
          <details className="render-advanced">
            <summary>高级 Advanced</summary>
            <Field label="边距 Margin" hint="0.04 = 默认">
              <input type="number" data-avatar-margin min={0} max={0.3} step={0.01} value={a.margin ?? AVATAR_DEFAULTS.margin} onChange={(e) => set({ margin: Number(e.target.value) })} />
            </Field>
          </details>
          <div className="hint ok" data-avatar-look-note>改位置、大小、形状、边框不用重新生成。</div>
          <AvatarGenerate av={av} timeline={timeline} />
          <button className="btn ghost small" onClick={() => set(null)} data-avatar-remove>去掉讲解人</button>
        </>
      )}
    </div>
  );
};

/** The Scene tab: 「本场景显示讲解人」, its corner, another photo, and fixes for covered text. */
export const SceneAvatarFields = ({
  timeline,
  index,
  dispatch,
  findings,
  onCheck,
}: {
  timeline: VideoTimeline;
  index: number;
  dispatch: Dispatch<Action>;
  /** The latest check's findings (the over-text fixes of this scene are offered here). */
  findings: CheckFinding[];
  onCheck?: () => void;
}) => {
  const scene = timeline.scenes[index];
  const def = defaultAvatar(timeline);
  if (!scene || (!def && !scene.avatar)) return null;
  const own = scene.avatar ?? {};
  const shown = sceneAvatar(timeline, index);
  const set = (patch: Partial<SceneAvatar> | null) => dispatch({ type: "setSceneAvatar", index, patch });
  const fixes = findings.map(overTextFix).filter((f): f is NonNullable<typeof f> => !!f && f.index === index);
  return (
    <div className="subform" data-scene-avatar>
      <div className="section-title">数字人讲解人</div>
      <label className="check">
        <input type="checkbox" data-scene-avatar-toggle checked={shown !== null} onChange={(e) => set({ enabled: e.target.checked })} />
        本场景显示讲解人
      </label>
      {shown && (
        <>
          <Field label="位置 Corner">
            <CornerPicker name="scene" value={own.corner} inherit={`默认（${CORNER_LABELS[def?.corner ?? AVATAR_DEFAULTS.corner]}）`} onChange={(c) => set({ corner: c })} />
          </Field>
          <Field label="换一张照片" hint="留空：用默认照片（要重新生成）">
            <MediaInput kind="image" value={own.photo ?? ""} placeholder={def?.photo ?? "assets/other.png"} onChange={(v) => set({ photo: v || undefined })} />
          </Field>
          {fixes.map((f) => (
            <div key={f.corner} className="row warn" data-avatar-fix={f.corner}>
              <span>讲解人挡住了文字</span>
              <span className="spacer" />
              <button className="btn small" onClick={() => set({ corner: f.corner })}>移到{CORNER_LABELS[f.corner]}角</button>
            </div>
          ))}
          {onCheck && fixes.length === 0 && (
            <button className="btn ghost small" onClick={onCheck} data-avatar-check>检查是否挡住文字</button>
          )}
        </>
      )}
      {(scene.avatar && Object.keys(scene.avatar).length > 0) && (
        <button className="btn ghost small" onClick={() => set(null)}>恢复默认</button>
      )}
    </div>
  );
};
