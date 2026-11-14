// Undo history with coalescing. Pure (no runtime imports), so it is unit-tested with Node
// (test/editor-history.test.ts).
//
// One user gesture is one undo step:
// - an edit that changes nothing (same document) is not a step;
// - edits that carry the same merge key, each within MERGE_IDLE_MS of the one before, are one
//   step (typing in one field, dragging one slider); blur ends the run (`endMerge`);
// - every other edit is its own step (a drag commits once, when it ends; "add keyframe" is one
//   action that sets several fields).
// Selection changes never touch the history.

export const MERGE_IDLE_MS = 1000;
export const HISTORY_LIMIT = 100;

/** The edit run that the next edit with the same key joins. */
export type Merge = { key: string; at: number } | null;

export type Undoable<T> = {
  timeline: T;
  history: T[];
  future: T[];
  merge?: Merge;
};

/**
 * `next` is `prev` after one edit (its timeline and selection already updated, its history not).
 * Returns `next` with the history updated. `merge` is the edit's merge key and time, or null for an
 * edit that is always its own step. `same` says whether two documents are equal.
 */
export const record = <T, S extends Undoable<T>>(prev: S, next: S, merge: Merge, same: (a: T, b: T) => boolean): S => {
  if (next.timeline === prev.timeline || same(next.timeline, prev.timeline)) {
    // Nothing changed: keep the document object, so nothing downstream re-renders for it.
    return { ...next, timeline: prev.timeline, history: prev.history, future: prev.future, merge: prev.merge ?? null };
  }
  const joins = merge !== null && prev.merge != null && prev.merge.key === merge.key && merge.at - prev.merge.at <= MERGE_IDLE_MS;
  return {
    ...next,
    // A joined edit leaves the step's starting document on top of the history.
    history: joins ? prev.history : [...prev.history.slice(-(HISTORY_LIMIT - 1)), prev.timeline],
    future: [],
    merge,
  };
};

/** Ends the current edit run, so the next edit starts a new step. */
export const endMerge = <T, S extends Undoable<T>>(s: S): S => (s.merge ? { ...s, merge: null } : s);

export const undo = <T, S extends Undoable<T>>(s: S): S | null => {
  const prev = s.history[s.history.length - 1];
  if (prev === undefined) return null;
  return { ...s, timeline: prev, history: s.history.slice(0, -1), future: [s.timeline, ...s.future], merge: null };
};

export const redo = <T, S extends Undoable<T>>(s: S): S | null => {
  const next = s.future[0];
  if (next === undefined) return null;
  return { ...s, timeline: next, history: [...s.history, s.timeline], future: s.future.slice(1), merge: null };
};

/**
 * Whether an `input` event from this element is typing-like (keystrokes, a slider or color drag)
 * and should merge into one step, rather than a discrete choice (checkbox, select, file).
 */
export const mergesInput = (tag: string, type: string | null, contentEditable = false): boolean => {
  const t = tag.toLowerCase();
  if (t === "textarea" || contentEditable) return true;
  if (t !== "input") return false;
  return ["", "text", "number", "range", "color", "search", "url", "email", "tel"].includes((type ?? "").toLowerCase());
};

/** Merge key for an edit made from field `fieldId`: the field plus what the edit targets. */
export const mergeKey = (fieldId: number, action: { type: string } & Record<string, unknown>): string => {
  const target = ["sceneIndex", "index", "keyframe"].map((k) => (typeof action[k] === "number" ? action[k] : "")).join(",");
  return `${fieldId}:${action.type}:${target}`;
};
