import { fraction, type RenderView } from "../render";

type Props = {
  view: RenderView;
  onCancel: () => void;
  onReveal: (path: string) => void;
  onDismiss: () => void;
};

// Floating card in the corner: progress while rendering, then the result.
export const RenderPanel = ({ view, onCancel, onReveal, onDismiss }: Props) => {
  if (view.status === "idle") return null;
  const pct = Math.round(fraction(view) * 100);

  return (
    <div className="render-card glass" role="status" aria-live="polite">
      {view.status === "running" && (
        <>
          <div className="render-title">Rendering</div>
          <div className="render-stage dim">{view.stage}{view.chunks && view.chunks.total > 0 ? ` · segment ${Math.min(view.chunks.done + 1, view.chunks.total)} of ${view.chunks.total}` : ""}</div>
          <div className="render-bar" aria-label={`${pct} percent`}>
            <span style={{ width: `${pct}%` }} />
          </div>
          <div className="render-foot">
            <span className="mono dim">{pct}%</span>
            <span className="spacer" />
            <button className="btn small danger" onClick={onCancel}>Cancel</button>
          </div>
        </>
      )}
      {view.status === "done" && (
        <>
          <div className="render-title ok">Render complete</div>
          <div className="render-stage mono dim">{view.output}</div>
          <div className="render-foot">
            <button className="btn small" onClick={() => onReveal(view.output)}>Show in Finder</button>
            <span className="spacer" />
            <button className="btn small ghost" onClick={onDismiss}>Close</button>
          </div>
        </>
      )}
      {view.status === "cancelled" && (
        <>
          <div className="render-title">Render cancelled</div>
          <div className="render-foot">
            <span className="spacer" />
            <button className="btn small ghost" onClick={onDismiss}>Close</button>
          </div>
        </>
      )}
      {view.status === "failed" && (
        <>
          <div className="render-title bad">Render failed</div>
          <div className="render-stage">{view.error}</div>
          <div className="render-foot">
            <span className="spacer" />
            <button className="btn small ghost" onClick={onDismiss}>Close</button>
          </div>
        </>
      )}
    </div>
  );
};
