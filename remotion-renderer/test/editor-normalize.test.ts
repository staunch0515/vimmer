import assert from "node:assert/strict";
import { test } from "node:test";
import { normalizeScene } from "../editor/src/normalize.ts";
import type { Scene } from "../src/types/timeline.ts";

test("a scene with only id and duration_ms gets the DSL defaults", () => {
  const s = normalizeScene({ id: "a", duration_ms: 1000 } as Scene);
  assert.deepEqual(s, { id: "a", duration_ms: 1000, text: null, highlights: [], image: null, motion: "static", audio: null });
});

test("present fields and optional keys are left alone", () => {
  const s = normalizeScene({ id: "a", duration_ms: 1, highlights: ["x"], layers: [] } as unknown as Scene);
  assert.deepEqual(s.highlights, ["x"]);
  assert.ok(!("video" in s) && !("keyframes" in s));
});
