import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  isDefaultLayout,
  pageAt,
  pageCapEm,
  proportionalStarts,
  resolveTextLayout,
  sceneStarts,
  scenePages,
  splitPages,
  topInsetPx,
} from "../src/textPages.ts";
import type { Brand, Paging, Scene, TextPosition } from "../src/types/timeline.ts";

type Case = { name: string; kind: string; expected: unknown; [k: string]: any };
const vectors: { cases: Case[] } = JSON.parse(
  readFileSync(new URL("../../rust-core/crates/vimmer-dsl/tests/text_pages_vectors.json", import.meta.url), "utf8"),
);

const r3 = (v: number) => Math.round(v * 1000) / 1000;

test("the renderer pages and times exactly like the Rust render (shared vectors)", () => {
  assert.ok(vectors.cases.length >= 15);
  for (const c of vectors.cases) {
    let got: unknown;
    if (c.kind === "split") {
      const { width, height } = c.resolution;
      got = { cap_em: r3(pageCapEm(c.size, width, height)), pages: splitPages(c.text, c.paging as Paging, c.size, width, height) };
    } else if (c.kind === "proportional") {
      got = proportionalStarts(c.pages, c.from_ms, c.span_ms, c.min_ms);
    } else if (c.kind === "scene") {
      got = sceneStarts(c.pages, c.duration_ms);
    } else if (c.kind === "top_inset") {
      got = r3(topInsetPx(c.position as TextPosition, c.brand as Brand, c.scene, c.resolution.width, c.resolution.height));
    }
    assert.deepEqual(got, c.expected, c.name);
  }
});

test("nothing set is the old layout; each part falls back to the brand", () => {
  assert.ok(isDefaultLayout(undefined, null));
  assert.ok(isDefaultLayout({}, { text_layout: {} }));
  assert.ok(!isDefaultLayout({ size: 92 }, null));
  assert.ok(!isDefaultLayout(null, { text_layout: { position: "top" } }));
  assert.deepEqual(resolveTextLayout(null, null), { size: 92, position: "bottom", paging: "off" });
  assert.deepEqual(resolveTextLayout({ size: 120 }, { text_layout: { size: 60, position: "middle" } }), {
    size: 120,
    position: "middle",
    paging: "off",
  });
});

const scene = (extra: Partial<Scene>): Scene => ({
  id: "s",
  duration_ms: 9000,
  text: "第一句话在这里。第二句话也在这里。第三句话就在这里！",
  highlights: [],
  image: null,
  motion: "static",
  audio: null,
  ...extra,
});
const LAND = { width: 1920, height: 1080 };

test("scene pages: none unpaged, timed by characters in the preview, the render's when given", () => {
  assert.deepEqual(scenePages(scene({}), null, LAND), []);
  assert.deepEqual(scenePages(scene({ text_layout: { paging: "auto" } }), null, LAND), [], "fits one page at 92 px");
  const paged = scenePages(scene({ text_layout: { paging: "auto", size: 160 } }), null, LAND);
  assert.deepEqual(
    paged.map((p) => p.text),
    ["第一句话在这里。", "第二句话也在这里。", "第三句话就在这里！"],
  );
  // 7 / 8 / 8 characters over 9000 ms, 1200 ms each first.
  assert.deepEqual(
    paged.map((p) => p.start_ms),
    [0, 1200 + Math.floor((5400 * 7) / 23), 2400 + Math.floor((5400 * 15) / 23)],
  );
  const given = [
    { text: "a", start_ms: 0 },
    { text: "b", start_ms: 1234 },
  ];
  assert.equal(scenePages(scene({ text_layout: { paging: "auto" }, text_pages: given }), null, LAND), given);
  // The brand's paging applies to every scene.
  assert.equal(scenePages(scene({}), { text_layout: { paging: "manual" } }, LAND).length, 0);
  assert.equal(scenePages(scene({ text: "a\n\nb" }), { text_layout: { paging: "manual" } }, LAND).length, 2);
});

test("the page showing at a time", () => {
  const pages = [{ start_ms: 0 }, { start_ms: 1000 }, { start_ms: 2500 }];
  assert.equal(pageAt(pages, 0), 0);
  assert.equal(pageAt(pages, 999), 0);
  assert.equal(pageAt(pages, 1000), 1);
  assert.equal(pageAt(pages, 99999), 2);
});

test("splitting is deterministic", () => {
  const t = "一杯好咖啡。需要耐心。还需要好的豆子，新鲜的水，以及合适的温度。Enjoy it slowly. 好吗？";
  const a = splitPages(t, "auto", 80, 1080, 1920);
  for (let i = 0; i < 5; i++) assert.deepEqual(splitPages(t, "auto", 80, 1080, 1920), a);
});
