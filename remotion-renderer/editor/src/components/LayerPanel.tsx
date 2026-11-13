import type { Dispatch } from "react";
import { useEffect, useState } from "react";
import type { Layer, LayerAnimation, LayerAnimationKind, Scene } from "../../../src/types/timeline";
import { splitLayers } from "../../../src/motion/layers";
import { ANIMATION_KINDS, LAYER_KINDS } from "../layers";
import type { Action } from "../state";
import { Field, MediaInput, NumField } from "./Inspector";

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
            + {kind}
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
          <div className="two-col">
            <Field label="Reveal">
              <select value={layer.reveal ?? "none"} onChange={(e) => patch({ reveal: e.target.value === "none" ? undefined : e.target.value })}>
                <option value="none">all at once</option>
                <option value="words">word by word</option>
              </select>
            </Field>
            <ColorField
              label="Background"
              optional
              value={layer.background?.color ?? ""}
              onChange={(c) => patch({ background: c ? { ...(layer.background ?? {}), color: c } : undefined })}
            />
          </div>
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
  }
};

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
    </div>
  </Field>
);

const labelOf = (k: LayerAnimationKind) => label(k);
