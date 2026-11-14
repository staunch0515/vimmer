import type { Dispatch } from "react";
import { useEffect, useState } from "react";
import type { BarChartLayer, ChartBar, CounterLayer, Easing, ProgressRingLayer, Layer, LayerAnimation, LayerAnimationKind, LowerThirdLayer, Scene } from "../../../src/types/timeline";
import { splitLayers } from "../../../src/motion/layers";
import { ANIMATION_KINDS, LAYER_KINDS } from "../layers";
import { EASINGS } from "../model";
import type { Action } from "../state";
import { Field, MediaInput, NumField } from "./Inspector";
import { ClipFields } from "./ClipFields";

type Props = {
  scene: Scene;
  sceneIndex: number;
  selectedIndex: number;
  dispatch: Dispatch<Action>;
};

const label = (s: string) => s.replace(/_/g, " ");

/** The scene's layers: list in stacking order (top first), add, select, reorder, delete, edit. */
export const LayerPanel = ({ scene, sceneIndex, selectedIndex, dispatch }: Props) => {
  const layers = scene.layers ?? [];
  const { below, above } = splitLayers(layers);
  const layer: Layer | undefined = layers[selectedIndex];
  const select = (index: number) => dispatch({ type: "select", selection: { kind: "layer", sceneIndex, index } });
  const row = (l: Layer) => {
    const i = layers.indexOf(l);
    return (
      <li key={`${l.id}-${i}`}>
        <button className={i === selectedIndex ? "cue active" : "cue"} onClick={() => select(i)} data-layer-row={l.id}>
          <span className="mono dim layer-kind">{l.kind}</span>
          <span className="cue-text">{l.id}</span>
          <span className="mono dim">z {l.z ?? 0}</span>
        </button>
      </li>
    );
  };
  return (
    <>
      <div className="section-title">Layers <span className="count">{layers.length}</span></div>
      <div className="button-row">
        {LAYER_KINDS.map((kind) => (
          <button key={kind} className="btn small" onClick={() => dispatch({ type: "addLayer", sceneIndex, kind })}>
            + {label(kind)}
          </button>
        ))}
      </div>
      {layers.length > 0 && (
        <ul className="cue-list" aria-label="Layers, top first">
          {[...above].reverse().map(row)}
          <li className="layer-divider dim">scene text</li>
          {[...below].reverse().map(row)}
        </ul>
      )}
      {layer && (
        <LayerEditor key={`${sceneIndex}-${selectedIndex}`} layer={layer} scene={scene} sceneIndex={sceneIndex} index={selectedIndex} dispatch={dispatch} />
      )}
    </>
  );
};

const LayerEditor = ({
  layer, scene, sceneIndex, index, dispatch,
}: { layer: Layer; scene: Scene; sceneIndex: number; index: number; dispatch: Dispatch<Action> }) => {
  const patch = (p: Record<string, unknown>) => dispatch({ type: "patchLayer", sceneIndex, index, patch: p });
  const frac = (key: "x" | "y" | "width", name: string) => (
    <NumField
      label={name}
      value={layer[key]}
      step={0.01}
      validate={(v) => (key === "width" ? (v >= 0 && v <= 4 ? null : "0 to 4") : v >= -1 && v <= 2 ? null : "-1 to 2")}
      onCommit={(v) => patch({ [key]: v })}
    />
  );
  return (
    <div className="layer-editor">
      <div className="section-title">Layer “{layer.id}” · {layer.kind}</div>
      <div className="button-row">
        <button className="btn ghost small" onClick={() => dispatch({ type: "moveLayer", sceneIndex, index, delta: 1 })}>Bring forward</button>
        <button className="btn ghost small" onClick={() => dispatch({ type: "moveLayer", sceneIndex, index, delta: -1 })}>Send backward</button>
        <button className="btn danger small" onClick={() => dispatch({ type: "deleteLayer", sceneIndex, index })}>Delete layer</button>
      </div>
      <div className="two-col">
        <Field label="ID">
          <input value={layer.id} onChange={(e) => patch({ id: e.target.value })} />
        </Field>
        <NumField
          label="Z"
          hint="< 0 under the scene text"
          value={layer.z ?? 0}
          step={1}
          validate={(v) => (Number.isInteger(v) ? null : "Whole number")}
          onCommit={(v) => patch({ z: v === 0 ? undefined : v })}
        />
      </div>
      <div className="two-col">
        {frac("x", "X")}
        {frac("y", "Y")}
      </div>
      <div className="two-col">
        {frac("width", "Width")}
        <OptionalNum
          label="Height"
          placeholder={layer.kind === "text" ? "auto" : ""}
          value={layer.height ?? undefined}
          step={0.01}
          onCommit={(v) => patch({ height: v })}
        />
      </div>
      <Field label="Anchor" hint="what x / y place">
        <select value={layer.anchor ?? "top_left"} onChange={(e) => patch({ anchor: e.target.value === "top_left" ? undefined : e.target.value })}>
          <option value="top_left">top left</option>
          <option value="center">center</option>
        </select>
      </Field>
      <div className="two-col">
        <NumField
          label="Start"
          suffix="ms"
          value={layer.start_ms ?? 0}
          step={100}
          validate={(v) => (Number.isInteger(v) && v >= 0 && v < scene.duration_ms ? null : `0 to ${scene.duration_ms - 1}`)}
          onCommit={(v) => patch({ start_ms: v === 0 ? undefined : v })}
        />
        <OptionalNum
          label="End"
          suffix="ms"
          placeholder={`${scene.duration_ms} (scene end)`}
          value={layer.end_ms ?? undefined}
          step={100}
          onCommit={(v) => patch({ end_ms: v })}
        />
      </div>
      <div className="two-col">
        <NumField
          label="Opacity"
          value={layer.opacity ?? 1}
          step={0.1}
          validate={(v) => (v >= 0 && v <= 1 ? null : "0 to 1")}
          onCommit={(v) => patch({ opacity: v === 1 ? undefined : v })}
        />
        <NumField
          label="Rotation"
          suffix="°"
          value={layer.rotation ?? 0}
          step={1}
          validate={(v) => (Math.abs(v) <= 360 ? null : "±360")}
          onCommit={(v) => patch({ rotation: v === 0 ? undefined : v })}
        />
      </div>
      <div className="two-col">
        <AnimationField label="Enter" value={layer.enter ?? null} onChange={(a) => patch({ enter: a ?? undefined })} />
        <AnimationField label="Exit" value={layer.exit ?? null} onChange={(a) => patch({ exit: a ?? undefined })} />
      </div>
      {(layer.keyframes?.length ?? 0) > 0 && (
        <div className="button-row">
          <span className="dim">{layer.keyframes!.length} keyframes (edit in JSON or with the assistant)</span>
          <button className="btn ghost small" onClick={() => patch({ keyframes: undefined })}>Clear keyframes</button>
        </div>
      )}
      <KindFields layer={layer} patch={patch} />
    </div>
  );
};

const KindFields = ({ layer, patch }: { layer: Layer; patch: (p: Record<string, unknown>) => void }) => {
  switch (layer.kind) {
    case "text":
      return (
        <>
          <Field label="Text">
            <textarea rows={2} value={layer.text} onChange={(e) => patch({ text: e.target.value })} />
          </Field>
          <div className="two-col">
            <NumField
              label="Font size"
              hint="of frame height"
              value={layer.font_size ?? 0.06}
              step={0.005}
              validate={(v) => (v > 0 && v <= 0.5 ? null : "above 0, at most 0.5")}
              onCommit={(v) => patch({ font_size: v })}
            />
            <ColorField label="Color" value={layer.color ?? "#fafafa"} onChange={(c) => patch({ color: c })} />
          </div>
          <div className="two-col">
            <Field label="Weight">
              <select value={layer.weight ?? 600} onChange={(e) => patch({ weight: Number(e.target.value) })}>
                {[300, 400, 500, 600, 700, 800, 900].map((w) => <option key={w} value={w}>{w}</option>)}
              </select>
            </Field>
            <Field label="Align">
              <select value={layer.align ?? "center"} onChange={(e) => patch({ align: e.target.value })}>
                {["left", "center", "right"].map((a) => <option key={a} value={a}>{a}</option>)}
              </select>
            </Field>
          </div>
          <Field label="Brand font">
            <select value={layer.font ?? "body"} onChange={(e) => patch({ font: e.target.value === "body" ? undefined : e.target.value })}>
              <option value="body">body</option>
              <option value="heading">heading</option>
            </select>
          </Field>
          <div className="two-col">
            <Field label="Reveal">
              <select value={layer.reveal ?? "none"} onChange={(e) => patch({ reveal: e.target.value === "none" ? undefined : e.target.value })}>
                <option value="none">all at once</option>
                <option value="words">word by word</option>
                <option value="typewriter">typewriter</option>
              </select>
            </Field>
            <ColorField
              label="Background"
              optional
              value={layer.background?.color ?? ""}
              onChange={(c) => patch({ background: c ? { ...(layer.background ?? {}), color: c } : undefined })}
            />
          </div>
          {layer.reveal === "typewriter" && (
            <div className="two-col">
              <NumField
                label="Typing speed"
                suffix="chars/s"
                value={layer.typing_cps ?? 20}
                step={1}
                validate={(v) => (v >= 1 && v <= 200 ? null : "1 to 200")}
                onCommit={(v) => patch({ typing_cps: v === 20 ? undefined : v })}
              />
              <Field label="Cursor">
                <label className="check">
                  <input type="checkbox" checked={Boolean(layer.cursor)} onChange={(e) => patch({ cursor: e.target.checked || undefined })} />
                  blinking cursor
                </label>
              </Field>
            </div>
          )}
          <Field label="Highlights" hint="Comma-separated words">
            <input
              value={(layer.highlights ?? []).join(", ")}
              onChange={(e) => {
                const list = e.target.value.split(",").map((w) => w.trim()).filter(Boolean);
                patch({ highlights: list.length ? list : undefined });
              }}
            />
          </Field>
        </>
      );
    case "shape":
      return (
        <>
          <Field label="Shape">
            <select value={layer.shape ?? "rect"} onChange={(e) => patch({ shape: e.target.value })}>
              {["rect", "ellipse", "line"].map((s) => <option key={s} value={s}>{s}</option>)}
            </select>
          </Field>
          <div className="two-col">
            <ColorField label="Fill" optional value={layer.fill ?? ""} onChange={(c) => patch({ fill: c || undefined })} />
            <ColorField label="Stroke" optional value={layer.stroke ?? ""} onChange={(c) => patch({ stroke: c || undefined })} />
          </div>
          <div className="two-col">
            <NumField
              label="Stroke width"
              value={layer.stroke_width ?? 0}
              step={0.001}
              validate={(v) => (v >= 0 && v <= 0.1 ? null : "0 to 0.1")}
              onCommit={(v) => patch({ stroke_width: v || undefined })}
            />
            <NumField
              label="Corner radius"
              value={layer.corner_radius ?? 0}
              step={0.005}
              validate={(v) => (v >= 0 && v <= 0.5 ? null : "0 to 0.5")}
              onCommit={(v) => patch({ corner_radius: v || undefined })}
            />
          </div>
        </>
      );
    case "image":
      return (
        <>
          <Field label="Image" hint="Path relative to public/">
            <MediaInput kind="image" value={layer.src} placeholder="assets/sample.png" onChange={(v) => patch({ src: v })} />
          </Field>
          <div className="two-col">
            <Field label="Fit">
              <select value={layer.fit ?? "cover"} onChange={(e) => patch({ fit: e.target.value })}>
                <option value="cover">cover (crop)</option>
                <option value="contain">contain</option>
              </select>
            </Field>
            <NumField
              label="Corner radius"
              value={layer.corner_radius ?? 0}
              step={0.005}
              validate={(v) => (v >= 0 && v <= 0.5 ? null : "0 to 0.5")}
              onCommit={(v) => patch({ corner_radius: v || undefined })}
            />
          </div>
        </>
      );
    case "icon":
      return (
        <>
          <Field label="Icon" hint=".svg under public/">
            <MediaInput kind="icon" value={layer.src} placeholder="assets/icons/star.svg" onChange={(v) => patch({ src: v })} />
          </Field>
          <ColorField label="Color" value={layer.color ?? "#fafafa"} onChange={(c) => patch({ color: c })} />
        </>
      );
    case "video":
      return <ClipFields clip={layer} patch={patch} />;
    case "lower_third":
      return <LowerThirdFields layer={layer} patch={patch} />;
    case "counter":
      return <CounterFields layer={layer} patch={patch} />;
    case "bar_chart":
      return <BarChartFields layer={layer} patch={patch} />;
    case "progress_ring":
      return <ProgressRingFields layer={layer} patch={patch} />;
  }
};

const LowerThirdFields = ({ layer, patch }: { layer: LowerThirdLayer; patch: (p: Record<string, unknown>) => void }) => (
  <>
    <Field label="Title">
      <input value={layer.title} onChange={(e) => patch({ title: e.target.value })} />
    </Field>
    <Field label="Subtitle" hint="empty = none">
      <input value={layer.subtitle ?? ""} onChange={(e) => patch({ subtitle: e.target.value || undefined })} />
    </Field>
    <div className="two-col">
      <NumField
        label="Font size"
        hint="title, of frame height"
        value={layer.font_size ?? 0.045}
        step={0.005}
        validate={(v) => (v > 0 && v <= 0.2 ? null : "above 0, at most 0.2")}
        onCommit={(v) => patch({ font_size: v })}
      />
      <ColorField label="Accent" value={layer.accent_color ?? "#6366f1"} onChange={(c) => patch({ accent_color: c })} />
    </div>
    <div className="two-col">
      <ColorField label="Title color" value={layer.color ?? "#fafafa"} onChange={(c) => patch({ color: c })} />
      <ColorField label="Subtitle color" value={layer.subtitle_color ?? "#d4d4d8"} onChange={(c) => patch({ subtitle_color: c })} />
    </div>
    <ColorField
      label="Background"
      optional
      value={layer.background_color === undefined ? "#09090bd9" : layer.background_color ?? ""}
      onChange={(c) => patch({ background_color: c === "" ? null : c })}
    />
    <p className="dim">Wipes in and out by itself unless Enter / Exit is set.</p>
  </>
);

const EaseField = ({ label, value, onChange }: { label: string; value: Easing; onChange: (e: Easing) => void }) => (
  <Field label={label}>
    <select value={value} onChange={(e) => onChange(e.target.value as Easing)}>
      {EASINGS.map((k) => (
        <option key={k} value={k}>{k.replace(/_/g, " ")}</option>
      ))}
    </select>
  </Field>
);

const CounterFields = ({ layer, patch }: { layer: CounterLayer; patch: (p: Record<string, unknown>) => void }) => (
  <>
    <div className="two-col">
      <NumField label="From" value={layer.from ?? 0} step={1} validate={(v) => (Math.abs(v) <= 1e15 ? null : "±1e15")} onCommit={(v) => patch({ from: v })} />
      <NumField label="To" value={layer.to} step={1} validate={(v) => (Math.abs(v) <= 1e15 ? null : "±1e15")} onCommit={(v) => patch({ to: v })} />
    </div>
    <div className="two-col">
      <OptionalNum label="Count time" suffix="ms" placeholder="until the exit" value={layer.duration_ms ?? undefined} step={100} onCommit={(v) => patch({ duration_ms: v })} />
      <EaseField label="Easing" value={layer.ease ?? "ease_out"} onChange={(e) => patch({ ease: e })} />
    </div>
    <div className="two-col">
      <NumField label="Decimals" value={layer.decimals ?? 0} step={1} validate={(v) => (Number.isInteger(v) && v >= 0 && v <= 6 ? null : "0 to 6")} onCommit={(v) => patch({ decimals: v || undefined })} />
      <Field label="Separator" hint="thousands; empty = none">
        <input value={layer.separator ?? ","} onChange={(e) => patch({ separator: e.target.value === "," ? undefined : e.target.value })} />
      </Field>
    </div>
    <div className="two-col">
      <Field label="Prefix">
        <input value={layer.prefix ?? ""} onChange={(e) => patch({ prefix: e.target.value || undefined })} />
      </Field>
      <Field label="Suffix">
        <input value={layer.suffix ?? ""} onChange={(e) => patch({ suffix: e.target.value || undefined })} />
      </Field>
    </div>
    <div className="two-col">
      <NumField
        label="Font size"
        hint="of frame height"
        value={layer.font_size ?? 0.12}
        step={0.01}
        validate={(v) => (v > 0 && v <= 0.5 ? null : "above 0, at most 0.5")}
        onCommit={(v) => patch({ font_size: v })}
      />
      <ColorField label="Color" value={layer.color ?? "#fafafa"} onChange={(c) => patch({ color: c })} />
    </div>
  </>
);

const BarChartFields = ({ layer, patch }: { layer: BarChartLayer; patch: (p: Record<string, unknown>) => void }) => {
  const setBar = (i: number, p: Partial<ChartBar>) =>
    patch({ bars: layer.bars.map((b, j) => (j === i ? Object.fromEntries(Object.entries({ ...b, ...p }).filter(([, v]) => v !== undefined)) : b)) });
  return (
    <>
      <div className="two-col">
        <Field label="Orientation">
          <select value={layer.orientation ?? "vertical"} onChange={(e) => patch({ orientation: e.target.value === "vertical" ? undefined : e.target.value })}>
            <option value="vertical">vertical</option>
            <option value="horizontal">horizontal</option>
          </select>
        </Field>
        <OptionalNum label="Max" placeholder="largest value" value={layer.max ?? undefined} step={1} onCommit={(v) => patch({ max: v })} />
      </div>
      <Field label="Bars" hint="label · value · color (empty = chart color)">
        <div className="bar-rows">
          {layer.bars.map((b, i) => (
            <div key={i} className="bar-row" data-bar-row={i}>
              <input value={b.label} aria-label={`Bar ${i + 1} label`} onChange={(e) => setBar(i, { label: e.target.value })} />
              <input
                type="number"
                value={b.value}
                aria-label={`Bar ${i + 1} value`}
                onChange={(e) => {
                  const v = Number(e.target.value);
                  if (e.target.value.trim() !== "" && Number.isFinite(v)) setBar(i, { value: v });
                }}
              />
              <input value={b.color ?? ""} placeholder="#rrggbb" aria-label={`Bar ${i + 1} color`} onChange={(e) => setBar(i, { color: e.target.value.trim() || undefined })} />
              <button className="btn ghost small" aria-label={`Remove bar ${i + 1}`} disabled={layer.bars.length <= 1} onClick={() => patch({ bars: layer.bars.filter((_, j) => j !== i) })}>
                ×
              </button>
            </div>
          ))}
          <button
            className="btn small"
            disabled={layer.bars.length >= 12}
            onClick={() => patch({ bars: [...layer.bars, { label: String.fromCharCode(65 + (layer.bars.length % 26)), value: layer.bars[layer.bars.length - 1]?.value ?? 50 }] })}
          >
            + bar
          </button>
        </div>
      </Field>
      <div className="two-col">
        <ColorField label="Bar color" value={layer.color ?? "#6366f1"} onChange={(c) => patch({ color: c })} />
        <ColorField label="Label color" value={layer.label_color ?? "#e4e4e7"} onChange={(c) => patch({ label_color: c })} />
      </div>
      <div className="two-col">
        <NumField
          label="Font size"
          hint="of frame height"
          value={layer.font_size ?? 0.032}
          step={0.002}
          validate={(v) => (v > 0 && v <= 0.2 ? null : "above 0, at most 0.2")}
          onCommit={(v) => patch({ font_size: v })}
        />
        <Field label="Values">
          <label className="check">
            <input type="checkbox" checked={layer.show_values ?? true} onChange={(e) => patch({ show_values: e.target.checked ? undefined : false })} />
            show values
          </label>
        </Field>
      </div>
      <div className="two-col">
        <NumField label="Grow" suffix="ms" value={layer.grow_ms ?? 800} step={100} validate={(v) => (v >= 1 && v <= 10000 ? null : "1 to 10000")} onCommit={(v) => patch({ grow_ms: v })} />
        <NumField label="Stagger" suffix="ms" value={layer.stagger_ms ?? 120} step={20} validate={(v) => (v >= 0 && v <= 2000 ? null : "0 to 2000")} onCommit={(v) => patch({ stagger_ms: v })} />
      </div>
      <div className="two-col">
        <EaseField label="Easing" value={layer.ease ?? "ease_out"} onChange={(e) => patch({ ease: e })} />
        <NumField label="Decimals" value={layer.decimals ?? 0} step={1} validate={(v) => (Number.isInteger(v) && v >= 0 && v <= 6 ? null : "0 to 6")} onCommit={(v) => patch({ decimals: v || undefined })} />
      </div>
      <div className="two-col">
        <Field label="Prefix">
          <input value={layer.prefix ?? ""} onChange={(e) => patch({ prefix: e.target.value || undefined })} />
        </Field>
        <Field label="Suffix">
          <input value={layer.suffix ?? ""} onChange={(e) => patch({ suffix: e.target.value || undefined })} />
        </Field>
      </div>
    </>
  );
};

const ProgressRingFields = ({ layer, patch }: { layer: ProgressRingLayer; patch: (p: Record<string, unknown>) => void }) => (
  <>
    <div className="two-col">
      <NumField label="Value" hint="0 to 1" value={layer.value} step={0.05} validate={(v) => (v >= 0 && v <= 1 ? null : "0 to 1")} onCommit={(v) => patch({ value: v })} />
      <NumField label="From" value={layer.from ?? 0} step={0.05} validate={(v) => (v >= 0 && v <= 1 ? null : "0 to 1")} onCommit={(v) => patch({ from: v || undefined })} />
    </div>
    <div className="two-col">
      <OptionalNum label="Fill time" suffix="ms" placeholder="until the exit" value={layer.duration_ms ?? undefined} step={100} onCommit={(v) => patch({ duration_ms: v })} />
      <EaseField label="Easing" value={layer.ease ?? "ease_out"} onChange={(e) => patch({ ease: e })} />
    </div>
    <div className="two-col">
      <ColorField label="Ring" value={layer.color ?? "#6366f1"} onChange={(c) => patch({ color: c })} />
      <ColorField label="Track" value={layer.track_color ?? "#ffffff26"} onChange={(c) => patch({ track_color: c })} />
    </div>
    <div className="two-col">
      <NumField
        label="Thickness"
        hint="of the diameter"
        value={layer.thickness ?? 0.12}
        step={0.01}
        validate={(v) => (v >= 0.02 && v <= 0.5 ? null : "0.02 to 0.5")}
        onCommit={(v) => patch({ thickness: v })}
      />
      <Field label="Center">
        <select
          value={layer.show_label === false ? "none" : layer.label != null ? "text" : "percent"}
          onChange={(e) => {
            const v = e.target.value;
            patch(v === "none" ? { show_label: false, label: undefined } : v === "text" ? { show_label: undefined, label: layer.label ?? "Done" } : { show_label: undefined, label: undefined });
          }}
        >
          <option value="percent">percentage</option>
          <option value="text">custom text</option>
          <option value="none">nothing</option>
        </select>
      </Field>
    </div>
    {layer.label != null && layer.show_label !== false && (
      <Field label="Center text">
        <input value={layer.label} onChange={(e) => patch({ label: e.target.value })} />
      </Field>
    )}
  </>
);

/** Number input whose empty value means "default" (undefined). */
const OptionalNum = ({
  label, suffix, placeholder, value, step, onCommit,
}: { label: string; suffix?: string; placeholder: string; value: number | undefined; step: number; onCommit: (v: number | undefined) => void }) => {
  const [text, setText] = useState(value === undefined ? "" : String(value));
  useEffect(() => setText(value === undefined ? "" : String(value)), [value]);
  return (
    <Field label={label} suffix={suffix}>
      <input
        type="number"
        step={step}
        placeholder={placeholder}
        value={text}
        onChange={(e) => {
          setText(e.target.value);
          if (e.target.value.trim() === "") onCommit(undefined);
          else if (Number.isFinite(Number(e.target.value))) onCommit(Number(e.target.value));
        }}
      />
    </Field>
  );
};

/** A #rgb / #rrggbb / #rrggbbaa text field with a swatch. Empty = none when `optional`. */
const ColorField = ({
  label, value, optional, onChange,
}: { label: string; value: string; optional?: boolean; onChange: (c: string) => void }) => {
  const valid = /^#([0-9a-fA-F]{3}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$/.test(value);
  return (
    <Field label={label} hint={optional ? "empty = none" : undefined}>
      <div className="color-field">
        <span className="swatch" style={{ background: valid ? value : "transparent" }} />
        <input
          value={value}
          placeholder={optional ? "none" : "#rrggbb"}
          aria-invalid={value !== "" && !valid ? true : undefined}
          onChange={(e) => onChange(e.target.value.trim())}
        />
      </div>
    </Field>
  );
};

const AnimationField = ({
  label, value, onChange,
}: { label: string; value: LayerAnimation | null; onChange: (a: LayerAnimation | null) => void }) => (
  <Field label={label} suffix={value ? `${value.duration_ms ?? 400} ms` : undefined}>
    <div className="anim-field">
      <select
        value={value?.kind ?? "none"}
        onChange={(e) =>
          onChange(e.target.value === "none" ? null : { kind: e.target.value as LayerAnimationKind, duration_ms: value?.duration_ms ?? 400 })
        }
      >
        <option value="none">none</option>
        {ANIMATION_KINDS.map((k) => (
          <option key={k} value={k}>{labelOf(k)}</option>
        ))}
      </select>
      {value && (
        <input
          type="number"
          min={1}
          step={50}
          value={value.duration_ms ?? 400}
          aria-label={`${label} duration in ms`}
          onChange={(e) => {
            const ms = Math.round(Number(e.target.value));
            if (ms > 0) onChange({ ...value, duration_ms: ms });
          }}
        />
      )}
      {value && (
        <select
          value={value.ease ?? ""}
          aria-label={`${label} easing`}
          onChange={(e) => onChange({ ...value, ease: (e.target.value || undefined) as Easing | undefined })}
        >
          <option value="">default curve</option>
          {EASINGS.map((k) => (
            <option key={k} value={k}>{k.replace(/_/g, " ")}</option>
          ))}
        </select>
      )}
    </div>
  </Field>
);

const labelOf = (k: LayerAnimationKind) => label(k);
