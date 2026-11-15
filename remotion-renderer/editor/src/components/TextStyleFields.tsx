import type { CustomShadow, TextBackground, TextBorder, TextOutline, TextShadow, TextStyle } from "../../../src/types/timeline";
import { Field } from "./Inspector";

// Outline / box / shadow controls, shared by the Scene tab (scene text), the layer panel (text
// layers: outline and shadow, next to their own Background) and the Brand tab (the default).
// Each part is optional: unset follows the brand (or the built-in look), shown as a hint.
// Typing and slider drags in one field merge into one undo step (App.tsx fieldDispatch); a
// checkbox or select is one step.

// "border" alone (text layers, whose box has its own Background field) edits the box's border.
type Part = "outline" | "background" | "shadow" | "border";

const DEFAULT_OUTLINE: TextOutline = { color: "#000000", width: 4 };
const DEFAULT_BOX: TextBackground = { color: "#000000aa", padding: 0.4, radius: 0.3 };
const DEFAULT_BORDER: TextBorder = { color: "#ffffff", width: 2 };
const NO_FILL = "#00000000";
const DEFAULT_CUSTOM: CustomShadow = { color: "#000000b3", x: 0, y: 4, blur: 12 };

/** `#rgb`/`#rrggbbaa` to the `#rrggbb` a color picker needs, and back keeping the alpha. */
const toPicker = (c: string | undefined): string => {
  const m = /^#([0-9a-f]{3}|[0-9a-f]{6}|[0-9a-f]{8})$/i.exec(c ?? "");
  if (!m) return "#000000";
  const h = m[1].length === 3 ? m[1].replace(/./g, (x) => x + x) : m[1].slice(0, 6);
  return `#${h.toLowerCase()}`;
};
const keepAlpha = (picked: string, old: string | undefined): string => {
  const m = /^#[0-9a-f]{6}([0-9a-f]{2})$/i.exec(old ?? "");
  return m ? picked + m[1] : picked;
};

/** `style` with `part` set (or removed when `value` is undefined); null when nothing is left. */
export const withPart = (style: TextStyle | null | undefined, part: Part, value: unknown): TextStyle | null => {
  const next: Record<string, unknown> = { ...(style ?? {}) };
  if (value === undefined || value === null) delete next[part];
  else next[part] = value;
  for (const k of Object.keys(next)) if (next[k] == null) delete next[k];
  return Object.keys(next).length ? (next as TextStyle) : null;
};

const ColorInput = ({ label, value, onChange, testId }: { label: string; value: string; onChange: (c: string) => void; testId?: string }) => (
  <div className="row">
    <input type="color" aria-label={`${label} color`} value={toPicker(value)} onChange={(e) => onChange(keepAlpha(e.target.value, value))} />
    <input className="mono" aria-label={label} data-style-color={testId} value={value} onChange={(e) => onChange(e.target.value.trim())} />
  </div>
);

/** Border on/off, color and width of a box; `box` null creates a border-only box. */
const BorderFields = ({ name, box, onBox }: { name: string; box: TextBackground | null; onBox: (b: TextBackground | undefined) => void }) => {
  const border = box?.border ?? null;
  const setBorder = (b: TextBorder | null) => {
    if (b) onBox({ ...(box ?? {}), border: b });
    else if (box) {
      const rest: TextBackground = { ...box };
      delete rest.border;
      // A box that was only a frame (no fill) goes away with its border.
      onBox(rest.color && rest.color !== NO_FILL ? rest : undefined);
    }
  };
  return (
    <>
      <label className="check">
        <input type="checkbox" data-style-toggle={`${name}-border`} checked={border != null} onChange={(e) => setBorder(e.target.checked ? { ...DEFAULT_BORDER } : null)} />
        Border around the box
      </label>
      {border && (
        <div className="two-col">
          <Field label="Border color">
            <ColorInput label="Border" testId={`${name}-border`} value={border.color} onChange={(c) => setBorder({ ...border, color: c })} />
          </Field>
          <Field label="Border width" suffix={`${border.width ?? 2} px`} hint="at 1080p">
            <input
              type="range"
              min={0}
              max={16}
              step={0.5}
              data-style-range={`${name}-border-width`}
              value={border.width ?? 2}
              onChange={(e) => setBorder({ ...border, width: Number(e.target.value) })}
            />
          </Field>
        </div>
      )}
    </>
  );
};

const describeShadow = (s: TextShadow | null | undefined): string =>
  s == null ? "" : typeof s === "string" ? s : `custom ${s.color}`;

export const TextStyleFields = ({
  style,
  inherited,
  parts = ["outline", "background", "shadow"],
  builtinShadow,
  onChange,
  name,
}: {
  style: TextStyle | null | undefined;
  /** The brand's text style, for "follows the brand" hints (none on the Brand tab itself). */
  inherited?: TextStyle | null;
  parts?: Part[];
  /** What an unset shadow draws without a brand shadow ("soft" for the scene text). */
  builtinShadow: string;
  onChange: (part: Part, value: unknown) => void;
  /** Prefix of the data attributes (tests, screenshots). */
  name: string;
}) => {
  const outline = style?.outline ?? null;
  const box = style?.background ?? null;
  const shadow = style?.shadow ?? null;
  const from = (p: Exclude<Part, "border">) => (inherited?.[p] ? "brand" : "built-in");
  const shadowKind = shadow == null ? "default" : typeof shadow === "string" ? shadow : "custom";
  const custom = shadow != null && typeof shadow === "object" ? shadow : null;

  return (
    <div className="subform" data-text-style={name}>
      {parts.includes("outline") && (
        <>
          <label className="check">
            <input
              type="checkbox"
              data-style-toggle={`${name}-outline`}
              checked={outline != null}
              onChange={(e) => onChange("outline", e.target.checked ? { ...(inherited?.outline ?? DEFAULT_OUTLINE) } : undefined)}
            />
            Outline
            {outline == null && <span className="dim"> ({inherited?.outline ? `brand: ${inherited.outline.width ?? 4}px ${inherited.outline.color ?? "#000000"}` : "none"})</span>}
          </label>
          {outline && (
            <div className="two-col">
              <Field label="Outline color">
                <ColorInput label="Outline" testId={`${name}-outline`} value={outline.color ?? "#000000"} onChange={(c) => onChange("outline", { ...outline, color: c })} />
              </Field>
              <Field label="Width" suffix={`${outline.width ?? 4} px`} hint="at 1080p, 0 = off">
                <input
                  type="range"
                  min={0}
                  max={16}
                  step={0.5}
                  data-style-range={`${name}-outline-width`}
                  value={outline.width ?? 4}
                  onChange={(e) => onChange("outline", { ...outline, width: Number(e.target.value) })}
                />
              </Field>
            </div>
          )}
        </>
      )}

      {parts.includes("background") && (
        <>
          <label className="check">
            <input
              type="checkbox"
              data-style-toggle={`${name}-box`}
              checked={box != null}
              onChange={(e) => onChange("background", e.target.checked ? { ...(inherited?.background ?? DEFAULT_BOX) } : undefined)}
            />
            Box behind the text
            {box == null && <span className="dim"> ({inherited?.background ? `brand: ${inherited.background.color}` : "none"})</span>}
          </label>
          {box && (
            <>
              <label className="check">
                <input
                  type="checkbox"
                  data-style-toggle={`${name}-fill`}
                  checked={box.color != null && box.color !== NO_FILL}
                  onChange={(e) => onChange("background", { ...box, color: e.target.checked ? DEFAULT_BOX.color : NO_FILL })}
                />
                Fill
              </label>
              {box.color != null && box.color !== NO_FILL && (
                <Field label="Box color" hint="#rrggbbaa: last two digits = opacity">
                  <ColorInput label="Box" testId={`${name}-box`} value={box.color} onChange={(c) => onChange("background", { ...box, color: c })} />
                </Field>
              )}
              <div className="two-col">
                <Field label="Padding" suffix={`${box.padding ?? 0.4} em`}>
                  <input type="range" min={0} max={1.5} step={0.05} value={box.padding ?? 0.4} onChange={(e) => onChange("background", { ...box, padding: Number(e.target.value) })} />
                </Field>
                <Field label="Corner radius" suffix={`${box.radius ?? 0.3} em`}>
                  <input type="range" min={0} max={1.5} step={0.05} value={box.radius ?? 0.3} onChange={(e) => onChange("background", { ...box, radius: Number(e.target.value) })} />
                </Field>
              </div>
              <BorderFields name={name} box={box} onBox={(b) => onChange("background", b)} />
            </>
          )}
        </>
      )}

      {parts.includes("border") && <BorderFields name={name} box={box} onBox={(b) => onChange("background", b)} />}

      {parts.includes("shadow") && (
        <>
          <Field label="Shadow">
            <select
              data-style-shadow={name}
              value={shadowKind}
              onChange={(e) => {
                const v = e.target.value;
                onChange("shadow", v === "default" ? undefined : v === "custom" ? { ...DEFAULT_CUSTOM } : v);
              }}
            >
              <option value="default">
                {from("shadow") === "brand" ? `brand (${describeShadow(inherited?.shadow)})` : `default (${builtinShadow})`}
              </option>
              <option value="none">none</option>
              <option value="soft">soft</option>
              <option value="strong">strong</option>
              <option value="custom">custom…</option>
            </select>
          </Field>
          {custom && (
            <>
              <Field label="Shadow color">
                <ColorInput label="Shadow" value={custom.color} onChange={(c) => onChange("shadow", { ...custom, color: c })} />
              </Field>
              <div className="two-col">
                {(["x", "y"] as const).map((k) => (
                  <Field key={k} label={k.toUpperCase()} suffix={`${custom[k] ?? 0} px`}>
                    <input type="range" min={-40} max={40} step={1} value={custom[k] ?? 0} onChange={(e) => onChange("shadow", { ...custom, [k]: Number(e.target.value) })} />
                  </Field>
                ))}
              </div>
              <Field label="Blur" suffix={`${custom.blur ?? 0} px`}>
                <input type="range" min={0} max={80} step={1} value={custom.blur ?? 0} onChange={(e) => onChange("shadow", { ...custom, blur: Number(e.target.value) })} />
              </Field>
            </>
          )}
        </>
      )}
    </div>
  );
};
