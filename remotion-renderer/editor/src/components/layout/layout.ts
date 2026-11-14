// Size math for resizable panes. Pure functions (no DOM), so the tests can run them under node.
//
// A split has one sized pane and one pane that takes the rest. Sizes are in CSS pixels.
// What is remembered is the size together with the space it was chosen in (`basis`): when the
// window grows or shrinks, the pane keeps its share of the space, then the limits apply, so a
// pane never pushes its neighbour below the neighbour's minimum or past the window edge.

export type Limits = {
  /** Smallest open size. */
  min: number;
  /** Largest size. */
  max: number;
  /** Space the other pane keeps at least. */
  minRest: number;
  /** Dragging below `collapseBelow` collapses the pane (when `collapsible`). */
  collapsible?: boolean;
  collapseBelow?: number;
};

export type Stored = {
  /** The open size (kept while collapsed, so reopening restores it). */
  size: number;
  /** The space available when `size` was chosen; null = a plain pixel size (the default). */
  basis: number | null;
  collapsed: boolean;
};

export const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(v, hi));

/** The largest size that leaves the other pane its minimum (never below 0). */
export const maxSize = (limits: Limits, available: number): number =>
  Math.max(0, Math.min(limits.max, available - limits.minRest));

/**
 * A size within the limits for this much space. When the space is too small for both minimums,
 * the other pane wins, so the window edge is never crossed.
 */
export const clampSize = (size: number, limits: Limits, available: number): number => {
  const hi = maxSize(limits, available);
  return Math.round(clamp(size, Math.min(limits.min, hi), hi));
};

/** The remembered size scaled to the space there is now (keeps its proportion). */
export const scaledSize = (stored: Stored, available: number): number =>
  stored.basis && stored.basis > 0 && available > 0 ? (stored.size * available) / stored.basis : stored.size;

/** The open size for this much space (ignores collapse). */
export const openSize = (stored: Stored, limits: Limits, available: number): number =>
  clampSize(scaledSize(stored, available), limits, available);

/** What the pane shows: 0 while collapsed. */
export const shownSize = (stored: Stored, limits: Limits, available: number): number =>
  stored.collapsed ? 0 : openSize(stored, limits, available);

export const collapseThreshold = (limits: Limits): number => limits.collapseBelow ?? limits.min / 2;

/**
 * Where a drag ends up. `start` is the shown size when the drag began (0 if collapsed), `delta`
 * how far the pane grew, `reopenTo` the open size to keep when the drag collapses the pane.
 * Below the collapse threshold a collapsible pane collapses; dragging back past it reopens it.
 */
export const dragSize = (
  start: number,
  delta: number,
  limits: Limits,
  available: number,
  reopenTo: number,
): { size: number; collapsed: boolean } => {
  const raw = start + delta;
  if (limits.collapsible && raw < collapseThreshold(limits)) {
    return { size: clampSize(reopenTo, limits, available), collapsed: true };
  }
  return { size: clampSize(raw, limits, available), collapsed: false };
};

/** A state to remember: the size, chosen in this much space. */
export const commit = (size: number, collapsed: boolean, available: number): Stored => ({
  size: Math.round(size),
  basis: available > 0 ? Math.round(available) : null,
  collapsed,
});

/** Back to the default size, open. */
export const restore = (defaultSize: number): Stored => ({ size: defaultSize, basis: null, collapsed: false });

export const KEY_STEP = 10;
export const KEY_STEP_LARGE = 50;

export type KeyResult = { kind: "delta"; delta: number } | { kind: "min" } | { kind: "max" } | { kind: "toggle" } | null;

/**
 * A key on a separator. `axis` is the drag axis ("x" for side-by-side panes), `sign` +1 when
 * moving the separator right/down grows the pane (it is the first pane), -1 otherwise.
 * Enter toggles collapse, Home/End go to the smallest/largest size (the WAI-ARIA splitter keys).
 */
export const keyResize = (key: string, shift: boolean, axis: "x" | "y", sign: 1 | -1): KeyResult => {
  const step = shift ? KEY_STEP_LARGE : KEY_STEP;
  const dir =
    axis === "x"
      ? ({ ArrowLeft: -1, ArrowRight: 1 } as Record<string, number>)[key]
      : ({ ArrowUp: -1, ArrowDown: 1 } as Record<string, number>)[key];
  if (dir) return { kind: "delta", delta: dir * step * sign };
  if (key === "Home") return { kind: "min" };
  if (key === "End") return { kind: "max" };
  if (key === "Enter") return { kind: "toggle" };
  return null;
};

// Persistence: one localStorage entry per split id. Storage can be missing or throw (private
// mode, a full disk); then sizes simply are not remembered.
export const storageKey = (id: string) => `vimmer-layout:${id}`;

export const parseStored = (raw: string | null): Stored | null => {
  if (!raw) return null;
  try {
    const v = JSON.parse(raw) as Partial<Stored>;
    if (typeof v.size !== "number" || !Number.isFinite(v.size) || v.size < 0) return null;
    const basis = typeof v.basis === "number" && Number.isFinite(v.basis) && v.basis > 0 ? v.basis : null;
    return { size: v.size, basis, collapsed: v.collapsed === true };
  } catch {
    return null;
  }
};

type Store = Pick<Storage, "getItem" | "setItem" | "removeItem">;

export const loadStored = (store: Store | null, id: string): Stored | null => {
  try {
    return parseStored(store?.getItem(storageKey(id)) ?? null);
  } catch {
    return null;
  }
};

export const saveStored = (store: Store | null, id: string, stored: Stored | null): void => {
  try {
    if (stored) store?.setItem(storageKey(id), JSON.stringify(stored));
    else store?.removeItem(storageKey(id));
  } catch {
    /* not remembered; the layout still works */
  }
};
