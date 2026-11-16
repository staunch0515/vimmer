import assert from "node:assert/strict";
import { test } from "node:test";
import {
  TAIL_MS, clipFor, lengthHint, listenKey, rememberClips, renderLength, secondsPair, toPreviewMs, toSavedMs, withNarration,
  type ListenClip,
} from "../editor/src/narration.ts";
import { TTS_QUEUE, TTS_RUNNING, helpHasTts, limiter, parseTtsArgs, ttsCliArgs } from "../editor/assistant-server.ts";
import type { Scene } from "../src/types/timeline.ts";

const scene = (p: Partial<Scene>): Scene =>
  ({ id: "a", duration_ms: 3000, text: "On screen", highlights: [], image: null, motion: "static", audio: null, narration: { voice: null }, ...p }) as Scene;

const clip = (p: Partial<ListenClip>): ListenClip => ({
  scene_id: "a", index: 0, text: "On screen", voice: null, source: "generated/tts/a-1.wav",
  speech_ms: 4200, scene_ms: 3000, render_ms: 4500, cached: false, ...p,
});

test("a clip belongs to the scene's id, engine, voice and spoken text; any change drops it", () => {
  const heard = rememberClips({}, "say", [clip({})]);
  const s = scene({});
  assert.equal(clipFor(heard, s, "say")?.source, "generated/tts/a-1.wav");
  assert.equal(clipFor(heard, s, "kokoro"), null, "another engine");
  assert.equal(clipFor(heard, scene({ text: "Edited" }), "say"), null, "text edited");
  assert.equal(clipFor(heard, scene({ narration: { voice: "Tingting" } }), "say"), null, "voice changed");
  assert.equal(clipFor(heard, scene({ id: "b" }), "say"), null, "renamed scene (new file name)");
  assert.equal(clipFor(heard, scene({ narration: { voice: null, text: "Said" } }), "say"), null, "a spoken line is what is said");
  // A blank voice is "by language", as in the render; a blank spoken line falls back to the text.
  assert.ok(clipFor(heard, scene({ narration: { voice: " ", text: "  " } }), "say"));
  // Not narrated by the render: no key at all.
  assert.equal(listenKey(scene({ narration: null }), "say"), null);
  assert.equal(listenKey(scene({ audio: { source: "x.wav", offset_ms: 0 } }), "say"), null);
  assert.equal(listenKey(scene({ text: null }), "say"), null);
});

test("the length hint says when the render will stretch the scene", () => {
  assert.equal(renderLength(3000, 4200), 4200 + TAIL_MS);
  assert.equal(renderLength(6000, 4200), 6000);
  const long = lengthHint(3000, 4200);
  assert.equal(long.text, "念完需 4.2 秒，本场景 3.0 秒，渲染时会自动拉长到 4.5 秒");
  assert.ok(long.stretched);
  assert.equal(lengthHint(6000, 4200).text, "念完需 4.2 秒，本场景 6.0 秒，时长够用");
});

test("the narrated preview plays each heard clip at its scene, as long as the render", () => {
  const t = {
    scenes: [
      scene({ id: "a", duration_ms: 3000 }),
      scene({ id: "b", duration_ms: 2000, narration: null }),
      scene({ id: "c", duration_ms: 5000, text: "Third" }),
      scene({ id: "d", duration_ms: 1000, text: "Unheard" }),
    ],
  };
  const heard = rememberClips({}, "say", [clip({}), clip({ scene_id: "c", index: 2, text: "Third", source: "generated/tts/c-1.wav", speech_ms: 1000 })]);
  const before = JSON.stringify(t);
  const p = withNarration(t, heard, "say");
  assert.equal(JSON.stringify(t), before, "the saved timeline is untouched");
  assert.deepEqual(p.timeline.scenes.map((s) => s.duration_ms), [4500, 2000, 5000, 1000]);
  assert.deepEqual(p.timeline.scenes[0].audio, { source: "generated/tts/a-1.wav", offset_ms: 0 });
  assert.equal(p.timeline.scenes[1].audio, null);
  assert.deepEqual(p.stretched, [{ id: "a", from: 3000, to: 4500 }]);
  assert.deepEqual(p.missing, ["d"]);
  // Times map scene by scene; the stretched tail of `a` holds its last saved moment.
  assert.equal(toPreviewMs(p, 0), 0);
  assert.equal(toPreviewMs(p, 3500), 5000);
  assert.equal(toSavedMs(p, 5000), 3500);
  assert.equal(toSavedMs(p, 4000), 2999);
  assert.equal(toSavedMs(p, 1200), 1200);
  // Editing the text of `a` silences it in the preview until it is heard again.
  const edited = withNarration({ scenes: [scene({ text: "New" }), ...t.scenes.slice(1)] }, heard, "say");
  assert.equal(edited.timeline.scenes[0].duration_ms, 3000);
  assert.deepEqual(edited.missing, ["a", "d"]);
});

test("the dev server checks a listen request and builds the vimmer tts command", () => {
  assert.equal(parseTtsArgs("nope"), "the request is not JSON");
  assert.equal(parseTtsArgs(JSON.stringify({ timelineJson: 1 })), "timelineJson must be a string");
  assert.equal(parseTtsArgs(JSON.stringify({ timelineJson: "{}", tts: "espeak" })), "tts must be say or kokoro");
  assert.equal(parseTtsArgs(JSON.stringify({ timelineJson: "{}", sceneIds: [1] })), "sceneIds must be a list of scene ids");
  const ok = parseTtsArgs(JSON.stringify({ timelineJson: "{}", sceneIds: ["-a", "b"], tts: "kokoro" }));
  assert.ok(typeof ok !== "string");
  assert.deepEqual(ttsCliArgs(ok, "/pub"), ["tts", "-", "--json", "--public-dir", "/pub", "--tts=kokoro", "--scene=-a", "--scene=b"]);
  const all = parseTtsArgs(JSON.stringify({ timelineJson: "{}" }));
  assert.ok(typeof all !== "string");
  assert.deepEqual(ttsCliArgs(all, "/pub"), ["tts", "-", "--json", "--public-dir", "/pub"]);
  assert.ok(helpHasTts("--json --public-dir --scene --tts"));
  assert.ok(!helpHasTts("--json --public-dir"));
});

test("at most two syntheses run at once; a full queue is refused at once", async () => {
  assert.equal(TTS_RUNNING, 2);
  const slots = limiter(TTS_RUNNING, 3);
  let now = 0;
  let peak = 0;
  const releases: (() => void)[] = [];
  const task = () =>
    slots.run(
      () =>
        new Promise<number>((done) => {
          now += 1;
          peak = Math.max(peak, now);
          releases.push(() => {
            now -= 1;
            done(1);
          });
        }),
    );
  const runs = [task(), task(), task(), task(), task()];
  await new Promise((r) => setTimeout(r, 10));
  assert.equal(slots.active, 2);
  assert.equal(slots.waiting, 3);
  await assert.rejects(task(), /busy/);
  while (releases.length || slots.active) {
    releases.shift()?.();
    await new Promise((r) => setTimeout(r, 1));
  }
  assert.deepEqual(await Promise.all(runs), [1, 1, 1, 1, 1]);
  assert.equal(peak, 2);
  assert.ok(TTS_QUEUE >= 1);
});

test("lengths that round alike get two decimals", () => {
  assert.deepEqual(secondsPair(3000, 3010), ["3.00", "3.01"]);
  assert.deepEqual(secondsPair(3000, 3430), ["3.0", "3.4"]);
  assert.equal(lengthHint(3000, 2710).text, "念完需 2.7 秒，本场景 3.00 秒，渲染时会自动拉长到 3.01 秒");
});
