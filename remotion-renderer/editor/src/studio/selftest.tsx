// End-to-end check of the Studio backend from inside the real desktop window. Development only:
// nothing links here. It runs every Studio command through Tauri's `invoke`, the way the screens
// do, in a projects folder given in the URL, and writes what happened to a JSON file.
//
//   cd desktop && npx tauri dev --config '{"identifier":"com.vimmer.studio.selftest","build":{
//     "beforeDevCommand":"","devUrl":"http://localhost:5195/src/studio/selftest.html?root=/tmp/x/projects&out=/tmp/x/result.json&image=/abs/pic.png&timeline=/abs/fixture.json"}}'
//
// The separate identifier keeps the app's real settings untouched. Options: `render=1` renders
// the project (minutes), `generate=1` makes one paid call (Sonnet, a three-sentence text, about
// one cent). After the checks the page shows the real Home screen, then the project sidebar
// (assets, then renders).
import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import "../styles.css";
import "./studio.css";
import { listenRender, type RenderFinished } from "../render";
import { DEFAULT_OPTIONS, FALLBACK_DEFAULTS } from "../renderOptions";
import { tauriApi as api } from "./api";
import { Home } from "./Home";
import { checkProjectBeforeRender, startProjectRender, type ProjectRenderFinished } from "./projectRender";
import { ProjectSidebar } from "./ProjectSidebar";
import type { GenerateFinished, ProjectSummary } from "./types";

type Step = { step: string; ok: boolean; detail: string };

const params = new URLSearchParams(window.location.search);
const need = (name: string): string => {
  const v = params.get(name);
  if (!v) throw new Error(`missing ?${name}=`);
  return v;
};

const report = async (steps: Step[], done: boolean) => {
  const out = params.get("out");
  if (!out) return;
  const body = JSON.stringify({ done, failed: steps.filter((s) => !s.ok).length, steps }, null, 2) + "\n";
  await invoke("write_project_file", { path: out, contents: body }).catch(() => undefined);
};

/** Loads a file through the asset protocol, as an <img> or <video> in the screens would. */
const loads = async (path: string): Promise<string> => {
  const res = await fetch(api.fileUrl(path));
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  const bytes = (await res.arrayBuffer()).byteLength;
  if (bytes === 0) throw new Error("empty");
  return `${bytes} bytes`;
};

const videoPlays = (path: string): Promise<string> =>
  new Promise((resolve, reject) => {
    const v = document.createElement("video");
    v.muted = true;
    v.onloadedmetadata = () => resolve(`${v.videoWidth}x${v.videoHeight}, ${v.duration.toFixed(2)} s`);
    v.onerror = () => reject(new Error("the video element could not load it"));
    v.src = api.fileUrl(path);
  });

const run = async (push: (s: Step) => void, setProject: (p: ProjectSummary, timelineJson: string) => void) => {
  const step = async (name: string, work: () => Promise<string>): Promise<boolean> => {
    try {
      push({ step: name, ok: true, detail: await work() });
      return true;
    } catch (e) {
      push({ step: name, ok: false, detail: e instanceof Error ? e.message : String(e) });
      return false;
    }
  };
  const expect = (cond: boolean, message: string) => {
    if (!cond) throw new Error(message);
  };

  let dir = "";
  let timelineJson = "";
  await step("set the projects folder", async () => {
    const s = await api.setProjectsRoot(need("root"));
    expect(s.root === need("root"), `root is ${s.root}`);
    return s.root;
  });
  await step("create a project", async () => {
    const p = await api.createProject("Selftest", "en", "landscape");
    dir = p.dir;
    expect(p.durationMs === 4000 && p.origin === "blank", JSON.stringify(p));
    return p.dir;
  });
  let imagePath = "";
  await step("import an image", async () => {
    const got = await api.importAssets(dir, [need("image")]);
    imagePath = got[0].path;
    expect(imagePath.startsWith("assets/") && !got[0].reused, JSON.stringify(got));
    const again = await api.importAssets(dir, [need("image")]);
    expect(again[0].reused && again[0].path === imagePath, "the second import was not reused");
    return imagePath;
  });
  await step("use it in the first scene and save", async () => {
    const opened = await api.openProject(dir);
    const t = JSON.parse(opened.timelineJson);
    t.scenes[0].image = imagePath;
    t.scenes[0].duration_ms = 2000;
    timelineJson = JSON.stringify(t, null, 2) + "\n";
    const saved = await api.saveProject(dir, timelineJson);
    expect(saved.durationMs === 2000, `duration ${saved.durationMs}`);
    expect(saved.thumb !== null, "no thumbnail from the first scene image");
    setProject(saved, timelineJson);
    return `thumbnail ${saved.thumb}`;
  });
  await step("the asset list shows it as used", async () => {
    const list = await api.listAssets(dir, timelineJson);
    const a = list.find((x) => x.path === imagePath);
    expect(!!a, "the imported file is not listed");
    expect(a!.usedBy.includes("scene-1"), `usedBy ${a!.usedBy}`);
    expect(a!.width !== null, "no dimensions");
    return `${a!.kind} ${a!.width}x${a!.height}, used by ${a!.usedBy.join(", ")}; loads: ${await loads(a!.file)}`;
  });
  await step("the thumbnail loads in the webview", async () => {
    const p = (await api.listProjects()).find((x) => x.dir === dir)!;
    return loads(p.thumb!);
  });
  await step("the pre-render check finds the project's file", async () => {
    const report = await checkProjectBeforeRender(dir, timelineJson, "say");
    const missing = report.findings.filter((f) => f.code.startsWith("assets.missing"));
    expect(missing.length === 0, JSON.stringify(missing));
    return `${report.findings.length} finding(s), none about a missing file`;
  });

  if (params.get("render") === "1") {
    await step("render into the project", async () => {
      const finished = await new Promise<ProjectRenderFinished>((resolve, reject) => {
        void listenRender(
          () => undefined,
          (f: RenderFinished) => resolve(f as ProjectRenderFinished),
        ).then(() => startProjectRender(dir, timelineJson, DEFAULT_OPTIONS, FALLBACK_DEFAULTS).catch(reject));
      });
      expect(finished.ok, finished.error ?? "cancelled");
      expect(!!finished.project && finished.project.renders === 1, "the render is not in the project's history");
      setProject(finished.project!, timelineJson);
      return `${finished.output}${finished.notes?.length ? ` · ${finished.notes.join(" · ")}` : ""}`;
    });
    await step("the render is in the history and plays", async () => {
      const renders = await api.listRenders(dir);
      expect(renders.length === 1 && !renders[0].missing, JSON.stringify(renders));
      expect(renders[0].file.startsWith("renders/") && renders[0].bytes > 0 && renders[0].durationMs > 1500, JSON.stringify(renders[0]));
      return `${renders[0].file}, ${renders[0].bytes} bytes, ${renders[0].durationMs} ms; video: ${await videoPlays(renders[0].path)}`;
    });
    await step("Home shows the render's thumbnail", async () => {
      const p = (await api.listProjects()).find((x) => x.dir === dir)!;
      expect(p.renders === 1 && p.thumb !== null, JSON.stringify(p));
      return loads(p.thumb!);
    });
  }

  await step("rename, duplicate, delete the copy (to the Trash)", async () => {
    const renamed = await api.renameProject(dir, "Selftest renamed");
    dir = renamed.dir;
    const copy = await api.duplicateProject(dir);
    expect(copy.renders === 0 && copy.name === "Selftest renamed copy", JSON.stringify(copy));
    await api.trashProject(copy.dir);
    const names = (await api.listProjects()).map((p) => p.name);
    expect(names.includes("Selftest renamed") && !names.includes(copy.name), names.join(", "));
    setProject(renamed, timelineJson);
    return names.join(", ");
  });
  if (params.get("timeline")) {
    await step("import a timeline as a project", async () => {
      const got = await api.importTimeline(need("timeline"));
      expect(got.missing.length === 0, `missing ${got.missing}`);
      const assets = await api.listAssets(got.project.dir, (await api.openProject(got.project.dir)).timelineJson);
      expect(assets.some((a) => a.usedBy.length > 0), "no imported file is used");
      return `${got.project.name}: copied ${got.copied.length} file(s)`;
    });
  }
  await step("commands refuse a folder outside the projects folder", async () => {
    const refused = await api.saveProject("/tmp", "{}").then(
      () => false,
      () => true,
    );
    expect(refused, "saving into /tmp was accepted");
    return "refused";
  });

  if (params.get("generate") === "1") {
    await step("generate a project from text (one paid call, Sonnet)", async () => {
      const args = {
        text: "Why the sea is salty. Rain dissolves minerals out of rock and rivers carry them to the sea. When seawater evaporates, the salt stays behind.",
        lang: "en" as const,
        orientation: "landscape" as const,
        targetSeconds: 12,
        model: "claude-sonnet-5-5" as const,
        illustrate: false,
        name: null,
      };
      const estimate = await api.estimate(args);
      expect(!estimate.problem, estimate.problem ?? "");
      const finished = await new Promise<GenerateFinished>((resolve, reject) => {
        void api.onGenerate(() => undefined, resolve).then(() => api.startGenerate(args).catch(reject));
      });
      expect(finished.ok && !!finished.project, `${finished.errorKind}: ${finished.error}`);
      const opened = await api.openProject(finished.project!.dir);
      expect(JSON.parse(opened.timelineJson).scenes.length === finished.scenes, "scene count differs");
      return `“${finished.project!.name}”, ${finished.scenes} scenes, estimate $${estimate.total.typical.toFixed(4)}, actual $${finished.costUsd.toFixed(4)}`;
    });
  }
};

const Selftest = () => {
  const [steps, setSteps] = useState<Step[]>([]);
  const [done, setDone] = useState(false);
  const [project, setProject] = useState<{ p: ProjectSummary; timelineJson: string } | null>(null);
  const [view, setView] = useState<"log" | "home" | "project">("log");
  const [rendersKey, setRendersKey] = useState(0);

  useEffect(() => {
    const all: Step[] = [];
    void run(
      (s) => {
        all.push(s);
        setSteps([...all]);
        void report(all, false);
      },
      (p, timelineJson) => setProject({ p, timelineJson }),
    )
      .catch((e) => all.push({ step: "self-test", ok: false, detail: String(e) }))
      .then(async () => {
        setSteps([...all]);
        setDone(true);
        await report(all, true);
        // Then the real screens on the real data, for a look (and a screenshot).
        window.setTimeout(() => setView("home"), 4000);
        window.setTimeout(() => setView("project"), 24000);
        // A bumped key is what a finished render does: the sidebar turns to the Renders tab.
        window.setTimeout(() => setRendersKey(1), 36000);
      });
  }, []);

  if (view === "home") return <Home onOpen={() => undefined} />;
  if (view === "project" && project) {
    return (
      <div style={{ height: "100%", display: "grid", gridTemplateColumns: "300px 1fr" }}>
        <aside className="sidebar panel" style={{ display: "flex", flexDirection: "column", minHeight: 0 }}>
          <ProjectSidebar project={project.p} timelineJson={project.timelineJson} rendersKey={rendersKey} onProjectChanged={(p) => setProject({ ...project, p })} />
        </aside>
        <main style={{ padding: 24 }} className="dim">
          The project sidebar on the self-test project.
        </main>
      </div>
    );
  }
  return (
    <div style={{ padding: 24, overflow: "auto", height: "100%" }}>
      <h2 style={{ marginTop: 0 }}>Studio self-test {done ? (steps.every((s) => s.ok) ? "— all passed" : "— FAILED") : "— running…"}</h2>
      <ol style={{ lineHeight: 1.7 }}>
        {steps.map((s, i) => (
          <li key={i} style={{ color: s.ok ? "var(--emerald)" : "var(--danger)" }}>
            {s.step} <span className="dim mono">{s.detail}</span>
          </li>
        ))}
      </ol>
    </div>
  );
};

createRoot(document.getElementById("root")!).render(<Selftest />);
