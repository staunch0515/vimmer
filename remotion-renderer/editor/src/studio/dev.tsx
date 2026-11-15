// Browser harness for the Studio screens, served by the editor's dev server at
// /src/studio/dev.html. It runs them against the in-memory fake (./fake.ts): no desktop shell,
// no disk, no API calls. Pick a screen with the hash: #home, #home-empty, #project.
import { useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import "../styles.css";
import "./studio.css";
import { StudioApiContext } from "./api";
import { createFakeApi } from "./fake";
import { Home } from "./Home";
import { ProjectSidebar } from "./ProjectSidebar";
import type { AssetInfo, OpenedProject, ProjectSummary } from "./types";

const Harness = () => {
  const empty = window.location.hash === "#home-empty";
  const api = useMemo(() => createFakeApi({ empty, generateError: window.location.hash === "#generate-error" ? "overloaded_error: the API is busy" : undefined }), [empty]);
  const [opened, setOpened] = useState<OpenedProject | null>(null);
  const [project, setProject] = useState<ProjectSummary | null>(null);
  const [timelineJson, setTimelineJson] = useState("");
  const [rendersKey, setRendersKey] = useState(0);
  const [log, setLog] = useState<string[]>([]);

  const open = (o: OpenedProject) => {
    setOpened(o);
    setProject(o.project);
    setTimelineJson(o.timelineJson);
  };
  const use = (asset: AssetInfo) => {
    // What the editor does with "use": here, the first scene's image.
    if (asset.kind !== "image") return setLog((l) => [`use ${asset.path} (${asset.kind})`, ...l]);
    const t = JSON.parse(timelineJson);
    t.scenes[0].image = asset.path;
    setTimelineJson(JSON.stringify(t, null, 2) + "\n");
    setLog((l) => [`scene-1 image = ${asset.path}`, ...l]);
  };

  return (
    <StudioApiContext.Provider value={api}>
      {opened && project ? (
        <div style={{ height: "100%", display: "grid", gridTemplateColumns: "280px 1fr" }}>
          <aside className="sidebar panel" style={{ borderRight: "1px solid var(--border)", minHeight: 0, display: "flex", flexDirection: "column" }}>
            <ProjectSidebar
              project={project}
              timelineJson={timelineJson}
              sceneLabel="scene-1"
              onUseAsset={use}
              rendersKey={rendersKey}
              onProjectChanged={setProject}
              onHome={() => setOpened(null)}
            />
          </aside>
          <main style={{ padding: 24, overflow: "auto" }}>
            <p className="dim">The editor goes here. This harness only shows the project sidebar.</p>
            <button className="btn" onClick={() => setRendersKey((k) => k + 1)}>
              Pretend a render finished
            </button>
            <pre className="mono dim" style={{ whiteSpace: "pre-wrap" }}>{log.join("\n")}</pre>
          </main>
        </div>
      ) : (
        <Home onOpen={open} onOpenFile={() => setLog((l) => ["open file", ...l])} />
      )}
    </StudioApiContext.Provider>
  );
};

createRoot(document.getElementById("root")!).render(<Harness />);
