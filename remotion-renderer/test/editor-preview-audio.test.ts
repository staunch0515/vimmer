import assert from "node:assert/strict";
import { test } from "node:test";
import { createElement, isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { Audio, Sequence } from "remotion";
import "./support/tsx.mjs";
import type { AudioItem } from "../src/audioPlan.ts";
// The composition and the plan import their modules without extensions (and .tsx): loaded through
// the test hooks registered above.
const { Timeline } = await import("../src/compositions/Timeline.tsx");
const { PreviewAudioContext, audioPlan, musicVolume } = await import("../src/audioPlan.ts");
const { AudioLengthsContext } = await import("../src/voiceover.ts");
import {
  BufferCache,
  PreviewMixer,
  bufferBytes,
  gainAt,
  planVoice,
  planKey,
  type ContextLike,
  type GainLike,
  type SourceLike,
} from "../editor/src/previewMixer.ts";
import type { Scene, VideoTimeline } from "../src/types/timeline.ts";

const FPS = 30;
const scene = (id: string, ms: number, p: Partial<Scene> = {}): Scene =>
  ({ id, duration_ms: ms, text: id, highlights: [], image: null, ...p }) as Scene;

// 3 scenes (2 s, 3 s, 1 s = 180 frames) with scene audio (one late, one early), sound effects, two
// voice-over clips (trims, gain) and faded music.
const timeline = {
  version: 1,
  fps: FPS,
  resolution: { width: 1920, height: 1080 },
  scenes: [
    scene("a", 2000, { audio: { source: "assets/a.wav", offset_ms: 500 } } as Partial<Scene>),
    scene("b", 3000, { audio: { source: "assets/b.wav", offset_ms: -1000 }, sfx: [{ src: "sfx/pop.wav", at_ms: 100 }, { src: "sfx/w.wav", at_ms: 9000, volume: 0.3 }] } as Partial<Scene>),
    scene("c", 1000),
  ],
  subtitles: [],
  voiceover: [
    { id: "v1", source: "assets/recordings/vo-1.wav", start_ms: 1000, trim_start_ms: 400, trim_end_ms: 600, gain: 1.5 },
    { id: "v2", source: "assets/recordings/vo-2.wav", start_ms: 4000 },
  ],
  music: { source: "music/bed.mp3", volume: 0.5, fade_in_ms: 1000, fade_out_ms: 2000 },
} as unknown as VideoTimeline;
const lengths = { "assets/recordings/vo-1.wav": 3000 };
const url = (p: string) => `asset://${p}`;

// ---------------------------------------------------------------- the composition's <Audio> elements

type Found = { src: string; from: number; lo: number; until: number | null; trimBefore: number; volume: unknown; loop: boolean };

/**
 * Walks the element tree Timeline returns: each <Audio> with where its Sequences put it: `from` is
 * its own Sequence start (file position = frame − from + trimBefore), `lo` the first frame every
 * enclosing Sequence shows, `until` the first frame one of them hides.
 */
const audiosOf = (node: ReactNode, from = 0, lo = 0, until: number | null = null, out: Found[] = []): Found[] => {
  if (Array.isArray(node)) {
    for (const n of node) audiosOf(n, from, lo, until, out);
    return out;
  }
  if (!isValidElement(node)) return out;
  const el = node as ReactElement<Record<string, unknown> & { children?: ReactNode }>;
  if (el.type === Audio) {
    const p = el.props as { src: string; trimBefore?: number; volume?: unknown; loop?: boolean };
    out.push({ src: p.src, from, lo, until, trimBefore: p.trimBefore ?? 0, volume: p.volume, loop: !!p.loop });
    return out;
  }
  if (el.type === Sequence) {
    const p = el.props as { from?: number; durationInFrames?: number };
    const start = from + (p.from ?? 0);
    const end = p.durationInFrames === undefined ? null : start + p.durationInFrames;
    const capped = until === null ? end : end === null ? until : Math.min(until, end);
    return audiosOf(el.props.children, start, Math.max(lo, start), capped, out);
  }
  return audiosOf(el.props.children, from, lo, until, out);
};

/** Timeline's returned tree, rendered inside a component so its hooks (contexts) run. */
const timelineTree = (preview: boolean): ReactNode => {
  let tree: ReactNode = null;
  const Capture = () => {
    tree = (Timeline as unknown as (p: VideoTimeline) => ReactNode)(timeline);
    return null;
  };
  renderToStaticMarkup(
    createElement(
      AudioLengthsContext.Provider,
      { value: lengths },
      createElement(PreviewAudioContext.Provider, { value: preview }, createElement(Capture)),
    ),
  );
  return tree;
};

test("renders keep every <Audio> (no preview flag); the preview draws none", () => {
  const render = audiosOf(timelineTree(false));
  assert.equal(render.length, 7); // 2 scene audio, 2 sfx, 2 voice-over, music
  assert.deepEqual(audiosOf(timelineTree(true)), []);
  // The default context is the render's.
  let flag: boolean | null = null;
  const Read = () => {
    flag = (PreviewAudioContext as unknown as { _currentValue: boolean })._currentValue;
    return null;
  };
  renderToStaticMarkup(createElement(Read));
  assert.equal(flag, false);
});

test("the plan is exactly where the composition's <Audio> elements sound", () => {
  const els = audiosOf(timelineTree(false));
  const plan = audioPlan(timeline, null, lengths);
  assert.equal(plan.length, els.length);
  for (const el of els) {
    const item = plan.find((i) => i.url === el.src);
    assert.ok(item, el.src);
    assert.equal(item.from, el.lo, el.src);
    assert.equal(item.from - item.fileStart, el.from - el.trimBefore, el.src); // same file position at every frame
    assert.equal(item.until, el.until, el.src);
    assert.equal(item.loop, el.loop);
    if (typeof el.volume === "function") assert.equal(typeof item.gain, "function");
    else assert.equal(item.gain, el.volume ?? 1, el.src);
  }
});

test("audioPlan: offsets, trims, gains and fades", () => {
  const plan = Object.fromEntries(audioPlan(timeline, url, lengths).map((i) => [i.key, i]));
  // Scene a: from 0, offset 0.5 s → starts at 15, cut at the scene end (60).
  assert.deepEqual({ ...plan["scene:a"], gain: plan["scene:a"].gain }, { key: "scene:a", url: "asset://assets/a.wav", from: 15, until: 60, fileStart: 0, gain: 1, loop: false });
  // Scene b: from 60, offset −1 s → starts at the scene's first frame, 30 frames into the file.
  assert.equal(plan["scene:b"].from, 60);
  assert.equal(plan["scene:b"].fileStart, 30);
  assert.equal(plan["scene:b"].until, 150);
  // Sound effects ring past the cut; a late one is held to the scene's last frame.
  assert.deepEqual([plan["sfx:b:0"].from, plan["sfx:b:0"].until, plan["sfx:b:0"].gain], [63, null, 0.8]);
  assert.deepEqual([plan["sfx:b:1"].from, plan["sfx:b:1"].gain], [149, 0.3]);
  // Voice-over 1: 3 s file, 0.4 s off the start, 0.6 s off the end → 2 s from frame 30.
  assert.deepEqual([plan["vo:v1"].from, plan["vo:v1"].until, plan["vo:v1"].fileStart, plan["vo:v1"].gain], [30, 90, 12, 1.5]);
  // Voice-over 2: length unknown → plays to the end of its file.
  assert.equal(plan["vo:v2"].until, null);
  // Music: the same fade function as the composition's.
  const m = plan.music;
  assert.equal(m.loop, true);
  const ref = musicVolume(timeline.music!, timeline);
  for (const f of [0, 15, 30, 100, 150, 165, 180]) assert.equal((m.gain as (f: number) => number)(f), ref(f));
});

// ---------------------------------------------------------------- scheduling math

const item = (p: Partial<AudioItem>): AudioItem => ({ key: "k", url: "u", from: 0, until: null, fileStart: 0, gain: 1, loop: false, ...p });
const clock = (frame: number, rate = 1) => ({ frame, fps: FPS, rate, endFrame: 300 });

test("planVoice: start ahead, seek mid-clip, trims, cuts, file end, rate", () => {
  // Starts 1 s after the playhead, at its trim.
  assert.deepEqual(planVoice(item({ from: 60, until: 120, fileStart: 12 }), clock(30), 5), { when: 1, offset: 0.4, stopAt: 3, loopEnd: null, startFrame: 60 });
  // Seek into the middle: plays now, 0.4 + 1 s into the file, stops at its end.
  assert.deepEqual(planVoice(item({ from: 60, until: 120, fileStart: 12 }), clock(90), 5), { when: 0, offset: 1.4, stopAt: 1, loopEnd: null, startFrame: 90 });
  // Fractional anchor frames are kept.
  assert.ok(Math.abs(planVoice(item({ from: 0 }), clock(15.5), 5)!.offset - 15.5 / 30) < 1e-12);
  // Past its cut, or past the end of its file: nothing.
  assert.equal(planVoice(item({ from: 60, until: 120 }), clock(120), 5), null);
  assert.equal(planVoice(item({ from: 0 }), clock(160), 5), null);
  // No cut: stops with the composition.
  assert.equal(planVoice(item({ from: 0 }), clock(0), 50)!.stopAt, 10);
  // Double speed: half the wall time to reach it and to the cut; the file offset is the same.
  assert.deepEqual(planVoice(item({ from: 60, until: 120 }), clock(30, 2), 5), { when: 0.5, offset: 0, stopAt: 1.5, loopEnd: null, startFrame: 60 });
  // Reverse or stopped: silent.
  assert.equal(planVoice(item({}), clock(0, -1), 5), null);
  assert.equal(planVoice(item({}), clock(0, 0), 5), null);
});

test("planVoice: looping music wraps at floor(length × fps) frames", () => {
  // A 2.51 s file loops every 75 frames (2.5 s).
  const m = item({ loop: true });
  assert.deepEqual(planVoice(m, clock(80), 2.51), { when: 0, offset: 5 / 30, stopAt: (300 - 80) / 30, loopEnd: 2.5, startFrame: 80 });
  assert.equal(planVoice(m, clock(150), 2.51)!.offset, 0);
});

test("gainAt: numbers, Remotion's per-loop volume frame, never negative", () => {
  assert.equal(gainAt(item({ gain: 1.5 }), 10, 1, FPS), 1.5);
  assert.equal(gainAt(item({ gain: -1 }), 10, 1, FPS), 0);
  const fadeIn = (f: number) => Math.min(1, f / 30);
  // Looped: the volume frame restarts every loop (Remotion's "repeat"), as the preview did.
  assert.equal(gainAt(item({ gain: fadeIn, loop: true }), 15, 2, FPS), 0.5);
  assert.equal(gainAt(item({ gain: fadeIn, loop: true }), 75, 2, FPS), 0.5);
  // Not looped: the frame inside its Sequence, plus the trim.
  assert.equal(gainAt(item({ gain: fadeIn, from: 30, fileStart: 6 }), 39, 2, FPS), 0.5);
});

// ---------------------------------------------------------------- the decoded-buffer cache

const buf = (seconds: number, channels = 1) => ({ duration: seconds, length: seconds * 48000, numberOfChannels: channels });

test("BufferCache: one load per URL, shared by concurrent asks", async () => {
  let loads = 0;
  const c = new BufferCache(async (u: string) => {
    loads++;
    return buf(u.length);
  });
  const [a, b] = await Promise.all([c.ensure("abc"), c.ensure("abc")]);
  assert.equal(a, b);
  await c.ensure("abc");
  assert.equal(loads, 1);
  assert.equal(c.bytes(), bufferBytes(buf(3)));
});

test("BufferCache: evicts least recently used past the budget, never what is kept", async () => {
  const budget = bufferBytes(buf(10)) * 2;
  const c = new BufferCache(async () => buf(10), budget);
  await c.ensure("a");
  await c.ensure("b");
  c.get("a"); // a is now more recent than b
  await c.ensure("c");
  assert.deepEqual(["a", "b", "c"].map((u) => c.has(u)), [true, false, true]);
  // Kept URLs stay even over budget.
  await c.ensure("d", new Set(["a", "c"]));
  assert.ok(c.has("a") && c.has("c") && c.has("d"));
  c.evict(new Set(["d"]));
  assert.ok(c.bytes() <= budget);
  assert.ok(c.has("d"));
});

test("BufferCache: a failed load is not retried at once", async () => {
  let now = 0;
  let loads = 0;
  const c = new BufferCache(
    async () => {
      loads++;
      throw new Error("404");
    },
    1e9,
    () => now,
  );
  assert.equal(await c.ensure("x"), null);
  assert.equal(await c.ensure("x"), null);
  assert.equal(loads, 1);
  now = 6000;
  await c.ensure("x");
  assert.equal(loads, 2);
});

// ---------------------------------------------------------------- the mixer, on a fake Web Audio

type Started = { url: string; when: number; offset: number; stop: number | null; loopEnd: number | null; rate: number; gain: GainLike; stopped: boolean };

const fakeAudio = () => {
  const started: Started[] = [];
  let resumes = 0;
  const param = () => ({ value: 1, events: [] as [number, number][], setValueAtTime(v: number, t: number) { this.events.push([v, t]); }, cancelScheduledValues() {} });
  const ctx: ContextLike & { currentTime: number } = {
    currentTime: 100,
    state: "suspended",
    destination: {},
    createGain: () => ({ gain: param(), connect: () => undefined, disconnect: () => undefined }),
    createBufferSource: () => {
      const rec: Partial<Started> = {};
      const src: SourceLike = {
        buffer: null,
        loop: false,
        loopStart: 0,
        loopEnd: 0,
        playbackRate: { value: 1 },
        onended: null,
        connect: (g: unknown) => {
          rec.gain = g as GainLike;
        },
        disconnect: () => undefined,
        start(when: number, offset: number) {
          Object.assign(rec, { url: (this.buffer as { url: string }).url, when, offset, stop: null, loopEnd: this.loop ? this.loopEnd : null, rate: this.playbackRate.value, stopped: false });
          started.push(rec as Started);
        },
        stop(when?: number) {
          if (when === undefined) rec.stopped = true;
          else rec.stop = when;
        },
      };
      return src;
    },
    resume: async () => {
      resumes++;
      ctx.state = "running";
    },
  };
  return { ctx, started, resumes: () => resumes };
};

const settle = () => new Promise((r) => setTimeout(r, 0));

const mixerWith = (lengthsSec: Record<string, number>) => {
  const fake = fakeAudio();
  let decodes = 0;
  const timers: (() => void)[] = [];
  const m = new PreviewMixer({
    createContext: () => fake.ctx,
    decode: async (_ctx, u) => {
      decodes++;
      return { ...buf(lengthsSec[u] ?? 1), url: u };
    },
    setTimer: (fn) => (timers.push(fn), timers.length),
    clearTimer: () => undefined,
  });
  return { m, fake, decodes: () => decodes, tick: () => timers.forEach((t) => t()) };
};

test("mixer: play schedules each sound at its frame; late decodes join at the right offset", async () => {
  const { m, fake } = mixerWith({ "a.wav": 4, "b.wav": 4 });
  m.setPlan([item({ key: "a", url: "a.wav", from: 30, until: 90, fileStart: 6 }), item({ key: "b", url: "b.wav", from: 0 })], FPS, 300);
  m.play(15);
  assert.equal(fake.resumes(), 1);
  await settle(); // decodes resolve → the tick runs again (the playhead is still at 15 by the fake clock)
  const a = fake.started.find((s) => s.url === "a.wav")!;
  const b = fake.started.find((s) => s.url === "b.wav")!;
  assert.deepEqual([a.when, a.offset, a.stop], [100.5, 0.2, 102.5]);
  assert.deepEqual([b.when, b.offset], [100, 0.5]);
});

test("mixer: seek while playing restarts at the new frame; pause stops; the context is resumed once", async () => {
  const { m, fake } = mixerWith({ "a.wav": 10 });
  m.setPlan([item({ key: "a", url: "a.wav", from: 0, until: 300 })], FPS, 300);
  m.play(0);
  await settle();
  assert.equal(fake.started.length, 1);
  fake.ctx.currentTime += 1;
  m.seek(150);
  assert.equal(fake.started[0].stopped, true);
  assert.equal(fake.started.length, 2);
  assert.deepEqual([fake.started[1].when, fake.started[1].offset], [101, 5]);
  m.pause();
  assert.equal(fake.started[1].stopped, true);
  assert.deepEqual(m.activeKeys(), []);
  m.play(150);
  assert.equal(fake.resumes(), 1);
});

test("mixer: small drift is left alone, large drift re-anchors to the Player's frame", async () => {
  const { m, fake } = mixerWith({ "a.wav": 10 });
  m.setPlan([item({ key: "a", url: "a.wav" })], FPS, 300);
  m.play(0);
  await settle();
  fake.ctx.currentTime += 1; // audio clock says frame 30
  m.frame(29);
  assert.equal(fake.started.length, 1);
  m.frame(45); // 0.5 s off
  assert.equal(fake.started.length, 2);
  assert.equal(fake.started[1].offset, 1.5);
});

test("mixer: rate change re-anchors with the new playback rate", async () => {
  const { m, fake } = mixerWith({ "a.wav": 10 });
  m.setPlan([item({ key: "a", url: "a.wav", from: 60, until: 120 })], FPS, 300);
  m.play(0);
  await settle();
  assert.equal(fake.started.length, 0); // 2 s away: beyond the lookahead
  m.setRate(2);
  await settle();
  const s = fake.started.at(-1)!;
  assert.deepEqual([s.when, s.rate, s.stop], [101, 2, 102]);
});

test("mixer: an edit that sounds the same keeps playing; a real change restarts", async () => {
  const { m, fake } = mixerWith({ "a.wav": 10 });
  const fade = () => (f: number) => Math.min(1, f / 30);
  m.setPlan([item({ key: "a", url: "a.wav", gain: fade(), loop: true })], FPS, 300);
  m.play(0);
  await settle();
  m.setPlan([item({ key: "a", url: "a.wav", gain: fade(), loop: true })], FPS, 300);
  assert.equal(fake.started.length, 1);
  m.setPlan([item({ key: "a", url: "a.wav", gain: 0.5, loop: true })], FPS, 300);
  assert.equal(fake.started.length, 2);
  assert.equal(planKey([item({ gain: fade() })]), planKey([item({ gain: fade() })]));
});

test("mixer: music fades step per frame; the Player's volume and mute act on the master", async () => {
  const { m, fake } = mixerWith({ "m.wav": 30 });
  m.setPlan([item({ key: "m", url: "m.wav", gain: (f) => Math.min(1, f / 30) * 0.5, loop: true })], FPS, 300);
  m.play(0);
  await settle();
  const g = fake.started[0].gain.gain as unknown as { value: number; events: [number, number][] };
  assert.equal(g.value, 0);
  // 1.5 s lookahead → frames 0..45 stepped, each at its time.
  assert.deepEqual(g.events[15], [0.25, 100.5]);
  assert.deepEqual(g.events.at(-1), [0.5, 101.5]);
  m.setVolume(0.4, false);
  m.setVolume(0.4, true);
  // (the master gain is the first gain the context made)
});

test("mixer: playOnce plays a file to the end and can be stopped", async () => {
  const { m, fake } = mixerWith({ "t.wav": 2 });
  let ended = 0;
  const stop = m.playOnce("t.wav", () => ended++);
  await settle();
  assert.equal(fake.started.length, 1);
  stop();
  assert.equal(fake.started[0].stopped, true);
  assert.equal(ended, 0);
  const errors: string[] = [];
  const bad = new PreviewMixer({
    createContext: () => fake.ctx,
    decode: async () => {
      throw new Error("x");
    },
  });
  bad.playOnce("missing.wav", (e) => e && errors.push(e));
  await settle();
  await settle();
  assert.equal(errors.length, 1);
});
