// Dev-server backend for the editing assistant (browser editor only; the desktop app has its
// own Tauri command). POST /__vimmer/edit runs `vimmer edit - <instruction> --dry-run --json`
// with the timeline on stdin and returns its JSON. Local requests only.
//
// The desktop app does not use this file: it runs the edit in-process through the Rust library.
// This endpoint needs a built CLI that has `vimmer edit`; a missing or too old binary is reported
// as "build the CLI" (see `checkEditCli`), never blamed on the Claude helper.
//
// The API key is never read here. When ANTHROPIC_API_KEY is not in this process's environment,
// the CLI is started through `zsh -c 'source ~/.zshrc; exec "$@"'`, so only the child sees it.

import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, rmSync, statSync, writeFileSync } from "node:fs";
import type { IncomingMessage, ServerResponse } from "node:http";
import { tmpdir } from "node:os";
import { join, resolve, sep } from "node:path";
import type { Plugin } from "vite";

const ENDPOINT = "/__vimmer/edit";
const MODELS = new Set(["claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5"]);
const MAX_BODY = 2 * 1024 * 1024;
const MAX_INSTRUCTION = 2000;
const TIMEOUT_MS = 6 * 60 * 1000;
const LOCAL_HOSTS = new Set(["localhost", "127.0.0.1", "[::1]"]);

/** The chat so far (see editor/src/chat.ts); the CLI checks its shape and limits again. */
type Conversation = { turns: unknown[]; dropped: number };
type Args = { timelineJson: string; instruction: string; sceneId: string | null; model: string; conversation?: Conversation };
const MAX_TURNS = 12;

const send = (res: ServerResponse, status: number, body: unknown) => {
  res.statusCode = status;
  res.setHeader("content-type", "application/json; charset=utf-8");
  res.setHeader("cache-control", "no-store");
  res.end(JSON.stringify(body));
};

const fail = (res: ServerResponse, status: number, kind: string, message: string) =>
  send(res, status, { ok: false, error: { kind, message } });

/** Loopback peer, a localhost Host header (no DNS rebinding) and, if sent, a localhost Origin. */
export const isLocalRequest = (req: IncomingMessage): boolean => {
  const peer = req.socket.remoteAddress ?? "";
  const loopback = peer === "127.0.0.1" || peer === "::1" || peer === "::ffff:127.0.0.1";
  const hostName = (h: string) => h.replace(/:\d+$/, "");
  const host = hostName(req.headers.host ?? "");
  let originOk = true;
  if (req.headers.origin) {
    try {
      originOk = LOCAL_HOSTS.has(new URL(req.headers.origin).hostname.replace(/^\[?(.*?)\]?$/, (_m, h) => (h === "::1" ? "[::1]" : h)));
    } catch {
      originOk = false;
    }
  }
  return loopback && LOCAL_HOSTS.has(host) && originOk;
};

const readBody = (req: IncomingMessage): Promise<string> =>
  new Promise((ok, bad) => {
    let size = 0;
    const chunks: Buffer[] = [];
    req.on("data", (c: Buffer) => {
      size += c.length;
      if (size > MAX_BODY) {
        bad(new Error("request too large"));
        req.destroy();
      } else chunks.push(c);
    });
    req.on("end", () => ok(Buffer.concat(chunks).toString("utf8")));
    req.on("error", bad);
  });

export const parseArgs = (raw: string): Args | string => {
  let v: Partial<Args>;
  try {
    v = JSON.parse(raw) as Partial<Args>;
  } catch {
    return "the request is not JSON";
  }
  if (typeof v.timelineJson !== "string") return "timelineJson must be a string";
  if (typeof v.instruction !== "string" || !v.instruction.trim()) return "the instruction is empty";
  if (v.instruction.length > MAX_INSTRUCTION) return `the instruction is longer than ${MAX_INSTRUCTION} characters`;
  if (v.sceneId != null && (typeof v.sceneId !== "string" || v.sceneId.length > 64)) return "bad sceneId";
  if (typeof v.model !== "string" || !MODELS.has(v.model)) return "unknown model";
  const args: Args = { timelineJson: v.timelineJson, instruction: v.instruction, sceneId: v.sceneId ?? null, model: v.model };
  if (v.conversation != null) {
    const c = v.conversation as Partial<Conversation>;
    if (typeof c !== "object" || !Array.isArray(c.turns)) return "conversation.turns must be a list";
    if (c.turns.length > MAX_TURNS) return `the conversation has more than ${MAX_TURNS} turns`;
    if (c.dropped != null && (typeof c.dropped !== "number" || !Number.isInteger(c.dropped) || c.dropped < 0)) return "bad conversation.dropped";
    // Only a chat with earlier turns changes the request; an empty one is the same as none.
    if (c.turns.length) args.conversation = { turns: c.turns, dropped: c.dropped ?? 0 };
  }
  return args;
};

/** `$VIMMER_BIN`, else the release or debug build in rust-core/target. */
export const findVimmer = (repo: string): string | null => {
  const candidates = [
    process.env.VIMMER_BIN,
    resolve(repo, "rust-core/target/release/vimmer"),
    resolve(repo, "rust-core/target/debug/vimmer"),
  ].filter((p): p is string => !!p);
  return candidates.find((p) => existsSync(p)) ?? null;
};

/**
 * The argv for the CLI: options first, then `--` so an instruction starting with `-` stays text.
 * `conversationFile` (the chat, written by the caller) is passed only when there is one, so a
 * first message runs exactly the command it ran before chats existed.
 */
export const cliArgs = (a: Args, publicDir: string, conversationFile?: string): string[] => {
  const out = ["edit", "--dry-run", "--json", "--public-dir", publicDir, "--model", a.model];
  if (a.sceneId) out.push("--scene", a.sceneId);
  if (conversationFile) out.push("--conversation", conversationFile);
  out.push("--", "-", a.instruction);
  return out;
};

let busy = false;

const BUILD_CLI = "Build the CLI: cd rust-core && cargo build -p vimmer-cli (or set VIMMER_BIN).";

/** True when `help` (the output of `vimmer edit --help`) is from a CLI that has the edit options this endpoint passes. */
export const helpHasEdit = (help: string): boolean => ["--dry-run", "--json", "--public-dir", "--scene"].every((o) => help.includes(o));

/** True when that help also has `--conversation` (the assistant's chat). */
export const helpHasChat = (help: string): boolean => helpHasEdit(help) && help.includes("--conversation");

const probed = new Map<string, string>();

/**
 * Null when `bin` has `vimmer edit --dry-run --json` (and `--conversation` when `chat`), else
 * what to do. Probed once per build (path and mtime).
 */
export const checkEditCli = async (bin: string, chat = false): Promise<string | null> => {
  let stamp: string;
  try {
    stamp = `${bin}@${statSync(bin).mtimeMs}`;
  } catch {
    return `The vimmer CLI was not found. ${BUILD_CLI}`;
  }
  let help = probed.get(stamp);
  if (help === undefined) {
    help = await new Promise<string>((done) => {
      const child = spawn(bin, ["edit", "--help"], { stdio: ["ignore", "pipe", "pipe"] });
      let text = "";
      const timer = setTimeout(() => child.kill("SIGTERM"), 10_000);
      child.stdout.on("data", (d: Buffer) => (text += d.toString("utf8")));
      child.on("error", () => {
        clearTimeout(timer);
        done("");
      });
      child.on("close", (code) => {
        clearTimeout(timer);
        done(code === 0 ? text : "");
      });
    });
    probed.set(stamp, help);
  }
  if (!helpHasEdit(help)) return `The vimmer CLI at ${bin} is too old: it has no \`vimmer edit --dry-run --json\`. ${BUILD_CLI}`;
  if (chat && !helpHasChat(help)) return `The vimmer CLI at ${bin} is too old for follow-up messages (no \`vimmer edit --conversation\`). ${BUILD_CLI}`;
  return null;
};

/** The CLI names install_llm.sh only when the helper's Python or its SDK is missing. */
export const refineKind = (body: unknown): unknown => {
  const b = body as { ok?: unknown; error?: { kind?: unknown; message?: unknown } } | null;
  if (b && b.ok === false && b.error?.kind === "helper" && String(b.error.message ?? "").includes("install_llm.sh")) {
    return { ...b, error: { ...b.error, kind: "helper_missing" } };
  }
  return body;
};

const run = (bin: string, args: string[], stdin: string): Promise<{ code: number | null; stdout: string; stderr: string }> =>
  new Promise((ok) => {
    const haveKey = !!process.env.ANTHROPIC_API_KEY?.trim();
    const useZsh = !haveKey && existsSync("/bin/zsh");
    const child = useZsh
      ? spawn("/bin/zsh", ["-c", 'source ~/.zshrc >/dev/null 2>&1; exec "$@"', "vimmer-edit", bin, ...args], { stdio: "pipe" })
      : spawn(bin, args, { stdio: "pipe" });
    let stdout = "";
    let stderr = "";
    const timer = setTimeout(() => child.kill("SIGTERM"), TIMEOUT_MS);
    child.stdout.on("data", (d: Buffer) => (stdout += d.toString("utf8")));
    child.stderr.on("data", (d: Buffer) => (stderr += d.toString("utf8")));
    child.on("error", (e) => {
      clearTimeout(timer);
      ok({ code: -1, stdout, stderr: String(e) });
    });
    child.on("close", (code) => {
      clearTimeout(timer);
      ok({ code, stdout, stderr });
    });
    child.stdin.end(stdin);
  });

const EXPAND_ENDPOINT = "/__vimmer/expand";

/** Runs `vimmer expand -` (template scenes into layers, no network, no key) on a timeline. */
const runExpand = (bin: string, timelineJson: string): Promise<{ code: number | null; stdout: string; stderr: string }> =>
  new Promise((ok) => {
    const child = spawn(bin, ["expand", "-"], { stdio: "pipe" });
    let stdout = "";
    let stderr = "";
    const timer = setTimeout(() => child.kill("SIGTERM"), 20_000);
    child.stdout.on("data", (d: Buffer) => (stdout += d.toString("utf8")));
    child.stderr.on("data", (d: Buffer) => (stderr += d.toString("utf8")));
    child.on("error", (e) => {
      clearTimeout(timer);
      ok({ code: -1, stdout, stderr: String(e) });
    });
    child.on("close", (code) => {
      clearTimeout(timer);
      ok({ code, stdout, stderr });
    });
    child.stdin.end(timelineJson);
  });

const BEATS_ENDPOINT = "/__vimmer/beats";

/** Runs `vimmer beats --json` on a file of the public dir, with the render's cache. */
const runBeats = (bin: string, file: string, cacheDir: string): Promise<{ code: number | null; stdout: string; stderr: string }> =>
  new Promise((ok) => {
    const child = spawn(bin, ["beats", "--json", "--cache-dir", cacheDir, file], { stdio: "pipe" });
    let stdout = "";
    let stderr = "";
    const timer = setTimeout(() => child.kill("SIGTERM"), 120_000);
    child.stdout.on("data", (d: Buffer) => (stdout += d.toString("utf8")));
    child.stderr.on("data", (d: Buffer) => (stderr += d.toString("utf8")));
    child.on("error", (e) => {
      clearTimeout(timer);
      ok({ code: -1, stdout, stderr: String(e) });
    });
    child.on("close", (code) => {
      clearTimeout(timer);
      ok({ code, stdout, stderr });
    });
    child.stdin.end();
  });

const MUSIC_ENDPOINT = "/__vimmer/music";

/** Runs `vimmer music ...` (no network, no key) with optional stdin. */
const runMusic = (bin: string, args: string[], stdin: string): Promise<{ code: number | null; stdout: string; stderr: string }> =>
  new Promise((ok) => {
    const child = spawn(bin, ["music", ...args], { stdio: "pipe" });
    let stdout = "";
    let stderr = "";
    const timer = setTimeout(() => child.kill("SIGTERM"), 20_000);
    child.stdout.on("data", (d: Buffer) => (stdout += d.toString("utf8")));
    child.stderr.on("data", (d: Buffer) => (stderr += d.toString("utf8")));
    child.on("error", (e) => {
      clearTimeout(timer);
      ok({ code: -1, stdout, stderr: String(e) });
    });
    child.on("close", (code) => {
      clearTimeout(timer);
      ok({ code, stdout, stderr });
    });
    child.stdin.end(stdin);
  });

/** `vimmer music` arguments for a request body, or an error. */
export const musicArgs = (body: unknown, publicDir: string): { args: string[]; stdin: string } | string => {
  if (!body || typeof body !== "object") return "body must be an object";
  const b = body as { action?: unknown; timelineJson?: unknown; mood?: unknown; energy?: unknown };
  const common = ["--json", "--public-dir", publicDir];
  if (b.action === "list") return { args: ["list", ...common], stdin: "" };
  if (b.action !== "pick") return "action must be list or pick";
  if (typeof b.timelineJson !== "string") return "timelineJson must be a string";
  const args = ["pick", "-", ...common];
  if (b.mood !== undefined && b.mood !== null) {
    if (typeof b.mood !== "string" || !/^[\p{L}\p{N} _-]{1,40}$/u.test(b.mood)) return "mood must be a short word";
    args.push(`--mood=${b.mood}`);
  }
  if (b.energy !== undefined && b.energy !== null) {
    if (typeof b.energy !== "number" || !(b.energy >= 0 && b.energy <= 1)) return "energy must be between 0 and 1";
    args.push(`--energy=${b.energy}`);
  }
  return { args, stdin: b.timelineJson };
};

/** A public-relative path that stays inside the public dir, or null. */
const publicFile = (publicDir: string, source: unknown): string | null => {
  if (typeof source !== "string" || !source || source.startsWith("/") || source.includes("\\")) return null;
  if (source.split("/").some((seg) => !seg || seg === "." || seg === "..")) return null;
  const file = resolve(publicDir, source);
  return file.startsWith(resolve(publicDir) + sep) && existsSync(file) ? file : null;
};

const TTS_ENDPOINT = "/__vimmer/tts";
/** Syntheses running at once (each a `vimmer tts` process); more wait, up to TTS_QUEUE. */
export const TTS_RUNNING = 2;
export const TTS_QUEUE = 8;
const TTS_TIMEOUT_MS = 5 * 60 * 1000;

type TtsArgs = { timelineJson: string; sceneIds: string[]; tts: "say" | "kokoro" | null };

/** Checks a "listen" request: the timeline, optional scene ids, optional engine. */
export const parseTtsArgs = (raw: string): TtsArgs | string => {
  let v: { timelineJson?: unknown; sceneIds?: unknown; tts?: unknown };
  try {
    v = JSON.parse(raw) as typeof v;
  } catch {
    return "the request is not JSON";
  }
  if (!v || typeof v !== "object") return "the request is not an object";
  if (typeof v.timelineJson !== "string") return "timelineJson must be a string";
  const ids = v.sceneIds ?? [];
  if (!Array.isArray(ids) || ids.length > 500 || !ids.every((id) => typeof id === "string" && id.length > 0 && id.length <= 64)) {
    return "sceneIds must be a list of scene ids";
  }
  if (v.tts != null && v.tts !== "say" && v.tts !== "kokoro") return "tts must be say or kokoro";
  return { timelineJson: v.timelineJson, sceneIds: ids as string[], tts: (v.tts as TtsArgs["tts"]) ?? null };
};

/** argv for `vimmer tts`: the timeline on stdin, `--scene=<id>` so an id starting with `-` stays an id. */
export const ttsCliArgs = (a: TtsArgs, publicDir: string): string[] => {
  const out = ["tts", "-", "--json", "--public-dir", publicDir];
  if (a.tts) out.push(`--tts=${a.tts}`);
  for (const id of a.sceneIds) out.push(`--scene=${id}`);
  return out;
};

/** True when `help` (of `vimmer tts --help`) is from a CLI that has what this endpoint passes. */
export const helpHasTts = (help: string): boolean => ["--json", "--public-dir", "--scene", "--tts"].every((o) => help.includes(o));

/**
 * At most `running` tasks at once, at most `queue` waiting; past that `run` rejects with "busy"
 * at once instead of piling up processes.
 */
export const limiter = (running: number, queue: number) => {
  let active = 0;
  const waiting: (() => void)[] = [];
  return {
    get active() {
      return active;
    },
    get waiting() {
      return waiting.length;
    },
    run: async <T>(task: () => Promise<T>): Promise<T> => {
      if (active >= running) {
        if (waiting.length >= queue) throw new Error("busy");
        await new Promise<void>((go) => waiting.push(go));
      } else active += 1;
      try {
        return await task();
      } finally {
        const next = waiting.shift();
        if (next) next();
        else active -= 1;
      }
    },
  };
};

const ttsSlots = limiter(TTS_RUNNING, TTS_QUEUE);
const ttsHelp = new Map<string, boolean>();

/** Null when `bin` has `vimmer tts`, else what to do. Probed once per build. */
const checkTtsCli = async (bin: string): Promise<string | null> => {
  let stamp: string;
  try {
    stamp = `${bin}@${statSync(bin).mtimeMs}`;
  } catch {
    return `The vimmer CLI was not found. ${BUILD_CLI}`;
  }
  let ok = ttsHelp.get(stamp);
  if (ok === undefined) {
    const out = await runCli(bin, ["tts", "--help"], "", 10_000);
    ok = out.code === 0 && helpHasTts(out.stdout);
    ttsHelp.set(stamp, ok);
  }
  return ok ? null : `The vimmer CLI at ${bin} is too old: it has no \`vimmer tts\` (listening before a render). ${BUILD_CLI}`;
};

const runCli = (bin: string, args: string[], stdin: string, timeoutMs: number): Promise<{ code: number | null; stdout: string; stderr: string }> =>
  new Promise((ok) => {
    const child = spawn(bin, args, { stdio: "pipe" });
    let stdout = "";
    let stderr = "";
    const timer = setTimeout(() => child.kill("SIGTERM"), timeoutMs);
    child.stdout.on("data", (d: Buffer) => (stdout += d.toString("utf8")));
    child.stderr.on("data", (d: Buffer) => (stderr += d.toString("utf8")));
    child.on("error", (e) => {
      clearTimeout(timer);
      ok({ code: -1, stdout, stderr: String(e) });
    });
    child.on("close", (code) => {
      clearTimeout(timer);
      ok({ code, stdout, stderr });
    });
    child.stdin.end(stdin);
  });

export const assistantPlugin = (repo: string, publicDir: string): Plugin => ({
  name: "vimmer-edit-assistant",
  configureServer(server) {
    // Template preview: the editor sends its timeline, the Rust core expands the template scenes
    // exactly as a render would and sends back {ok, timeline} or {ok: false, errors}.
    server.middlewares.use(EXPAND_ENDPOINT, (req: IncomingMessage, res: ServerResponse) => {
      void (async () => {
        if (req.method !== "POST") return send(res, 405, { ok: false, errors: ["POST only"] });
        if (!isLocalRequest(req)) return send(res, 403, { ok: false, errors: ["local requests only"] });
        let body: { timelineJson?: unknown };
        try {
          body = JSON.parse(await readBody(req)) as { timelineJson?: unknown };
        } catch (e) {
          return send(res, 400, { ok: false, errors: [`bad request: ${String(e)}`] });
        }
        if (typeof body.timelineJson !== "string") return send(res, 400, { ok: false, errors: ["timelineJson must be a string"] });
        const bin = findVimmer(repo);
        if (!bin) return send(res, 200, { ok: false, errors: ["Build the CLI to preview templates: cd rust-core && cargo build -p vimmer-cli"] });
        const out = await runExpand(bin, body.timelineJson);
        const line = out.stdout.trim().split("\n").filter(Boolean).pop() ?? "";
        try {
          return send(res, 200, JSON.parse(line));
        } catch {
          return send(res, 200, { ok: false, errors: [`vimmer expand failed (exit ${out.code}): ${out.stderr.trim().slice(-300)}`] });
        }
      })();
    });
    // "Detect beats": the music file's tempo and beat times, from the same code and cache as a render.
    server.middlewares.use(BEATS_ENDPOINT, (req: IncomingMessage, res: ServerResponse) => {
      void (async () => {
        if (req.method !== "POST") return send(res, 405, { ok: false, error: "POST only" });
        if (!isLocalRequest(req)) return send(res, 403, { ok: false, error: "local requests only" });
        let body: { source?: unknown };
        try {
          body = JSON.parse(await readBody(req)) as { source?: unknown };
        } catch (e) {
          return send(res, 400, { ok: false, error: `bad request: ${String(e)}` });
        }
        const file = publicFile(publicDir, body.source);
        if (!file) return send(res, 200, { ok: false, error: `No music file at public/${String(body.source)}.` });
        const bin = findVimmer(repo);
        if (!bin) return send(res, 200, { ok: false, error: "Build the CLI to detect beats: cd rust-core && cargo build -p vimmer-cli" });
        const out = await runBeats(bin, file, join(publicDir, "generated", "beats"));
        const line = out.stdout.trim().split("\n").filter(Boolean).pop() ?? "";
        try {
          return send(res, 200, JSON.parse(line));
        } catch {
          return send(res, 200, { ok: false, error: `vimmer beats failed (exit ${out.code}): ${out.stderr.trim().slice(-300)}` });
        }
      })();
    });
    // Music by mood: the library's moods and tracks, and which track a pick gets.
    server.middlewares.use(MUSIC_ENDPOINT, (req: IncomingMessage, res: ServerResponse) => {
      void (async () => {
        if (req.method !== "POST") return send(res, 405, { ok: false, error: "POST only" });
        if (!isLocalRequest(req)) return send(res, 403, { ok: false, error: "local requests only" });
        let body: unknown;
        try {
          body = JSON.parse(await readBody(req));
        } catch (e) {
          return send(res, 400, { ok: false, error: `bad request: ${String(e)}` });
        }
        const call = musicArgs(body, publicDir);
        if (typeof call === "string") return send(res, 400, { ok: false, error: call });
        const bin = findVimmer(repo);
        if (!bin) return send(res, 200, { ok: false, error: "Build the CLI to pick music by mood: cd rust-core && cargo build -p vimmer-cli" });
        const out = await runMusic(bin, call.args, call.stdin);
        const line = out.stdout.trim().split("\n").filter(Boolean).pop() ?? "";
        try {
          return send(res, 200, JSON.parse(line));
        } catch {
          return send(res, 200, { ok: false, error: `vimmer music failed (exit ${out.code}): ${out.stderr.trim().slice(-300)}` });
        }
      })();
    });
    // "Listen" (试听): the narration of some or all scenes, synthesized by `vimmer tts` into the
    // render's TTS cache (same files a render uses). Clips are served from the public dir.
    server.middlewares.use(TTS_ENDPOINT, (req: IncomingMessage, res: ServerResponse) => {
      void (async () => {
        if (req.method !== "POST") return send(res, 405, { ok: false, error: "POST only" });
        if (!isLocalRequest(req)) return send(res, 403, { ok: false, error: "local requests only" });
        let raw: string;
        try {
          raw = await readBody(req);
        } catch (e) {
          return send(res, 413, { ok: false, error: String(e) });
        }
        const args = parseTtsArgs(raw);
        if (typeof args === "string") return send(res, 400, { ok: false, error: args });
        const bin = findVimmer(repo);
        if (!bin) return send(res, 200, { ok: false, error: `Build the CLI to listen before rendering: ${BUILD_CLI}` });
        const stale = await checkTtsCli(bin);
        if (stale) return send(res, 200, { ok: false, error: stale });
        let out: { code: number | null; stdout: string; stderr: string };
        try {
          out = await ttsSlots.run(() => runCli(bin, ttsCliArgs(args, publicDir), args.timelineJson, TTS_TIMEOUT_MS));
        } catch {
          return send(res, 200, { ok: false, error: "Too many narrations are being made right now; try again in a moment." });
        }
        const line = out.stdout.trim().split("\n").filter(Boolean).pop() ?? "";
        try {
          return send(res, 200, JSON.parse(line));
        } catch {
          return send(res, 200, { ok: false, error: `vimmer tts failed (exit ${out.code}): ${out.stderr.trim().slice(-300)}` });
        }
      })();
    });
    server.middlewares.use(ENDPOINT, (req: IncomingMessage, res: ServerResponse) => {
      void (async () => {
        if (req.method !== "POST") return fail(res, 405, "method", "POST only");
        if (!isLocalRequest(req)) return fail(res, 403, "forbidden", "local requests only");
        if (!String(req.headers["content-type"] ?? "").startsWith("application/json")) {
          return fail(res, 415, "input", "expected application/json");
        }
        let raw: string;
        try {
          raw = await readBody(req);
        } catch (e) {
          return fail(res, 413, "input", String(e));
        }
        const args = parseArgs(raw);
        if (typeof args === "string") return fail(res, 400, "input", args);
        const bin = findVimmer(repo);
        if (!bin) return fail(res, 200, "no_binary", `The vimmer CLI was not found in rust-core/target. ${BUILD_CLI}`);
        const stale = await checkEditCli(bin, !!args.conversation);
        if (stale) return fail(res, 200, "no_binary", stale);
        if (busy) return fail(res, 200, "busy", "Another proposal is still running.");
        busy = true;
        // The chat goes to the CLI in a private temp file (stdin carries the timeline).
        const chatDir = args.conversation ? mkdtempSync(join(tmpdir(), "vimmer-chat-")) : null;
        try {
          let chatFile: string | undefined;
          if (chatDir && args.conversation) {
            chatFile = join(chatDir, "conversation.json");
            writeFileSync(chatFile, JSON.stringify(args.conversation), { mode: 0o600 });
          }
          const out = await run(bin, cliArgs(args, publicDir, chatFile), args.timelineJson);
          const line = out.stdout.trim().split("\n").filter(Boolean).pop() ?? "";
          try {
            return send(res, 200, refineKind(JSON.parse(line)));
          } catch {
            const tail = out.stderr.trim().split("\n").slice(-3).join(" | ");
            return fail(res, 200, "cli", `vimmer edit failed (exit ${out.code}): ${tail || "no output"}`);
          }
        } finally {
          busy = false;
          if (chatDir) rmSync(chatDir, { recursive: true, force: true });
        }
      })();
    });
  },
});
