// The scope of an assistant message: 全部 (all scenes), 选几幕 (several chosen scenes) or
// 当前这一幕 (the scene selected on the timeline). Pure logic, tested on its own.
//
// How each is sent with today's backend (one optional scene id per request):
// - all:      no scene id, the text as typed ({场景} becomes 「所有场景」).
// - one:      scene id = that scene (what the old "about the selected scene" checkbox did).
//             With no scene selected it falls back to all, as the disabled checkbox did,
//             unless the text needs a scene ({场景}): then nothing is sent.
// - selected: no scene id; one scope line is put before the text («只修改这些场景：…。其它场景不要改。»)
//             and {场景} becomes the same list.
// After an answer, change-summary lines of scenes outside the scope are reported.

export type SceneRef = { index: number; id: string };

export type ScopeMode = "all" | "selected" | "one";
export const SCOPE_MODES: readonly ScopeMode[] = ["all", "selected", "one"];
export const isScopeMode = (v: unknown): v is ScopeMode => typeof v === "string" && (SCOPE_MODES as readonly string[]).includes(v);

export const SCOPE_LABEL: Record<ScopeMode, string> = { all: "全部", selected: "选几幕", one: "当前这一幕" };

/** What {场景} becomes for 全部. */
export const ALL_SCENES = "所有场景";

const PLACEHOLDER = /\{\s*(?:scene|场景)\s*\}/gi;

export const needsScene = (text: string): boolean => new RegExp(PLACEHOLDER.source, "i").test(text);

/** How a scene is named in an instruction: 「第 3 幕 (scene-03)」 (number as on the timeline, plus the id). */
export const sceneLabel = (index: number, id: string): string => `第 ${index + 1} 幕 (${id})`;

/** Replaces every {场景} / {scene} with `replacement`. */
export const fillWith = (text: string, replacement: string): string => text.replace(PLACEHOLDER, () => replacement);

/** 「第 2 幕 (scene-02)、第 3 幕 (scene-03)」, in timeline order. */
export const sceneList = (scenes: SceneRef[]): string =>
  [...scenes].sort((a, b) => a.index - b.index).map((s) => sceneLabel(s.index, s.id)).join("、");

/** The line put before the text for 选几幕. */
export const scopeLine = (scenes: SceneRef[]): string => `只修改这些场景：${sceneList(scenes)}。其它场景不要改。`;

export type ScopeRequest = {
  mode: ScopeMode;
  text: string;
  /** The scene selected on the timeline, if any. */
  current: SceneRef | null;
  /** The scenes ticked for 选几幕. */
  chosen: SceneRef[];
};

export type Resolved = {
  /** The mode actually used (one without a selected scene becomes all). */
  mode: ScopeMode;
  /** What is sent as the instruction. */
  instruction: string;
  sceneId: string | null;
  /** The line put before the text (selected only), shown dimmed in the chat. */
  line: string | null;
  /** The scenes the message is limited to; null = all. */
  scopeIds: string[] | null;
};

export type ResolveError = "needsScene" | "noScenesChosen";

export const SCOPE_ERROR: Record<ResolveError, string> = {
  needsScene: "需要一个场景：先在时间线上选中一幕，或把范围改成「全部」/「选几幕」 · Select a scene first.",
  noScenesChosen: "「选几幕」还没有选场景：在下面勾选要改的几幕 · Tick the scenes to change.",
};

export const resolveScope = (r: ScopeRequest): { ok: true; value: Resolved } | { ok: false; error: ResolveError } => {
  const text = r.text.trim();
  if (r.mode === "selected") {
    if (r.chosen.length === 0) return { ok: false, error: "noScenesChosen" };
    const line = scopeLine(r.chosen);
    const body = fillWith(text, sceneList(r.chosen));
    const ids = [...r.chosen].sort((a, b) => a.index - b.index).map((s) => s.id);
    return { ok: true, value: { mode: "selected", instruction: `${line}\n${body}`, sceneId: null, line, scopeIds: ids } };
  }
  if (r.mode === "one" && r.current) {
    const body = fillWith(text, sceneLabel(r.current.index, r.current.id));
    return { ok: true, value: { mode: "one", instruction: body, sceneId: r.current.id, line: null, scopeIds: [r.current.id] } };
  }
  if (r.mode === "one" && needsScene(text)) return { ok: false, error: "needsScene" };
  // all, or one with no scene selected (sent like the old unticked checkbox).
  return { ok: true, value: { mode: "all", instruction: fillWith(text, ALL_SCENES), sceneId: null, line: null, scopeIds: null } };
};

/**
 * Scenes a proposal changes outside the scope: summary lines «scene-07: …» whose id was a
 * scene when the message was sent (`knownIds`) and is not in `scopeIds`. New scenes (a split)
 * and timeline-wide lines (format, total length, music…) are not counted. Each id once, in order.
 */
export const outOfScope = (summary: string[], scopeIds: string[] | null, knownIds: string[]): string[] => {
  if (!scopeIds) return [];
  const inScope = new Set(scopeIds);
  const known = new Set(knownIds);
  const out: string[] = [];
  for (const line of summary) {
    const id = /^([^\s:]+):\s/.exec(line)?.[1];
    if (id && known.has(id) && !inScope.has(id) && !out.includes(id)) out.push(id);
  }
  return out;
};

export const outOfScopeWarning = (ids: string[]): string => `这个方案改动了范围以外的场景：${ids.join("、")} · This proposal changes scenes outside the scope.`;

/** The scenes for the 选几幕 list: number, id and the first words of the text. */
export type SceneOption = SceneRef & { preview: string };

export const sceneOptions = (scenes: { id: string; text?: string | null }[], words = 10): SceneOption[] =>
  scenes.map((s, index) => {
    const t = (s.text ?? "").trim().replace(/\s+/g, " ");
    const chars = Array.from(t);
    return { index, id: s.id, preview: chars.length <= words ? t : `${chars.slice(0, words).join("")}…` };
  });

/** Keeps only chosen ids that are still scenes (after scenes were removed or renamed). */
export const pruneChosen = (chosen: string[], scenes: { id: string }[]): string[] => {
  const ids = new Set(scenes.map((s) => s.id));
  return chosen.filter((id) => ids.has(id));
};

// ---- {场景} for a single scene, and back -------------------------------------------------

export type Filled = { ok: true; text: string } | { ok: false; reason: "needsScene" };

/** Replaces {场景} / {scene} with one scene's label. Without a scene, a text that needs one is refused. */
export const fillPlaceholders = (text: string, scene: SceneRef | null): Filled => {
  if (!needsScene(text)) return { ok: true, text };
  if (!scene) return { ok: false, reason: "needsScene" };
  return { ok: true, text: fillWith(text, sceneLabel(scene.index, scene.id)) };
};

const LABEL = String.raw`第 \d+ 幕 \([A-Za-z0-9_-]{1,64}\)`;

/**
 * The reverse, for saving a sent message: scene labels made by `sceneLabel` («第 3 幕 (scene-03)»,
 * or a list of them joined by 「、」) become {场景} again, so the saved prompt works for any
 * scene. Other text is left alone.
 */
export const generalizeScene = (text: string): string => text.replace(new RegExp(`${LABEL}(?:、${LABEL})*`, "g"), "{场景}");
