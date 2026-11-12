type Props = {
  fileName: string;
  dirty: boolean;
  problemCount: number;
  format: { width: number; height: number };
  formats: { width: number; height: number; label: string }[];
  canUndo: boolean;
  canRedo: boolean;
  onNew: () => void;
  onOpen: () => void;
  onSave: () => void;
  onSaveAs: () => void;
  onUndo: () => void;
  onRedo: () => void;
  onResolution: (w: number, h: number) => void;
  onRender: () => void;
  rendering: boolean;
};

export const Toolbar = (p: Props) => {
  const current = `${p.format.width}x${p.format.height}`;
  return (
    <header className="toolbar glass">
      <div className="brand">
        <span className="logo" aria-hidden />
        <span>Vimmer</span>
        <span className="brand-sub">Studio</span>
      </div>
      <div className="divider" />
      <button className="btn ghost" onClick={p.onNew} title="New (⌘N)">New</button>
      <button className="btn ghost" onClick={p.onOpen} title="Open (⌘O)">Open</button>
      <button className="btn ghost" onClick={p.onSave} title="Save (⌘S)">Save</button>
      <button className="btn ghost" onClick={p.onSaveAs} title="Save as (⇧⌘S)">Save as</button>
      <div className="divider" />
      <button className="btn ghost" onClick={p.onUndo} disabled={!p.canUndo} title="Undo (⌘Z)">Undo</button>
      <button className="btn ghost" onClick={p.onRedo} disabled={!p.canRedo} title="Redo (⇧⌘Z)">Redo</button>
      <div className="divider" />
      <label className="field-inline">
        <span className="dim">Format</span>
        <select value={current} onChange={(e) => {
          const [w, h] = e.target.value.split("x").map(Number);
          p.onResolution(w, h);
        }}>
          {p.formats.map((r) => (
            <option key={r.label} value={`${r.width}x${r.height}`}>{r.label}</option>
          ))}
        </select>
      </label>
      <span className="file-name" title={p.fileName}>
        {p.fileName}
        {p.dirty && <span className="dirty" aria-label="unsaved changes"> •</span>}
      </span>
      <span className="spacer" />
      <span className={`status ${p.problemCount ? "warn" : "ok"}`}>
        <span className="dot" />
        {p.problemCount ? `${p.problemCount} issue${p.problemCount > 1 ? "s" : ""}` : "Ready to render"}
      </span>
      <button className="btn primary" onClick={p.onRender} disabled={p.rendering} title="Render to MP4">
        {p.rendering ? "Rendering…" : "Render"}
      </button>
    </header>
  );
};
