import type { Look, Overlay, OverlayKind } from "../../../src/types/timeline";
import { MAX_OVERLAYS, lookProblems } from "../../../src/motion/look";
import {
  DEFAULT_TINT,
  GRADE_CHOICES,
  OVERLAY_CHOICES,
  addOverlay,
  patchOverlay,
  removeOverlay,
  takesColor,
  withOverlays,
  withPreset,
  withStrength,
} from "../look";
import { Field } from "./Inspector";

const pct = (v: number) => `${Math.round(v * 100)}%`;
const gradeLabel = (preset: string | undefined) => GRADE_CHOICES.find((c) => c.value === preset)?.label ?? preset ?? "";
const overlayLabel = (kind: OverlayKind) => OVERLAY_CHOICES.find((c) => c.value === kind)?.label ?? kind;

/**
 * The 画面氛围 Look fields: a grade preset with its strength, and a list of overlays.
 * - `scope="scene"`: unset parts inherit the timeline default (`inherited`); "own overlays" replaces
 *   the default list (an empty own list = none in this scene).
 * - `scope="timeline"`: the default of every scene.
 * Sliders merge into one undo step per drag (the editor's field merging); selects and buttons are
 * one step each.
 */
export const LookFields = ({
  look,
  onChange,
  scope,
  inherited,
  autoAs,
}: {
  look: Look | null | undefined;
  onChange: (look: Look | undefined) => void;
  scope: "scene" | "timeline";
  inherited?: Look | null;
  /** What `auto` stands for now (from the music mood). */
  autoAs: string;
}) => {
  const grade = look?.grade ?? null;
  const ownOverlays = look?.overlays ?? null;
  const shownOverlays = ownOverlays ?? (scope === "scene" ? inherited?.overlays ?? [] : []);
  const editable = scope === "timeline" || ownOverlays !== null;
  const setList = (list: Overlay[]) => onChange(withOverlays(look, scope === "timeline" && list.length === 0 ? undefined : list));
  const problems = look ? lookProblems(look) : [];
  const autoHint = `auto = ${autoAs === "none" ? "none (no music mood)" : gradeLabel(autoAs)}`;
  const inheritedGrade = inherited?.grade;
  return (
    <div className="look-fields" data-look={scope}>
      <Field label="调色 Grade" hint={grade?.preset === "auto" || (!grade && inheritedGrade?.preset === "auto") ? autoHint : undefined}>
        <select data-look-grade value={grade?.preset ?? ""} onChange={(e) => onChange(withPreset(look, e.target.value))}>
          <option value="">
            {scope === "scene"
              ? `继承默认 Inherit (${inheritedGrade ? gradeLabel(inheritedGrade.preset) : "none"})`
              : "— 不调色 No grade —"}
          </option>
          {GRADE_CHOICES.map((c) => (
            <option key={c.value} value={c.value}>{c.label}</option>
          ))}
        </select>
      </Field>
      {grade && grade.preset !== "none" && (
        <Field label="Strength" hint={pct(grade.strength ?? 1)}>
          <input
            type="range"
            data-look-strength
            min={0}
            max={1}
            step={0.05}
            value={grade.strength ?? 1}
            onChange={(e) => onChange(withStrength(look, Number(e.target.value)))}
          />
        </Field>
      )}
      {scope === "scene" && (
        <label className="check">
          <input
            type="checkbox"
            data-look-own-overlays
            checked={ownOverlays !== null}
            onChange={(e) => onChange(withOverlays(look, e.target.checked ? [...(inherited?.overlays ?? [])] : undefined))}
          />
          <span>Own overlays for this scene (replace the default)</span>
        </label>
      )}
      {!editable && (
        <div className="dim" data-look-inherited-overlays>
          {shownOverlays.length
            ? `Default: ${shownOverlays.map((o) => `${overlayLabel(o.kind)} ${pct(o.intensity ?? 0.5)}`).join(", ")}`
            : "Default: no overlays"}
        </div>
      )}
      {editable && (
        <>
          {shownOverlays.length === 0 && <div className="dim">{scope === "scene" ? "No overlays in this scene." : "No overlays."}</div>}
          {shownOverlays.map((o, i) => (
            <div className="look-overlay" key={i} data-look-overlay={i}>
              <div className="two-col">
                <Field label="Overlay">
                  <select
                    data-look-overlay-kind
                    value={o.kind}
                    onChange={(e) => setList(patchOverlay(shownOverlays, i, { kind: e.target.value as OverlayKind }))}
                  >
                    {OVERLAY_CHOICES.map((c) => (
                      <option key={c.value} value={c.value}>{c.label}</option>
                    ))}
                  </select>
                </Field>
                <Field label="Intensity" hint={pct(o.intensity ?? 0.5)}>
                  <input
                    type="range"
                    data-look-overlay-intensity
                    min={0}
                    max={1}
                    step={0.05}
                    value={o.intensity ?? 0.5}
                    onChange={(e) => setList(patchOverlay(shownOverlays, i, { intensity: Number(e.target.value) }))}
                  />
                </Field>
              </div>
              <div className="row">
                {takesColor(o.kind) && (
                  <>
                    <input
                      type="color"
                      data-look-overlay-color
                      value={o.color && /^#[0-9a-f]{6}$/i.test(o.color) ? o.color : DEFAULT_TINT[o.kind]}
                      onChange={(e) => setList(patchOverlay(shownOverlays, i, { color: e.target.value }))}
                    />
                    {o.color ? (
                      <button className="btn ghost small" onClick={() => setList(patchOverlay(shownOverlays, i, { color: undefined }))}>
                        Default tint
                      </button>
                    ) : (
                      <span className="dim">default tint</span>
                    )}
                  </>
                )}
                <button className="btn ghost small" data-look-overlay-remove onClick={() => setList(removeOverlay(shownOverlays, i))}>
                  Remove
                </button>
              </div>
            </div>
          ))}
          <button
            className="btn small"
            data-look-add-overlay
            disabled={shownOverlays.length >= MAX_OVERLAYS}
            onClick={() => setList(addOverlay(shownOverlays))}
          >
            + Add overlay
          </button>
        </>
      )}
      {problems.map((p) => (
        <span key={p} className="field-error">{p}</span>
      ))}
    </div>
  );
};
