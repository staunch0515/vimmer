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
  onAssets?: (dir: string, assets: AssetInfo[]) => void;
};

type Tab = "assets" | "renders";

/** The editor's left sidebar for a project: its files and its renders. */
export const ProjectSidebar = ({ project, timelineJson, sceneLabel, onUseAsset, assetsKey, rendersKey = 0, onProjectChanged, onHome, onAssets }: Props) => {
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
        <AssetPanel dir={project.dir} timelineJson={timelineJson} sceneLabel={sceneLabel} onUse={onUseAsset} refreshKey={assetsKey} onAssets={onAssets} />
      ) : (
        <RendersPanel dir={project.dir} refreshKey={rendersKey} onProjectChanged={onProjectChanged} />
      )}
    </div>
  );
};

type PlainProps = {
  /** The file's name, or "Untitled". */
  fileName: string;
  onHome: () => void;
  /** Makes a project from the saved file. Absent while there is nothing saved to import. */
  onMakeProject?: () => void;
};

/** The sidebar for a timeline file that is not in a project: where its files are, and the way to projects. */
export const PlainFileSidebar = ({ fileName, onHome, onMakeProject }: PlainProps) => (
  <div className="st-sidebar">
    <div className="st-sidebar-head">
      <button className="btn ghost small" onClick={onHome} title="All projects">
        ← Projects
      </button>
      <span className="st-sidebar-name" title={fileName}>
        {fileName}
      </span>
    </div>
    <div className="st-plain">
      <div className="section-label">Plain timeline file</div>
      <p className="dim">This timeline is not in a project. Its images, audio and clips are read from the app's shared public folder, and renders go where you save them.</p>
      <p className="dim">A project keeps its own assets and a history of its renders.</p>
      <button
        className="btn small"
        disabled={!onMakeProject}
        onClick={onMakeProject}
        title={onMakeProject ? "Copies the timeline and the files it uses into a new project" : "Save the file first"}
      >
        Make it a project…
      </button>
    </div>
  </div>
);
