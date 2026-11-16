// The whole app as the desktop shell shows it, in a plain browser: /src/studio/app-dev.html.
// Development only. Tauri's IPC is mocked: Studio commands are answered by the in-memory fake
// (./fake.ts), the editor's own commands by small stand-ins, dialogs by canned answers. Nothing
// touches the disk and nothing calls an API. Use it to click through Home, a project, the
// sidebar, saving and a (pretend) render without building the desktop app.
import { emit } from "@tauri-apps/api/event";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import sample from "../../../../fixtures/sample-timeline.json";
import { createFakeApi } from "./fake";
import type { GenerateArgs } from "./types";

type Args = Record<string, any>;

const fake = createFakeApi();
const log: string[] = [];
(window as unknown as { __shellLog: string[] }).__shellLog = log;

const dialogOpen = (options: Args) => {
  if (options?.directory) return "/Users/you/Movies/Vimmer";
  const filter: string = options?.filters?.[0]?.name ?? "";
  if (filter === "Timeline JSON") return "/Users/you/old/product-demo.timeline.json";
  if (filter === "Text") return "/Users/you/notes/sea.txt";
  if (filter === "Video") return "/assets/clips/life-tone-4s.mp4";
  if (options?.multiple) return ["/assets/brand/logo.png", "/assets/clips/gradients-2s.mp4"];
  return "/assets/design.png";
};

const handlers: Record<string, (a: Args) => unknown> = {
  // Studio
  studio_settings: () => fake.settings(),
  set_projects_root: (a) => fake.setProjectsRoot(a.root),
  list_projects: () => fake.listProjects(),
  create_project: (a) => fake.createProject(a.name, a.lang, a.orientation),
  open_project: (a) => fake.openProject(a.dir),
  save_project: (a) => fake.saveProject(a.dir, a.timelineJson),
  rename_project: (a) => fake.renameProject(a.dir, a.name),
  duplicate_project: (a) => fake.duplicateProject(a.dir),
  trash_project: (a) => fake.trashProject(a.dir),
  import_timeline: (a) => fake.importTimeline(a.path),
  export_timeline: (a) => fake.exportTimeline(a.dir, a.path, a.contents),
  reveal_path: () => null,
  list_assets: (a) => fake.listAssets(a.dir, a.timelineJson),
  import_assets: (a) => fake.importAssets(a.dir, a.sources),
  trash_asset: (a) => fake.trashAsset(a.dir, a.path, a.timelineJson),
  trash_unused_assets: (a) => fake.trashUnusedAssets(a.dir, a.timelineJson),
  list_renders: (a) => fake.listRenders(a.dir),
  trash_render: (a) => fake.trashRender(a.dir, a.id),
  add_existing_render: (a) => fake.addExistingRender(a.dir, a.source),
  generate_estimate: (a) => fake.estimate(a.args as GenerateArgs),
  start_generate: (a) => {
    if (!a.args.confirmed) throw new Error("Generating costs money: confirm the estimate first.");
    return fake.startGenerate(a.args as GenerateArgs);
  },
  cancel_generate: () => fake.cancelGenerate(),

  // The editor's own commands
  asset_root: () => "",
  import_asset: () => "assets/sample.png",
  render_defaults: () => ({ tts: "say", ttsFromEnv: false, chunkMs: 30000, preprocessJobs: 4, renderJobs: 2 }),
  check_before_render: () => ({ schema: "vimmer-check/1", summary: { errors: 0, warnings: 0, infos: 0 }, findings: [] }),
  cancel_render: () => true,
  reveal_in_finder: () => null,
  read_project_file: (a) => (String(a.path).endsWith(".txt") ? "Why the sea is salty\n\nRain dissolves minerals out of rock." : JSON.stringify(sample)),
  write_project_file: () => null,
  expand_timeline: (a) => ({ ok: true, timeline: JSON.parse(a.timelineJson) }),
  detect_beats: () => ({ ok: false, error: "Beat detection needs the desktop app." }),
  music_library: () => ({ ok: false, error: "The music library needs the desktop app." }),
  music_pick: () => ({ ok: false, error: "The music library needs the desktop app." }),
  propose_edit: () => ({ ok: false, error: { kind: "helper", message: "The assistant needs the desktop app." } }),
  // A pretend render: progress, then a finished event like a real project render's.
  start_render: (a) => {
    const { projectDir, output } = a.args as { projectDir?: string; output?: string };
    const step = (ms: number, run: () => void) => window.setTimeout(run, ms);
    step(200, () => void emit("render-progress", { kind: "stage", name: "Preparing images" }));
    step(600, () => void emit("render-progress", { kind: "chunks", done: 0, total: 1 }));
    step(1400, () => void emit("render-progress", { kind: "chunks", done: 1, total: 1 }));
    step(1800, async () => {
      if (!projectDir) return void emit("render-finished", { ok: true, cancelled: false, output: output ?? "/fake/out.mp4", error: null });
      const changed = await fake.addExistingRender(projectDir, "/assets/clips/counter-6s.mp4");
      const newest = changed.renders[0];
      void emit("render-finished", {
        ok: true, cancelled: false, output: newest.path, error: null,
        render: { ...newest, imported: false }, project: changed.project, notes: [],
      });
    });
    return null;
  },

  // Plugins
  "plugin:dialog|open": (a) => dialogOpen(a.options),
  "plugin:dialog|save": (a) => a.options?.defaultPath ? `/Users/you/Desktop/${String(a.options.defaultPath).split("/").pop()}` : "/Users/you/Desktop/out.mp4",
  "plugin:dialog|ask": (a) => window.confirm(a.message),
  "plugin:dialog|confirm": (a) => window.confirm(a.message),
  "plugin:dialog|message": () => true,
};

mockWindows("main");
mockIPC(
  (cmd, payload) => {
    const handler = handlers[cmd];
    log.push(cmd);
    if (handler) return handler((payload ?? {}) as Args);
    // Window titles, close guards and the like: nothing to do in a browser tab.
    if (cmd.startsWith("plugin:")) return null;
    throw new Error(`The mocked shell has no command ${cmd}`);
  },
  { shouldMockEvents: true },
);
// Files are served by the dev server from the renderer's public folder.
const internals = (window as unknown as { __TAURI_INTERNALS__: { convertFileSrc: (path: string) => string } }).__TAURI_INTERNALS__;
internals.convertFileSrc = (path) => path.replace(/^.*?\/(assets|generated)\//, "/$1/");

// The fake reports a generation through its own listeners; the app listens to Tauri events.
void fake.onGenerate(
  (p) => void emit("generate-progress", p),
  (f) => void emit("generate-finished", f),
);

// Only now load the app: it asks "am I in the desktop shell?" as it starts.
void Promise.all([import("react-dom/client"), import("../App"), import("../styles.css")]).then(([dom, app]) => {
  dom.createRoot(document.getElementById("root")!).render(<app.App />);
});
