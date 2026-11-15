import type { ReactNode } from "react";
import { Split, useMediaQuery } from "./Split";

type Props = {
  /** Left sidebar content (the project's assets). Empty: a placeholder. */
  left?: ReactNode;
  /** The preview. */
  center: ReactNode;
  /** The inspector. */
  right: ReactNode;
  /** The timeline (full width, under everything else). */
  bottom: ReactNode;
};

/** Below this width the inspector goes under the preview and the sidebar starts collapsed. */
export const NARROW_QUERY = "(max-width: 900px)";

const SidebarPlaceholder = () => (
  <div className="sidebar-placeholder">
    <div className="section-label">Project</div>
    <p className="dim">Project assets will show here.</p>
  </div>
);

/**
 * The studio shell: sidebar | preview | inspector on top, the timeline below, every divider
 * draggable. The narrow layout has its own split ids, so its sizes do not overwrite the wide ones.
 */
export const StudioLayout = ({ left, center, right, bottom }: Props) => {
  const narrow = useMediaQuery(NARROW_QUERY);
  const n = narrow ? "-narrow" : "";
  return (
    <Split
      id={`timeline${n}`}
      className="studio-shell"
      direction="column"
      side="end"
      defaultSize={narrow ? 200 : 260}
      min={110}
      minRest={narrow ? 320 : 240}
      collapsible
      collapseBelow={60}
      label="Resize timeline"
      reopenLabel="Show timeline"
    >
      <Split
        id={`sidebar${n}`}
        direction="row"
        side="start"
        defaultSize={240}
        min={160}
        max={480}
        minRest={narrow ? 320 : 600}
        collapsible
        defaultCollapsed={narrow}
        label="Resize sidebar"
        reopenLabel="Show sidebar"
      >
        <aside className="sidebar panel" aria-label="Project">{left ?? <SidebarPlaceholder />}</aside>
        <Split
          id={`inspector${n}`}
          direction={narrow ? "column" : "row"}
          side="end"
          defaultSize={narrow ? 300 : 320}
          min={narrow ? 160 : 260}
          max={narrow ? 900 : 760}
          minRest={narrow ? 180 : 360}
          collapsible
          keep="pixels"
          label="Resize inspector"
          reopenLabel="Show inspector"
        >
          {center}
          {right}
        </Split>
      </Split>
      {bottom}
    </Split>
  );
};
