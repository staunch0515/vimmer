// The Studio API served from memory, for the browser (no desktop shell): the dev harness in
// ./dev.tsx and screenshots. Nothing is read from or written to disk, and "generate" calls no
// API: it plays canned progress and makes a demo project. File "paths" are URLs the dev server
// serves from remotion-renderer/public.
import type { StudioApi } from "./api";
import { kindOfFile } from "./format";
import type {
  AssetInfo,
  Estimate,
  GenerateArgs,
  GenerateFinished,
  GenerateProgress,
  ProjectSummary,
  RenderRecord,
} from "./types";

type FakeProject = {
  summary: ProjectSummary;
  timelineJson: string;
  assets: AssetInfo[];
  renders: RenderRecord[];
};

const ROOT = "/Users/you/Documents/Vimmer";
const HOUR = 3_600_000;

const timelineWith = (name: string, images: string[], portrait = false): string =>
  JSON.stringify(
    {
      fps: 30,
      resolution: portrait ? { width: 1080, height: 1920 } : { width: 1920, height: 1080 },
      scenes: (images.length ? images : [null]).map((image, i) => ({
        id: `scene-${i + 1}`,
        duration_ms: 4000,
        text: i === 0 ? name : `Scene ${i + 1}`,
        highlights: [],
        image,
        motion: i % 2 ? "zoom_in" : "static",
        audio: null,
      })),
      subtitles: [],
    },
    null,
    2,
  ) + "\n";

const asset = (path: string, bytes: number, extra: Partial<AssetInfo> = {}): AssetInfo => ({
  path,
  name: path.split("/").pop()!,
  kind: kindOfFile(path) ?? "image",
  bytes,
  modifiedMs: Date.now() - 2 * HOUR,
  width: null,
  height: null,
  durationMs: null,
  usedBy: [],
  // The dev server serves the renderer's public folder at "/".
  file: `/${path}`,
  ...extra,
});

const usedBy = (timelineJson: string, path: string): string[] => {
  try {
    const t = JSON.parse(timelineJson) as { scenes: { id: string; image?: string | null; video?: { src: string } | null; audio?: { source: string } | null }[] };
    return t.scenes.filter((s) => s.image === path || s.video?.src === path || s.audio?.source === path).map((s) => s.id);
  } catch {
    return [];
  }
};

const durationOf = (timelineJson: string): number => {
  try {
    return (JSON.parse(timelineJson) as { scenes: { duration_ms: number }[] }).scenes.reduce((n, s) => n + s.duration_ms, 0);
  } catch {
    return 0;
  }
};

const seed = (now: number): FakeProject[] => {
  const make = (
    name: string,
    ageHours: number,
    opts: { images?: string[]; thumb?: string | null; origin?: ProjectSummary["origin"]; lang?: ProjectSummary["lang"]; portrait?: boolean; renders?: number },
  ): FakeProject => {
    const timelineJson = timelineWith(name, opts.images ?? [], opts.portrait);
    const renders: RenderRecord[] = Array.from({ length: opts.renders ?? 0 }, (_, i) => ({
      id: `r${i + 1}`,
      file: `renders/2026111${i + 2}-10300${i}.mp4`,
      finishedMs: now - (ageHours + (opts.renders! - i) * 5) * HOUR,
      bytes: 3_400_000 + i * 900_000,
      durationMs: durationOf(timelineJson),
      options: { tts: i ? "kokoro" : "say", autoSubtitles: "on", wordSubtitles: true },
      imported: false,
      path: "/assets/clips/counter-6s.mp4",
      missing: false,
    }));
    return {
      summary: {
        dir: `${ROOT}/${name}`,
        name,
        createdMs: now - (ageHours + 40) * HOUR,
        updatedMs: now - ageHours * HOUR,
        lang: opts.lang ?? null,
        orientation: opts.portrait ? "portrait" : "landscape",
        durationMs: durationOf(timelineJson),
        origin: opts.origin ?? "blank",
        renders: renders.length,
        thumb: opts.thumb ?? null,
        thumbVersion: 1,
      },
      timelineJson,
      assets: [],
      renders,
    };
  };
  const coffee = make("How coffee is made", 0.2, {
    images: ["assets/sample.jpeg", "assets/design.png", "assets/sample.png"],
    thumb: "/assets/sample.jpeg",
    origin: "generated",
    lang: "en",
    renders: 2,
  });
  coffee.assets = [
    asset("assets/sample.jpeg", 184_000, { width: 1600, height: 900 }),
    asset("assets/design.png", 412_000, { width: 1920, height: 1080 }),
    asset("assets/sample.png", 96_000, { width: 1280, height: 720 }),
    asset("assets/clips/counter-6s.mp4", 1_250_000, { width: 1280, height: 720, durationMs: 6000 }),
    asset("assets/pad-loop.m4a", 310_000, { durationMs: 16_000 }),
    asset("assets/icons/star.svg", 640),
    asset("assets/quizbuks-taker-cover-center.png", 228_000, { width: 1200, height: 630 }),
  ];
  return [
    coffee,
    make("西安钟楼", 5, { images: ["assets/design.png"], thumb: "/assets/design.png", origin: "generated", lang: "zh", renders: 1 }),
    make("桜の季節", 30, { origin: "generated", lang: "ja", portrait: true }),
    make("Product launch teaser", 75, { images: ["assets/sample.png"], thumb: "/assets/sample.png", origin: "imported" }),
    make("Untitled", 200, {}),
  ];
};

const SAMPLE_TEXT = `Why the sea is salty

Rain slowly dissolves minerals out of rock. Rivers carry them to the sea.
When seawater evaporates, the salt stays behind, so over millions of years the sea became salty.`;

export type FakeOptions = {
  /** Start with no projects (the empty Home screen). */
  empty?: boolean;
  /** Make "generate" fail with this message instead of finishing. */
  generateError?: string;
  /** Milliseconds between the fake generation's steps. */
  stepMs?: number;
};

export const createFakeApi = (options: FakeOptions = {}): StudioApi => {
  let projects = options.empty ? [] : seed(Date.now());
  let root = ROOT;
  let recent: string[] = projects.slice(0, 3).map((p) => p.summary.dir);
  const generateListeners = new Set<{ progress: (p: GenerateProgress) => void; finished: (f: GenerateFinished) => void }>();
  let running: { cancelled: boolean } | null = null;
  const stepMs = options.stepMs ?? 900;

  const find = (dir: string): FakeProject => {
    const p = projects.find((x) => x.summary.dir === dir);
    if (!p) throw new Error(`${dir} is not a project in ${root}`);
    return p;
  };
  const freeName = (name: string): string => {
    const taken = new Set(projects.map((p) => p.summary.name));
    if (!taken.has(name)) return name;
    for (let n = 2; ; n += 1) if (!taken.has(`${name} ${n}`)) return `${name} ${n}`;
  };
  const add = (name: string, timelineJson: string, extra: Partial<ProjectSummary> = {}): FakeProject => {
    const finalName = freeName(name.trim() || "Untitled");
    const now = Date.now();
    const p: FakeProject = {
      summary: {
        dir: `${root}/${finalName}`,
        name: finalName,
        createdMs: now,
        updatedMs: now,
        lang: null,
        orientation: "landscape",
        durationMs: durationOf(timelineJson),
        origin: "blank",
        renders: 0,
        thumb: null,
        thumbVersion: 0,
        ...extra,
      },
      timelineJson,
      assets: [],
      renders: [],
    };
    projects = [p, ...projects];
    return p;
  };
  const withUsage = (p: FakeProject, timelineJson: string): AssetInfo[] =>
    p.assets.map((a) => ({ ...a, usedBy: usedBy(timelineJson, a.path) }));
  const sorted = (): ProjectSummary[] => [...projects].sort((a, b) => b.summary.updatedMs - a.summary.updatedMs).map((p) => p.summary);
  const wait = (ms: number) => new Promise<void>((resolve) => window.setTimeout(resolve, ms));

  const estimate = (args: GenerateArgs): Estimate => {
    if (!args.text.trim()) throw new Error("The text is empty.");
    const [pIn, pOut] = args.model === "claude-opus-5-5" ? [4, 20] : [2, 10];
    const wide = Array.from(args.text).filter((c) => c.charCodeAt(0) > 127 && c.trim()).length;
    const words = args.text.split(/\s+/).filter((w) => w && /^[\x00-\x7f]+$/.test(w)).length;
    const scenes = Math.min(60, Math.max(3, Math.ceil(args.targetSeconds ? args.targetSeconds / 6 : wide / 45 + words / 30)));
    const input = 2600 + wide + (args.text.length - wide) / 4;
    const output = 250 + 170 * scenes;
    const call = (n: number) => ((input + n * (output + 300)) * pIn + output * pOut) / 1e6;
    const storyboard = { typical: call(0), worst: call(0) + call(1) + call(2) };
    const perScene = (1700 * pIn + 3500 * pOut) / 1e6;
    const style = (1200 * pIn + 1000 * pOut) / 1e6;
    const retry = (5900 * pIn + 3500 * pOut) / 1e6;
    const illustrate = args.illustrate ? { typical: style + scenes * perScene, worst: style + scenes * (perScene + retry) } : null;
    return {
      model: args.model,
      inputTokens: Math.round(input),
      outputTokens: Math.round(output),
      scenes,
      storyboard,
      illustrate,
      total: { typical: storyboard.typical + (illustrate?.typical ?? 0), worst: storyboard.worst + (illustrate?.worst ?? 0) },
      problem: null,
    };
  };

  return {
    settings: async () => ({
      root,
      defaultRoot: ROOT,
      isDefault: root === ROOT,
      recent: recent.flatMap((dir) => projects.filter((p) => p.summary.dir === dir).map((p) => p.summary)),
    }),
    setProjectsRoot: async (next) => {
      root = next;
      projects = [];
      recent = [];
      return { root, defaultRoot: ROOT, isDefault: root === ROOT, recent: [] };
    },
    listProjects: async () => sorted(),
    createProject: async (name, lang, orientation) =>
      add(name, timelineWith(name.trim() || "Untitled", [], orientation === "portrait"), { lang, orientation }).summary,
    openProject: async (dir) => {
      const p = find(dir);
      recent = [dir, ...recent.filter((d) => d !== dir)];
      return { project: p.summary, timelineJson: p.timelineJson };
    },
    saveProject: async (dir, timelineJson) => {
      const p = find(dir);
      p.timelineJson = timelineJson;
      p.summary = { ...p.summary, updatedMs: Date.now(), durationMs: durationOf(timelineJson) || p.summary.durationMs };
      return p.summary;
    },
    renameProject: async (dir, name) => {
      const p = find(dir);
      const next = name.trim();
      if (!next) throw new Error("The name is empty.");
      if (projects.some((x) => x !== p && x.summary.name === next)) throw new Error(`A project named "${next}" already exists.`);
      const newDir = `${root}/${next}`;
      recent = recent.map((d) => (d === dir ? newDir : d));
      p.summary = { ...p.summary, name: next, dir: newDir, updatedMs: Date.now() };
      return p.summary;
    },
    duplicateProject: async (dir) => {
      const p = find(dir);
      const copy = add(`${p.summary.name} copy`, p.timelineJson, {
        lang: p.summary.lang,
        orientation: p.summary.orientation,
        origin: p.summary.origin,
        thumb: p.summary.thumb,
        thumbVersion: p.summary.thumbVersion,
      });
      copy.assets = p.assets.map((a) => ({ ...a }));
      return copy.summary;
    },
    trashProject: async (dir) => {
      find(dir);
      projects = projects.filter((p) => p.summary.dir !== dir);
      recent = recent.filter((d) => d !== dir);
    },
    importTimeline: async (path) => {
      const stem = (path.split("/").pop() ?? "Imported").replace(/(\.timeline)?\.json$/, "");
      const p = add(stem, timelineWith(stem, ["assets/sample.jpeg"]), { origin: "imported", thumb: "/assets/sample.jpeg", thumbVersion: 1 });
      p.assets = [asset("assets/sample.jpeg", 184_000, { width: 1600, height: 900 })];
      return { project: p.summary, copied: ["assets/sample.jpeg"], missing: [] };
    },
    reveal: async () => undefined,

    listAssets: async (dir, timelineJson) => withUsage(find(dir), timelineJson),
    importAssets: async (dir, sources) => {
      const p = find(dir);
      return sources.map((source) => {
        const name = source.split("/").pop()!;
        const existing = p.assets.find((a) => a.name === name);
        if (existing) return { source, path: existing.path, reused: true };
        const path = `assets/${name}`;
        p.assets = [...p.assets, asset(path, 150_000, { file: source.startsWith("/") ? source : `/${path}`, modifiedMs: Date.now() })];
        return { source, path, reused: false };
      });
    },
    trashAsset: async (dir, path, timelineJson) => {
      const p = find(dir);
      const users = usedBy(timelineJson, path);
      if (users.length) throw new Error(`${path} is used by ${users.join(", ")}.`);
      p.assets = p.assets.filter((a) => a.path !== path);
    },
    trashUnusedAssets: async (dir, timelineJson) => {
      const p = find(dir);
      const unused = p.assets.filter((a) => usedBy(timelineJson, a.path).length === 0).map((a) => a.path);
      p.assets = p.assets.filter((a) => !unused.includes(a.path));
      return unused;
    },

    listRenders: async (dir) => [...find(dir).renders].reverse(),
    trashRender: async (dir, id) => {
      const p = find(dir);
      p.renders = p.renders.filter((r) => r.id !== id);
      p.summary = { ...p.summary, renders: p.renders.length };
      return { project: p.summary, renders: [...p.renders].reverse() };
    },
    addExistingRender: async (dir, source) => {
      const p = find(dir);
      const name = source.split("/").pop()!;
      p.renders = [
        ...p.renders,
        { id: `r${Date.now()}`, file: `renders/${name}`, finishedMs: Date.now(), bytes: 2_100_000, durationMs: 6000, options: null, imported: true, path: source, missing: false },
      ];
      p.summary = { ...p.summary, renders: p.renders.length };
      return { project: p.summary, renders: [...p.renders].reverse() };
    },

    estimate: async (args) => estimate(args),
    startGenerate: async (args) => {
      if (running) throw new Error("A generation is already running.");
      const run = { cancelled: false };
      running = run;
      const emit = (p: GenerateProgress) => generateListeners.forEach((l) => l.progress(p));
      const finish = (f: GenerateFinished) => {
        running = null;
        generateListeners.forEach((l) => l.finished(f));
      };
      const failed = (kind: string, error: string, costUsd: number): GenerateFinished => ({
        ok: false, cancelled: kind === "cancelled", project: null, costUsd, inputTokens: 0, outputTokens: 0, scenes: 0, illustrated: 0, warnings: [], errorKind: kind, error,
      });
      void (async () => {
        const e = estimate(args);
        emit({ stage: "storyboard", message: `Writing the storyboard with ${args.model}` });
        await wait(stepMs);
        if (run.cancelled) return finish(failed("cancelled", "cancelled during attempt 1; tokens of that call are not counted", 0));
        if (options.generateError) return finish(failed("api", options.generateError, 0));
        if (args.illustrate) {
          for (let i = 1; i <= Math.min(e.scenes, 4); i += 1) {
            emit({ stage: "illustrations", message: `scene ${i} of ${e.scenes}: drawn` });
            await wait(stepMs / 2);
            if (run.cancelled) break;
          }
        }
        emit({ stage: "project", message: "Creating the project" });
        await wait(stepMs / 2);
        const title = args.name?.trim() || args.text.trim().split("\n")[0].slice(0, 40) || "Generated";
        const p = add(title, timelineWith(title, ["assets/sample.jpeg", "assets/design.png"], args.orientation === "portrait"), {
          lang: args.lang, orientation: args.orientation, origin: "generated", thumb: "/assets/sample.jpeg", thumbVersion: 1,
        });
        p.assets = [asset("assets/sample.jpeg", 184_000, { width: 1600, height: 900 }), asset("assets/design.png", 412_000, { width: 1920, height: 1080 })];
        finish({
          ok: true, cancelled: false, project: p.summary, costUsd: e.total.typical * 0.93, inputTokens: e.inputTokens, outputTokens: e.outputTokens,
          scenes: 2, illustrated: args.illustrate ? 2 : 0, warnings: [], errorKind: null, error: null,
        });
      })();
    },
    cancelGenerate: async () => {
      if (!running) return false;
      running.cancelled = true;
      return true;
    },
    onGenerate: async (progress, finished) => {
      const l = { progress, finished };
      generateListeners.add(l);
      return () => void generateListeners.delete(l);
    },

    fileUrl: (path) => path,
    pickTimelineFile: async () => "/Users/you/old/product-demo.timeline.json",
    pickMediaFiles: async () => ["/assets/brand/logo.png", "/assets/clips/gradients-2s.mp4"],
    pickVideoFile: async () => "/assets/clips/life-tone-4s.mp4",
    pickTextFile: async () => ({ name: "sea.txt", text: SAMPLE_TEXT }),
    pickFolder: async () => "/Users/you/Movies/Vimmer",
    confirm: async (message) => window.confirm(message),
    onFilesDropped: async () => () => undefined,
  };
};
