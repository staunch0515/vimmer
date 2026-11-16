import assert from "node:assert/strict";
import { test } from "node:test";
import * as vo from "../editor/src/voiceover.ts";
import { record, undo, type Undoable } from "../editor/src/history.ts";
import type { Scene, VideoTimeline } from "../src/types/timeline.ts";

const scene = (id: string, ms: number, p: Partial<Scene> = {}): Scene => ({
  id,
  duration_ms: ms,
  text: id,
  highlights: [],
  image: null,
  motion: "static",
  audio: null,
  ...p,
});

const base = (): VideoTimeline => ({
  fps: 30,
  resolution: { width: 1920, height: 1080 },
  scenes: [scene("a", 3000), scene("b", 3000), scene("c", 3000)],
  subtitles: [],
});

const L: vo.Lengths = { "rec/one.wav": 2000, "rec/two.wav": 1500, "rec/long.wav": 12000 };

test("adding places a clip at the playhead, or after the clip already playing there", () => {
  const t0 = base();
  const one = vo.addClip(t0, "rec/one.wav", 500, L);
  assert.ok(!("error" in one));
  assert.deepEqual(one.timeline.voiceover, [{ id: "vo-1", source: "rec/one.wav", start_ms: 500 }]);
  // At 1 s the first clip still plays (500..2500): the new one starts when it ends.
  const two = vo.addClip(one.timeline, "rec/two.wav", 1000, L);
  assert.ok(!("error" in two));
  assert.deepEqual(two.timeline.voiceover?.map((c) => [c.id, c.start_ms]), [["vo-1", 500], ["vo-2", 2500]]);
  assert.equal(two.index, 1);
  // Before the first clip but too long to fit before it: goes after.
  const three = vo.addClip(two.timeline, "rec/two.wav", 0, L);
  assert.ok(!("error" in three));
  assert.equal(three.timeline.voiceover?.find((c) => c.id === "vo-3")?.start_ms, 4000);
  // Past the end: refused with a message.
  const late = vo.addClip(base(), "rec/one.wav", 99_000, L);
  assert.ok(!("error" in late) && late.timeline.voiceover?.[0].start_ms === 8999);
  assert.ok("error" in vo.addClip(base(), " ", 0, L));
});

test("an empty track is not written; defaults are not written", () => {
  const made = vo.addClip(base(), "rec/one.wav", 0, L);
  assert.ok(!("error" in made));
  const gone = vo.deleteClip(made.timeline, 0);
  assert.equal("voiceover" in gone, false);
  assert.deepEqual(JSON.stringify(gone), JSON.stringify(base()));
  const g = vo.setGain(made.timeline, 0, 1);
  assert.deepEqual(g.voiceover, [{ id: "vo-1", source: "rec/one.wav", start_ms: 0 }]);
  assert.equal(vo.setGain(made.timeline, 0, 9).voiceover?.[0].gain, vo.MAX_VOICEOVER_GAIN);
});

test("moving never lands on a neighbour or past the end", () => {
  let t: VideoTimeline = { ...base(), voiceover: [{ id: "a", source: "rec/one.wav", start_ms: 0 }, { id: "b", source: "rec/two.wav", start_ms: 5000 }] };
  // a (2 s) wants 4 s: it would run into b at 5 s, so it stops at 3 s.
  t = vo.moveClip(t, 0, 4000, L);
  assert.equal(t.voiceover?.[0].start_ms, 3000);
  // b wants 1 s: a ends at 5 s, so b stays at 5 s (unchanged document).
  assert.equal(vo.moveClip(t, 1, 1000, L), t);
  // b to the end: clamped inside the video.
  assert.equal(vo.moveClip(t, 1, 50_000, L).voiceover?.[1].start_ms, 8999);
});

test("trimming the start keeps the sound in place; trims stay inside the file and the neighbours", () => {
  let t: VideoTimeline = { ...base(), voiceover: [{ id: "a", source: "rec/one.wav", start_ms: 1000 }, { id: "b", source: "rec/two.wav", start_ms: 3500 }] };
  t = vo.trimClip(t, 0, "start", 300, L);
  assert.deepEqual(t.voiceover?.[0], { id: "a", source: "rec/one.wav", start_ms: 1300, trim_start_ms: 300 });
  // Untrimming past the file start is clamped to 0 (the clip back at 1000).
  t = vo.trimClip(t, 0, "start", -500, L);
  assert.deepEqual(t.voiceover?.[0], { id: "a", source: "rec/one.wav", start_ms: 1000 });
  // The end: at least MIN_CLIP_MS stays.
  const short = vo.trimClip(t, 0, "end", 5000, L);
  assert.equal(short.voiceover?.[0].trim_end_ms, 2000 - vo.MIN_CLIP_MS);
  assert.equal(vo.playedMs(short.voiceover![0], L), vo.MIN_CLIP_MS);
  // Untrimming the end cannot run into b (a ends at 3000 with 1000 trimmed; b starts at 3500).
  const trimmed = vo.trimClip(t, 0, "end", 1000, L);
  const back = vo.trimClip(trimmed, 0, "end", 0, L);
  assert.equal(back.voiceover?.[0].trim_end_ms, undefined);
  const tight: VideoTimeline = { ...trimmed, voiceover: [trimmed.voiceover![0], { id: "b", source: "rec/two.wav", start_ms: 2500 }] };
  assert.equal(vo.trimClip(tight, 0, "end", 0, L).voiceover?.[0].trim_end_ms, 500);
});

test("移到旁白音轨: the scene's audio becomes a clip at the same absolute time, narration off", () => {
  const t: VideoTimeline = {
    ...base(),
    scenes: [scene("a", 3000), scene("b", 3000, { audio: { source: "rec/long.wav", offset_ms: 250 }, narration: { voice: null } }), scene("c", 3000)],
  };
  const moved = vo.moveSceneAudioToTrack(t, 1, L);
  assert.ok(!("error" in moved));
  assert.deepEqual(moved.timeline.voiceover, [{ id: "vo-1", source: "rec/long.wav", start_ms: 3250 }]);
  assert.equal(moved.timeline.scenes[1].audio, null);
  assert.equal(moved.timeline.scenes[1].narration, null);
  assert.deepEqual(moved.timeline.scenes[0], t.scenes[0]);
  // A clip already playing then: refused.
  const busy = { ...t, voiceover: [{ id: "x", source: "rec/one.wav", start_ms: 4000 }] };
  const refused = vo.moveSceneAudioToTrack(busy, 1, L);
  assert.ok("error" in refused && refused.error.includes("“x”"));
  assert.ok("error" in vo.moveSceneAudioToTrack(t, 0, L));
});

test("problems mirror validation and the measured overlap check", () => {
  const t: VideoTimeline = {
    ...base(),
    voiceover: [
      { id: "a", source: "rec/one.wav", start_ms: 0, gain: 5 },
      { id: "a", source: "", start_ms: 1000 },
      { id: "late", source: "rec/two.wav", start_ms: 9000 },
    ],
  };
  const p = vo.voiceoverProblems(t, L);
  assert.deepEqual(p, [
    "Voice-over clip “a”: gain must be between 0 and 4.",
    "Two voice-over clips are called “a”.",
    "Voice-over clip “a” has no audio file.",
    "Voice-over clip “late” starts after the end of the video.",
  ]);
  const overlap: VideoTimeline = { ...base(), voiceover: [{ id: "a", source: "rec/one.wav", start_ms: 0 }, { id: "b", source: "rec/two.wav", start_ms: 1500 }] };
  assert.deepEqual(vo.voiceoverProblems(overlap, L), ["Voice-over clips “a” and “b” overlap."]);
  assert.deepEqual(vo.voiceoverProblems(overlap, {}), [], "unmeasured clips are not judged");
  assert.deepEqual(vo.voiceoverProblems(base()), []);
});

test("a drag (move or trim) is one undo step", () => {
  type S = Undoable<VideoTimeline>;
  const same = (a: VideoTimeline, b: VideoTimeline) => JSON.stringify(a) === JSON.stringify(b);
  const start: VideoTimeline = { ...base(), voiceover: [{ id: "a", source: "rec/one.wav", start_ms: 0 }] };
  let s: S = { timeline: start, history: [], future: [] };
  // The lane commits once, on pointer up.
  s = record(s, { ...s, timeline: vo.moveClip(s.timeline, 0, 2000, L) }, null, same);
  s = record(s, { ...s, timeline: vo.trimClip(s.timeline, 0, "start", 400, L) }, null, same);
  assert.equal(s.history.length, 2);
  assert.deepEqual(s.timeline.voiceover, [{ id: "a", source: "rec/one.wav", start_ms: 2400, trim_start_ms: 400 }]);
  s = undo(s)!;
  assert.deepEqual(s.timeline.voiceover, [{ id: "a", source: "rec/one.wav", start_ms: 2000 }]);
  s = undo(s)!;
  assert.deepEqual(s.timeline, start);
  // A gain slider drag (same merge key, within a second) is one step too.
  let g: S = { timeline: start, history: [], future: [] };
  for (const [k, v] of [0.9, 0.8, 0.7].entries()) {
    g = record(g, { ...g, timeline: vo.setGain(g.timeline, 0, v) }, { key: "gain", at: k * 100 }, same);
  }
  assert.equal(g.history.length, 1);
  assert.equal(g.timeline.voiceover?.[0].gain, 0.7);
});

test("scenes under a clip", () => {
  const t = base();
  assert.deepEqual(vo.scenesUnder(t, { id: "a", source: "rec/one.wav", start_ms: 2500 }, L), [0, 1]);
  assert.deepEqual(vo.scenesUnder(t, { id: "a", source: "rec/long.wav", start_ms: 0 }, L), [0, 1, 2]);
  assert.deepEqual(vo.scenesUnder(t, { id: "a", source: "rec/two.wav", start_ms: 3000 }, L), [1]);
});
