import { useEffect, useMemo, useRef, useState } from "react";
import { errorText, useStudioApi } from "./api";
import { formatCost, formatUsd } from "./format";
import { Modal } from "./Modal";
import type { Estimate, GenerateArgs, GenerateFinished, GenerateModel, GenerateProgress, IllustrationDetail, Lang, Orientation } from "./types";

type Props = {
  onClose: () => void;
  /** The generated project should be opened in the editor. */
  onOpenProject: (dir: string) => void;
};

// form → confirm (the estimate, one more click) → running → done | failed.
type Step = "form" | "confirm" | "running" | "done" | "failed";

const MODELS: { value: GenerateModel; label: string; note: string }[] = [
  { value: "claude-opus-5-5", label: "Opus 5.5", note: "best storyboards" },
  { value: "claude-sonnet-5-5", label: "Sonnet 5.5", note: "about half the price" },
];

const DETAILS: { value: IllustrationDetail; label: string; note: string }[] = [
  { value: "rich", label: "Rich", note: "Layered, lit pictures with depth, glow and texture." },
  { value: "simple", label: "Simple", note: "Flat pictures with a few shapes; about a third of the cost." },
];

const STAGES: { key: GenerateProgress["stage"]; label: string }[] = [
  { key: "storyboard", label: "Writing the storyboard" },
  { key: "illustrations", label: "Drawing illustrations" },
  { key: "project", label: "Creating the project" },
];

/** The language a text is most likely in: kana means Japanese, Han without kana Chinese. */
export const guessLang = (text: string): Lang => {
  if (/[぀-ヿ]/.test(text)) return "ja";
  if (/[一-鿿]/.test(text)) return "zh";
  return "en";
};

const NO_KEY_HELP = "Add  export ANTHROPIC_API_KEY=…  to ~/.zshrc, then try again. The key is read when a generation starts and is never stored by the app.";

export const GenerateDialog = ({ onClose, onOpenProject }: Props) => {
  const api = useStudioApi();
  const [step, setStep] = useState<Step>("form");
  const [text, setText] = useState("");
  const [lang, setLang] = useState<Lang | null>(null);
  const [orientation, setOrientation] = useState<Orientation>("landscape");
  const [target, setTarget] = useState("");
  const [model, setModel] = useState<GenerateModel>("claude-opus-5-5");
  const [illustrate, setIllustrate] = useState(false);
  const [detail, setDetail] = useState<IllustrationDetail>("rich");
  const [name, setName] = useState("");
  const [estimate, setEstimate] = useState<Estimate | null>(null);
  const [estimateError, setEstimateError] = useState<string | null>(null);
  const [progress, setProgress] = useState<GenerateProgress[]>([]);
  const [result, setResult] = useState<GenerateFinished | null>(null);
  const [startError, setStartError] = useState<string | null>(null);
  const [startedAt, setStartedAt] = useState(0);
  const [elapsed, setElapsed] = useState(0);
  const [cancelling, setCancelling] = useState(false);

  const targetSeconds = target.trim() === "" ? null : Number(target);
  const targetBad = targetSeconds !== null && !(Number.isInteger(targetSeconds) && targetSeconds >= 5 && targetSeconds <= 1800);
  const usedLang = lang ?? guessLang(text);
  const args: GenerateArgs = useMemo(
    () => ({ text, lang: usedLang, orientation, targetSeconds: targetBad ? null : targetSeconds, model, illustrate, detail, name: name.trim() || null }),
    [text, usedLang, orientation, targetSeconds, targetBad, model, illustrate, detail, name],
  );
  const hasText = text.trim().length > 0;

  // The estimate follows the form (free: it only counts tokens).
  const wanted = useRef(0);
  useEffect(() => {
    if (!hasText) {
      setEstimate(null);
      setEstimateError(null);
      return;
    }
    const id = ++wanted.current;
    const handle = window.setTimeout(() => {
      api.estimate(args).then(
        (e) => id === wanted.current && (setEstimate(e), setEstimateError(null)),
        (e) => id === wanted.current && (setEstimate(null), setEstimateError(errorText(e))),
      );
    }, 250);
    return () => window.clearTimeout(handle);
  }, [api, args, hasText]);

  // Progress and the result arrive as events while a generation runs.
  useEffect(() => {
    let off: (() => void) | undefined;
    let closed = false;
    void api
      .onGenerate(
        (p) => setProgress((list) => [...list, p]),
        (f) => {
          setResult(f);
          setCancelling(false);
          setStep(f.ok ? "done" : "failed");
        },
      )
      .then((u) => (closed ? u() : (off = u)));
    return () => {
      closed = true;
      off?.();
    };
  }, [api]);

  useEffect(() => {
    if (step !== "running") return;
    const tick = window.setInterval(() => setElapsed(Math.floor((Date.now() - startedAt) / 1000)), 500);
    return () => window.clearInterval(tick);
  }, [step, startedAt]);

  const openTextFile = async () => {
    try {
      const file = await api.pickTextFile();
      if (!file) return;
      setText(file.text);
      if (!name.trim()) setName(file.name.replace(/\.[^.]+$/, ""));
    } catch (e) {
      setEstimateError(errorText(e));
    }
  };

  /** The confirm button: the only place a paid call starts. */
  const start = async () => {
    setProgress([]);
    setResult(null);
    setStartError(null);
    setStartedAt(Date.now());
    setElapsed(0);
    setStep("running");
    try {
      await api.startGenerate(args);
    } catch (e) {
      setStartError(errorText(e));
      setStep("failed");
    }
  };

  const cancel = async () => {
    setCancelling(true);
    await api.cancelGenerate().catch(() => false);
  };

  const stageNow = progress.length ? progress[progress.length - 1].stage : "storyboard";
  const stages = STAGES.filter((s) => s.key !== "illustrations" || illustrate);
  const lastDetail = [...progress].reverse().find((p) => p.stage === "illustrations")?.message;
  const canContinue = hasText && !targetBad && estimate !== null && !estimate.problem;

  const optionsSummary = (
    <div className="st-summary">
      <span>{MODELS.find((m) => m.value === model)!.label}</span>
      <span>{{ zh: "中文", ja: "日本語", en: "English" }[usedLang]}</span>
      <span>{orientation === "portrait" ? "Portrait 9:16" : "Landscape 16:9"}</span>
      <span>{targetSeconds && !targetBad ? `about ${targetSeconds} s` : "length from the text"}</span>
      <span>{illustrate ? `with ${detail} illustrations` : "no illustrations"}</span>
    </div>
  );

  return (
    <Modal title="Generate a video from text" onClose={onClose} dismissible={step !== "running"} wide>
      {step === "form" && (
        <div className="st-form">
          <label className="field">
            <span className="field-label">
              Text
              <span className="suffix">{Array.from(text).length.toLocaleString()} characters</span>
            </span>
            <textarea
              data-autofocus
              className="st-text"
              rows={9}
              value={text}
              placeholder="Paste the text the video should tell: an article, notes, a script."
              onChange={(e) => setText(e.target.value)}
            />
          </label>
          <div className="st-row">
            <button className="btn small" onClick={() => void openTextFile()}>
              Open a .txt file…
            </button>
            <span className="dim">Claude writes the storyboard: scenes, narration, camera moves and music mood.</span>
          </div>

          <div className="st-row top">
            <label className="field">
              <span className="field-label">Language</span>
              <select value={lang ?? ""} onChange={(e) => setLang((e.target.value || null) as Lang | null)}>
                <option value="">Detect ({{ zh: "中文", ja: "日本語", en: "English" }[guessLang(text)]})</option>
                <option value="zh">中文</option>
                <option value="ja">日本語</option>
                <option value="en">English</option>
              </select>
            </label>
            <div className="field">
              <span className="field-label">Format</span>
              <div className="segmented">
                <button className={`seg${orientation === "landscape" ? " active" : ""}`} onClick={() => setOrientation("landscape")}>
                  Landscape
                </button>
                <button className={`seg${orientation === "portrait" ? " active" : ""}`} onClick={() => setOrientation("portrait")}>
                  Portrait
                </button>
              </div>
            </div>
            <label className="field st-narrow">
              <span className="field-label">Length (seconds)</span>
              <input
                inputMode="numeric"
                value={target}
                placeholder="Auto"
                aria-invalid={targetBad}
                onChange={(e) => setTarget(e.target.value.replace(/[^\d]/g, ""))}
              />
              {targetBad && <span className="field-error">Between 5 and 1800.</span>}
            </label>
          </div>

          <div className="st-row top">
            <div className="field">
              <span className="field-label">Model</span>
              <div className="segmented">
                {MODELS.map((m) => (
                  <button key={m.value} className={`seg${model === m.value ? " active" : ""}`} onClick={() => setModel(m.value)} title={m.note}>
                    {m.label}
                  </button>
                ))}
              </div>
              <span className="dim st-hint">{MODELS.find((m) => m.value === model)!.note}</span>
            </div>
            <label className="field">
              <span className="field-label">Project name</span>
              <input value={name} placeholder="The storyboard's title" onChange={(e) => setName(e.target.value)} />
            </label>
          </div>

          <label className="st-check">
            <input type="checkbox" checked={illustrate} onChange={(e) => setIllustrate(e.target.checked)} />
            <span>
              Draw an illustration for each scene
              <span className="dim">
                {" "}
                {estimate?.illustrate
                  ? `— adds ${formatCost(estimate.illustrate)} for about ${estimate.scenes} scenes`
                  : `— costs far more than the storyboard (about ${detail === "rich" ? "$0.25" : "$0.08"} a scene with Opus)`}
              </span>
            </span>
          </label>
          {illustrate && (
            <div className="field">
              <span className="field-label">Picture detail</span>
              <div className="segmented">
                {DETAILS.map((d) => (
                  <button key={d.value} className={`seg${detail === d.value ? " active" : ""}`} onClick={() => setDetail(d.value)} title={d.note}>
                    {d.label}
                  </button>
                ))}
              </div>
              <span className="dim st-hint">{DETAILS.find((d) => d.value === detail)!.note}</span>
            </div>
          )}

          <div className={`st-estimate${estimate?.problem || estimateError ? " bad" : ""}`} aria-live="polite">
            {!hasText ? (
              <span className="dim">The cost estimate shows here once there is text.</span>
            ) : estimateError ? (
              <span>{estimateError}</span>
            ) : !estimate ? (
              <span className="dim">Estimating…</span>
            ) : estimate.problem ? (
              <span>{estimate.problem}</span>
            ) : (
              <>
                <span className="st-estimate-main">Estimated cost: {formatCost(estimate.total)}</span>
                <span className="dim">
                  about {estimate.scenes} scenes · {estimate.inputTokens.toLocaleString()} input + {estimate.outputTokens.toLocaleString()} output tokens
                  for the storyboard · paid with your Anthropic API key
                </span>
              </>
            )}
          </div>

          <div className="st-modal-foot">
            <span className="dim">Nothing is sent until you confirm the cost.</span>
            <span className="spacer" />
            <button className="btn ghost" onClick={onClose}>
              Cancel
            </button>
            <button className="btn primary" disabled={!canContinue} onClick={() => setStep("confirm")}>
              Review cost…
            </button>
          </div>
        </div>
      )}

      {step === "confirm" && estimate && (
        <div className="st-form">
          <div className="st-confirm">
            <div className="st-confirm-cost">{formatUsd(estimate.total.typical)}</div>
            <div>
              <div className="st-confirm-title">This generation costs {formatCost(estimate.total)}.</div>
              <div className="dim">
                Storyboard {formatUsd(estimate.storyboard.typical)}
                {estimate.illustrate ? ` + illustrations ${formatUsd(estimate.illustrate.typical)} for about ${estimate.scenes} scenes` : ""}. The higher
                figure applies when Claude has to correct its answers. The amount is charged to your Anthropic API key; the actual cost shows when
                it is done.
              </div>
            </div>
          </div>
          {optionsSummary}
          <div className="st-modal-foot">
            <span className="spacer" />
            <button className="btn ghost" onClick={() => setStep("form")}>
              Back
            </button>
            <button className="btn primary" data-autofocus onClick={() => void start()}>
              Generate for about {formatUsd(estimate.total.typical)}
            </button>
          </div>
        </div>
      )}

      {step === "running" && (
        <div className="st-form">
          <ol className="st-stages">
            {stages.map((s) => {
              const index = stages.findIndex((x) => x.key === s.key);
              const current = stages.findIndex((x) => x.key === stageNow);
              const state = index < current ? "done" : index === current ? "now" : "todo";
              return (
                <li key={s.key} className={state}>
                  <span className="st-stage-dot" aria-hidden />
                  <span>{s.label}</span>
                  {state === "now" && s.key === "illustrations" && lastDetail && <span className="dim st-stage-detail">{lastDetail}</span>}
                </li>
              );
            })}
          </ol>
          <div className="render-bar indeterminate">
            <span />
          </div>
          <div className="st-modal-foot">
            <span className="dim mono">
              {Math.floor(elapsed / 60)}:{String(elapsed % 60).padStart(2, "0")}
            </span>
            <span className="dim">{cancelling ? "Stopping…" : "A storyboard usually takes 20 to 60 seconds."}</span>
            <span className="spacer" />
            <button className="btn" disabled={cancelling} onClick={() => void cancel()}>
              Cancel
            </button>
          </div>
        </div>
      )}

      {step === "done" && result?.project && (
        <div className="st-form">
          <div className="st-result ok">
            <div className="st-result-title">“{result.project.name}” is ready</div>
            <div className="dim">
              {result.scenes} scenes
              {result.illustrated > 0 ? `, ${result.illustrated} illustrated` : ""} · actual cost {formatUsd(result.costUsd)} (
              {result.inputTokens.toLocaleString()} input + {result.outputTokens.toLocaleString()} output tokens)
            </div>
          </div>
          {result.warnings.length > 0 && (
            <ul className="st-warnings">
              {result.warnings.map((w, i) => (
                <li key={i}>{w}</li>
              ))}
            </ul>
          )}
          <div className="st-modal-foot">
            <span className="dim">The text, storyboard and usage are saved in the project's source folder.</span>
            <span className="spacer" />
            <button className="btn ghost" onClick={onClose}>
              Close
            </button>
            <button className="btn primary" data-autofocus onClick={() => onOpenProject(result.project!.dir)}>
              Open project
            </button>
          </div>
        </div>
      )}

      {step === "failed" && (
        <div className="st-form">
          <div className="st-result bad" role="alert">
            <div className="st-result-title">{result?.cancelled ? "Cancelled" : "The video was not generated"}</div>
            <div>{startError ?? result?.error ?? "Something went wrong."}</div>
            {result?.errorKind === "no_api_key" && <div className="dim">{NO_KEY_HELP}</div>}
            <div className="dim">
              {result && result.costUsd > 0
                ? `The calls made before this cost ${formatUsd(result.costUsd)}.`
                : result?.cancelled
                  ? "A call that was running when you cancelled may still be billed by Anthropic; its tokens are not known here."
                  : "No cost was reported."}{" "}
              No project was created.
            </div>
          </div>
          <div className="st-modal-foot">
            <span className="spacer" />
            <button className="btn ghost" onClick={onClose}>
              Close
            </button>
            <button className="btn primary" onClick={() => setStep("form")}>
              Back to the text
            </button>
          </div>
        </div>
      )}
    </Modal>
  );
};
