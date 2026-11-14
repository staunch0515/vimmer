import { useState } from "react";
import { AssetPanel } from "./AssetPanel";
import { RendersPanel } from "./RendersPanel";
import type { AssetInfo, ProjectSummary } from "./types";

type Props = {
  project: ProjectSummary;
  /** The editor's current document. */
  timelineJson: string;
  sceneLabel?: string | null;
  onUseAsset?: (asset: AssetInfo) => void;
  /** Bumped when project files changed outside the sidebar. */
  assetsKey?: number;
  /** Bumped when a render finished; also switches to the Renders tab. */
  rendersKey?: number;
  onProjectChanged?: (project: ProjectSummary) => void;
  /** Leaves the editor for the Home screen. */
  onHome?: () => void;
};

type Tab = "assets" | "renders";

/** The editor's left sidebar for a project: its files and its renders. */
export const ProjectSidebar = ({ project, timelineJson, sceneLabel, onUseAsset, assetsKey, rendersKey = 0, onProjectChanged, onHome }: Props) => {
  const [tab, setTab] = useState<Tab>("assets");
  // A finished render brings its history forward once.
  const [seenRenders, setSeenRenders] = useState(rendersKey);
  if (rendersKey !== seenRenders) {
    setSeenRenders(rendersKey);
    setTab("renders");
  }
  return (
    <div className="st-sidebar">
      <div className="st-sidebar-head">
        {onHome && (
          <button className="btn ghost small" onClick={onHome} title="All projects">
            ← Projects
          </button>
        )}
        <span className="st-sidebar-name" title={project.dir}>
          {project.name}
        </span>
      </div>
      <div className="tabs" role="tablist">
        <button role="tab" aria-selected={tab === "assets"} className={`tab${tab === "assets" ? " active" : ""}`} onClick={() => setTab("assets")}>
          Assets
        </button>
        <button role="tab" aria-selected={tab === "renders"} className={`tab${tab === "renders" ? " active" : ""}`} onClick={() => setTab("renders")}>
          Renders{project.renders > 0 ? ` ${project.renders}` : ""}
        </button>
      </div>
      {tab === "assets" ? (
        <AssetPanel dir={project.dir} timelineJson={timelineJson} sceneLabel={sceneLabel} onUse={onUseAsset} refreshKey={assetsKey} />
      ) : (
        <RendersPanel dir={project.dir} refreshKey={rendersKey} onProjectChanged={onProjectChanged} />
      )}
    </div>
  );
};
