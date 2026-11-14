import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type CSSProperties,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
} from "react";
import {
  clampSize,
  commit,
  dragSize,
  keyResize,
  loadStored,
  maxSize,
  openSize,
  restore,
  saveStored,
  shownSize,
  type KeyResult,
  type Limits,
  type Stored,
} from "./layout";

const storage = (): Storage | null => {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
};

/** Width of the visible divider line; the hit area around it is wider (CSS). */
const LINE = 1;

/**
 * A remembered pane size, loaded per id from localStorage. When `id` changes (the narrow layout
 * uses its own ids) the entry for the new id is loaded without remounting anything.
 */
export const usePersistedSize = (id: string, defaultSize: number, defaultCollapsed = false) => {
  const load = useCallback(
    (): Stored => loadStored(storage(), id) ?? { ...restore(defaultSize), collapsed: defaultCollapsed },
    [id, defaultSize, defaultCollapsed],
  );
  const [entry, setEntry] = useState(() => ({ id, stored: load() }));
  let stored = entry.stored;
  if (entry.id !== id) {
    stored = load();
    setEntry({ id, stored });
  }
  const save = useCallback(
    (next: Stored | null) => {
      saveStored(storage(), id, next);
      setEntry({ id, stored: next ?? { ...restore(defaultSize), collapsed: defaultCollapsed } });
    },
    [id, defaultSize, defaultCollapsed],
  );
  return [stored, save] as const;
};

export const useMediaQuery = (query: string): boolean =>
  useSyncExternalStore(
    (onChange) => {
      const m = window.matchMedia(query);
      m.addEventListener("change", onChange);
      return () => m.removeEventListener("change", onChange);
    },
    () => window.matchMedia(query).matches,
    () => false,
  );

type HandleProps = {
  /** The drag axis: "x" for side-by-side panes (a vertical line), "y" for stacked panes. */
  axis: "x" | "y";
  /** +1 when moving the line right/down grows the pane, -1 when it shrinks it. */
  sign: 1 | -1;
  value: number;
  min: number;
  max: number;
  label: string;
  controls?: string;
  collapsed?: boolean;
  /** Which side of the line the sized pane is on (where the reopen tab points). */
  side?: "start" | "end";
  reopenLabel?: string;
  /** How much the pane grew since the drag started (already signed). */
  onDrag: (delta: number) => void;
  onDragStart?: () => void;
  onDragEnd: () => void;
  onKey: (r: Exclude<KeyResult, null>) => void;
  /** Double-click: back to the default size. */
  onReset: () => void;
  onReopen?: () => void;
  className?: string;
  style?: CSSProperties;
};

/**
 * A draggable divider. Pointer capture keeps the drag alive over the video player and outside
 * the window; arrow keys resize (Shift for larger steps), Home/End go to the limits, Enter
 * toggles collapse and double-click restores the default size.
 */
export const ResizeHandle = (p: HandleProps) => {
  const start = useRef<number | null>(null);
  const [active, setActive] = useState(false);
  const coord = (e: ReactPointerEvent) => (p.axis === "x" ? e.clientX : e.clientY);

  const end = () => {
    if (start.current === null) return;
    start.current = null;
    setActive(false);
    document.body.classList.remove("split-dragging", "split-x", "split-y");
    p.onDragEnd();
  };
  // A handle unmounted mid-drag must not leave the page in drag mode.
  useEffect(() => () => document.body.classList.remove("split-dragging", "split-x", "split-y"), []);

  return (
    <div
      className={`split-handle ${p.axis}${active ? " active" : ""}${p.collapsed ? " collapsed" : ""} ${p.className ?? ""}`}
      style={p.style}
      role="separator"
      aria-orientation={p.axis === "x" ? "vertical" : "horizontal"}
      aria-label={p.label}
      aria-controls={p.controls}
      aria-valuenow={Math.round(p.value)}
      aria-valuemin={Math.round(p.min)}
      aria-valuemax={Math.round(p.max)}
      aria-valuetext={p.collapsed ? "collapsed" : `${Math.round(p.value)} pixels`}
      tabIndex={0}
      title={`${p.label} · double-click to reset`}
      onPointerDown={(e) => {
        if (e.button !== 0) return;
        e.preventDefault();
        e.stopPropagation();
        e.currentTarget.setPointerCapture(e.pointerId);
        start.current = coord(e);
        setActive(true);
        document.body.classList.add("split-dragging", p.axis === "x" ? "split-x" : "split-y");
        p.onDragStart?.();
      }}
      onPointerMove={(e) => {
        if (start.current === null) return;
        e.stopPropagation();
        p.onDrag((coord(e) - start.current) * p.sign);
      }}
      onPointerUp={end}
      onPointerCancel={end}
      onLostPointerCapture={end}
      onDoubleClick={(e) => {
        e.stopPropagation();
        p.onReset();
      }}
      onKeyDown={(e: ReactKeyboardEvent) => {
        const r = keyResize(e.key, e.shiftKey, p.axis, p.sign);
        if (!r) return;
        e.preventDefault();
        e.stopPropagation();
        p.onKey(r);
      }}
    >
      {p.collapsed && p.onReopen && (
        <button
          className={`split-reopen ${p.side ?? "start"}`}
          aria-label={p.reopenLabel ?? "Show panel"}
          title={p.reopenLabel ?? "Show panel"}
          onPointerDown={(e) => e.stopPropagation()}
          onDoubleClick={(e) => e.stopPropagation()}
          onClick={(e) => {
            e.stopPropagation();
            p.onReopen?.();
          }}
        >
          {chevron(p.axis, p.side ?? "start")}
        </button>
      )}
    </div>
  );
};

const chevron = (axis: "x" | "y", side: "start" | "end") =>
  axis === "x" ? (side === "start" ? "›" : "‹") : side === "start" ? "⌄" : "⌃";

export type SplitProps = {
  /** localStorage key (per split). */
  id: string;
  /** "row": panes side by side, "column": stacked. */
  direction: "row" | "column";
  /** Which child has the size; the other takes the rest. */
  side?: "start" | "end";
  defaultSize: number;
  min: number;
  max?: number;
  /** Space the other pane keeps at least. */
  minRest?: number;
  collapsible?: boolean;
  /** Collapse when dragged below this (default: half of `min`). */
  collapseBelow?: number;
  defaultCollapsed?: boolean;
  /**
   * "ratio" (default): the pane keeps its share of the space when the window resizes.
   * "pixels": it keeps its width (only clamped), e.g. a side panel inside a split that moves.
   */
  keep?: "ratio" | "pixels";
  /** Separator label, e.g. "Resize timeline". */
  label: string;
  reopenLabel?: string;
  className?: string;
  children: [ReactNode, ReactNode];
};

export const Split = ({
  id,
  direction,
  side = "start",
  defaultSize,
  min,
  max = Number.POSITIVE_INFINITY,
  minRest = 0,
  collapsible = false,
  collapseBelow,
  defaultCollapsed = false,
  keep = "ratio",
  label,
  reopenLabel,
  className,
  children,
}: SplitProps) => {
  const axis = direction === "row" ? "x" : "y";
  const sign: 1 | -1 = side === "start" ? 1 : -1;
  const limits: Limits = { min, max, minRest, collapsible, collapseBelow };
  const [stored, save] = usePersistedSize(id, defaultSize, defaultCollapsed);
  // The size while a drag is under way; committed (and remembered) when it ends.
  const [live, setLiveState] = useState<{ size: number; collapsed: boolean } | null>(null);
  const liveRef = useRef(live);
  const setLive = (l: typeof live) => {
    liveRef.current = l;
    setLiveState(l);
  };
  const drag = useRef<{ start: number; reopenTo: number } | null>(null);
  const container = useRef<HTMLDivElement>(null);
  const [available, setAvailable] = useState(0);
  const paneId = useId();

  // The space to share, followed through window and parent resizes.
  useLayoutEffect(() => {
    const el = container.current;
    if (!el) return;
    const measure = () => setAvailable(Math.max(0, (axis === "x" ? el.clientWidth : el.clientHeight) - LINE));
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [axis]);

  const measured = available > 0;
  const open = measured ? openSize(stored, limits, available) : stored.size;
  const collapsed = live ? live.collapsed : stored.collapsed;
  const shown = live ? (live.collapsed ? 0 : live.size) : measured ? shownSize(stored, limits, available) : stored.collapsed ? 0 : stored.size;
  const hi = measured ? maxSize(limits, available) : max;

  const keepAs = (size: number, isCollapsed: boolean) =>
    save(keep === "ratio" ? commit(size, isCollapsed, available) : { ...commit(size, isCollapsed, available), basis: null });

  const onKey = (r: Exclude<KeyResult, null>) => {
    if (r.kind === "toggle") {
      if (collapsible) keepAs(open, !collapsed);
      return;
    }
    if (r.kind === "min") return keepAs(collapsible ? open : clampSize(min, limits, available), collapsible);
    if (r.kind === "max") return keepAs(hi, false);
    const next = dragSize(shown, r.delta, limits, available, open);
    keepAs(next.size, next.collapsed);
  };

  const sized = (
    <div
      id={paneId}
      className={`split-pane sized${collapsed ? " collapsed" : ""}`}
      style={{ flex: `0 0 ${shown}px`, display: collapsed ? "none" : undefined }}
    >
      {side === "start" ? children[0] : children[1]}
    </div>
  );
  const rest = <div className="split-pane rest">{side === "start" ? children[1] : children[0]}</div>;
  const handle = (
    <ResizeHandle
      axis={axis}
      sign={sign}
      value={shown}
      min={collapsible ? 0 : Math.min(min, hi)}
      max={hi}
      label={label}
      controls={paneId}
      collapsed={collapsed}
      side={side}
      reopenLabel={reopenLabel}
      onDragStart={() => {
        drag.current = { start: shown, reopenTo: open };
      }}
      onDrag={(delta) => {
        const d = drag.current;
        if (!d) return;
        setLive(dragSize(d.start, delta, limits, available, d.reopenTo));
      }}
      onDragEnd={() => {
        drag.current = null;
        const l = liveRef.current;
        if (l) keepAs(l.size, l.collapsed);
        setLive(null);
      }}
      onKey={onKey}
      onReset={() => {
        setLive(null);
        save(restore(defaultSize));
      }}
      onReopen={collapsible ? () => keepAs(open, false) : undefined}
    />
  );

  return (
    <div ref={container} className={`split split-${direction}${className ? ` ${className}` : ""}`} data-split={id}>
      {side === "start" ? (
        <>
          {sized}
          {handle}
          {rest}
        </>
      ) : (
        <>
          {rest}
          {handle}
          {sized}
        </>
      )}
    </div>
  );
};
