import { useState, type Dispatch } from "react";
import type { Brand, BrandColors, BrandFont, Logo, LogoCorner, SceneTemplate, VideoTimeline } from "../../../src/types/timeline";
import { BUILTIN } from "../../../src/brand";
import type { Action } from "../state";
import type { Selection } from "../model";
import { pickToOpen, saveTimeline } from "../project";
import { CATALOG, parseBrand, templateInfo, type ParamSpec } from "../templates";
import { Field, MediaInput } from "./Inspector";

type Props = {
  timeline: VideoTimeline;
  selection: Selection;
  dispatch: Dispatch<Action>;
};

const COLORS: { key: keyof BrandColors; label: string; builtin: string; hint: string }[] = [
  { key: "primary", label: "Primary", builtin: BUILTIN.primary, hint: "bars, rings, template shapes" },
  { key: "accent", label: "Accent", builtin: BUILTIN.accent, hint: "highlighted words" },
  { key: "text", label: "Text", builtin: BUILTIN.text, hint: "" },
  { key: "muted_text", label: "Muted text", builtin: BUILTIN.lowerThirdSubtitle, hint: "secondary lines" },
  { key: "background", label: "Background", builtin: BUILTIN.background, hint: "scenes without a picture" },
  { key: "subtitle_background", label: "Subtitle box", builtin: "#09090bb8", hint: "" },
];

const CORNERS: LogoCorner[] = ["top_left", "top_right", "bottom_left", "bottom_right"];

/** `#rgb`/`#rrggbbaa` to the `#rrggbb` a color input needs (alpha is kept in the text field). */
const toPicker = (c: string): string => {
  const m = /^#([0-9a-f]{3}|[0-9a-f]{6}|[0-9a-f]{8})$/i.exec(c);
  if (!m) return "#000000";
  const h = m[1].length === 3 ? m[1].replace(/./g, (x) => x + x) : m[1].slice(0, 6);
  return `#${h.toLowerCase()}`;
};

/** Drops empty parts so the saved brand has only what was set. */
const compact = (b: Brand): Brand | null => {
  const colors = Object.fromEntries(Object.entries(b.colors ?? {}).filter(([, v]) => v)) as BrandColors;
  const fonts = Object.fromEntries(Object.entries(b.fonts ?? {}).filter(([, v]) => v && (v as BrandFont).family)) as Brand["fonts"];
  const out: Brand = {};
  if (Object.keys(colors).length) out.colors = colors;
  if (fonts && Object.keys(fonts).length) out.fonts = fonts;
  if (b.logo?.src) out.logo = b.logo;
  return Object.keys(out).length ? out : null;
};

export const BrandEditor = ({ timeline, selection, dispatch }: Props) => {
  const brand: Brand = timeline.brand ?? {};
  const [error, setError] = useState<string | null>(null);
  const set = (next: Brand) => dispatch({ type: "setBrand", brand: compact(next) });
  const setColor = (key: keyof BrandColors, value: string | undefined) =>
    set({ ...brand, colors: { ...brand.colors, [key]: value } });
  const setFont = (role: "heading" | "body", font: BrandFont | null) =>
    set({ ...brand, fonts: { ...brand.fonts, [role]: font && (font.family || font.src) ? font : null } });
  const setLogo = (patch: Partial<Logo> | null) =>
    set({ ...brand, logo: patch === null ? null : ({ ...(brand.logo ?? { src: "" }), ...patch } as Logo) });

  const load = async () => {
    try {
      const picked = await pickToOpen();
      if (!picked) return;
      dispatch({ type: "setBrand", brand: compact(parseBrand(picked.text)) });
      setError(null);
    } catch (e) {
      setError(`Could not load the brand: ${(e as Error).message}`);
    }
  };
  const save = async () => {
    try {
      await saveTimeline(JSON.stringify(brand, null, 2) + "\n", { path: "brand.json" }, { saveAs: true });
    } catch (e) {
      setError(`Could not save the brand: ${(e as Error).message}`);
    }
  };

  const after = selection.kind === "scene" ? selection.index : selection.kind === "layer" || selection.kind === "keyframe" ? selection.sceneIndex : timeline.scenes.length - 1;

  return (
    <div className="form" data-brand-panel>
      <div className="section-title">Brand kit</div>
      <div className="dim">Defaults for every scene: an element's own color always wins.</div>
      <div className="row">
        <button className="btn small" onClick={() => void load()} data-brand-load>Load brand.json</button>
        <button className="btn small" onClick={() => void save()} disabled={!timeline.brand}>Save brand.json</button>
        <button className="btn ghost small" onClick={() => dispatch({ type: "setBrand", brand: null })} disabled={!timeline.brand}>
          Clear
        </button>
      </div>
      {error && <div className="field-error">{error}</div>}

      <div className="section-title">Colors</div>
      {COLORS.map(({ key, label, builtin, hint }) => {
        const value = brand.colors?.[key];
        return (
          <Field key={key} label={label} hint={hint}>
            <div className="row">
              <input
                type="color"
                aria-label={`${label} color`}
                data-brand-color={key}
                value={toPicker(value ?? builtin)}
                onChange={(e) => setColor(key, e.target.value)}
              />
              <input
                className="mono"
                placeholder={`${builtin} (built-in)`}
                value={value ?? ""}
                onChange={(e) => setColor(key, e.target.value.trim() || undefined)}
              />
            </div>
          </Field>
        );
      })}

      <div className="section-title">Fonts</div>
      {(["heading", "body"] as const).map((role) => {
        const font = brand.fonts?.[role] ?? null;
        return (
          <div key={role} className="subform">
            <Field label={role === "heading" ? "Heading family" : "Body family"} hint="name it uniquely when loading a file">
              <input
                value={font?.family ?? ""}
                placeholder="Inter (built-in)"
                onChange={(e) => setFont(role, { ...(font ?? { family: "" }), family: e.target.value })}
              />
            </Field>
            <Field label="Font file" hint=".ttf .otf .woff .woff2 under public/">
              <input
                className="mono"
                value={font?.src ?? ""}
                placeholder="assets/fonts/MyFont.woff2"
                onChange={(e) => setFont(role, { ...(font ?? { family: "" }), src: e.target.value || null })}
              />
            </Field>
          </div>
        );
      })}

      <div className="section-title">Logo</div>
      <Field label="Logo image" hint="PNG or JPEG under public/">
        <MediaInput kind="image" value={brand.logo?.src ?? ""} placeholder="assets/brand/logo.png" onChange={(v) => setLogo(v ? { src: v } : null)} />
      </Field>
      {brand.logo?.src && (
        <>
          <Field label="Corner">
            <select value={brand.logo.corner ?? "top_right"} onChange={(e) => setLogo({ corner: e.target.value as LogoCorner })}>
              {CORNERS.map((c) => (
                <option key={c} value={c}>{c.replace("_", " ")}</option>
              ))}
            </select>
          </Field>
          <Field label="Size" hint="fraction of the frame height">
            <input type="number" min={0.02} max={0.5} step={0.01} value={brand.logo.size ?? 0.08} onChange={(e) => setLogo({ size: Number(e.target.value) })} />
          </Field>
          <Field label="Margin" hint="0.05 = title-safe edge">
            <input type="number" min={0} max={0.4} step={0.01} value={brand.logo.margin ?? 0.05} onChange={(e) => setLogo({ margin: Number(e.target.value) })} />
          </Field>
          <Field label="Opacity">
            <input type="number" min={0} max={1} step={0.05} value={brand.logo.opacity ?? 1} onChange={(e) => setLogo({ opacity: Number(e.target.value) })} />
          </Field>
          <Field label="Hide on scenes" hint="comma-separated ids">
            <input
              value={(brand.logo.hide_on ?? []).join(", ")}
              onChange={(e) => {
                const ids = e.target.value.split(",").map((w) => w.trim()).filter(Boolean);
                setLogo({ hide_on: ids.length ? ids : undefined });
              }}
            />
          </Field>
        </>
      )}

      <div className="section-title">Insert template scene</div>
      <div className="dim">Added after the selected scene; edit its text in the Scene tab.</div>
      <div className="template-grid">
        {CATALOG.map((t) => (
          <button
            key={t.name}
            className="btn small"
            title={t.description}
            data-insert-template={t.name}
            onClick={() => dispatch({ type: "insertTemplate", name: t.name, after })}
          >
            {t.label}
          </button>
        ))}
      </div>
    </div>
  );
};

const ParamField = ({ spec, value, onChange }: { spec: ParamSpec; value: unknown; onChange: (v: unknown) => void }) => {
  const label = spec.required ? `${spec.label} *` : spec.label;
  if (spec.type === "bool") {
    return (
      <label className="field check">
        <input type="checkbox" checked={value == null ? spec.default : Boolean(value)} onChange={(e) => onChange(e.target.checked)} />
        <span className="field-label">{label}</span>
      </label>
    );
  }
  if (spec.type === "lines") {
    const lines = Array.isArray(value) ? (value as string[]) : [];
    return (
      <Field label={label} hint={`one per line, up to ${spec.max_items}, ${spec.max_chars} characters each`}>
        <textarea
          rows={Math.max(2, lines.length + 1)}
          data-template-param={spec.name}
          value={lines.join("\n")}
          onChange={(e) => onChange(e.target.value.split("\n"))}
          onBlur={(e) => onChange(e.target.value.split("\n").map((l) => l.trim()).filter(Boolean))}
        />
      </Field>
    );
  }
  const text = value == null ? "" : String(value);
  const n = [...text].length;
  return (
    <Field label={label} hint={`${n}/${spec.max_chars}`}>
      {spec.max_chars > 80 ? (
        <textarea rows={3} data-template-param={spec.name} value={text} onChange={(e) => onChange(e.target.value)} />
      ) : (
        <input data-template-param={spec.name} value={text} onChange={(e) => onChange(e.target.value)} />
      )}
    </Field>
  );
};

/** The params of a template scene, as a form built from the catalog. */
export const TemplateParams = ({ index, template, dispatch }: { index: number; template: SceneTemplate; dispatch: Dispatch<Action> }) => {
  const info = templateInfo(template.name);
  if (!info) return <div className="field-error">Unknown template “{template.name}”.</div>;
  const params = template.params ?? {};
  return (
    <div className="subform" data-template-form={template.name}>
      <div className="section-title">Template: {info.label}</div>
      <div className="dim">{info.description}</div>
      {info.params.map((spec) => (
        <ParamField
          key={spec.name}
          spec={spec}
          value={params[spec.name]}
          onChange={(v) => dispatch({ type: "patchTemplate", index, params: { ...params, [spec.name]: v as never } })}
        />
      ))}
    </div>
  );
};
