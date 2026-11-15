import assert from "node:assert/strict";
import { test } from "node:test";
import {
  assetFacts,
  cardHue,
  countAssets,
  filterAssets,
  filterProjects,
  formatBytes,
  formatCost,
  formatDuration,
  formatUsd,
  formatWhen,
  kindOfFile,
  usedByLabel,
} from "../editor/src/studio/format.ts";
import type { AssetInfo, ProjectSummary } from "../editor/src/studio/types.ts";

const project = (name: string): ProjectSummary => ({
  dir: `/p/${name}`, name, createdMs: 0, updatedMs: 0, lang: null, orientation: "landscape", durationMs: 0, origin: "blank", renders: 0, thumb: null, thumbVersion: 0,
});
const asset = (path: string, kind: AssetInfo["kind"], usedBy: string[] = [], extra: Partial<AssetInfo> = {}): AssetInfo => ({
  path, name: path.split("/").pop()!, kind, bytes: 2_100_000, modifiedMs: 0, width: null, height: null, durationMs: null, usedBy, file: `/p/${path}`, ...extra,
});

test("durations read as m:ss, with hours when needed", () => {
  assert.equal(formatDuration(0), "0:00");
  assert.equal(formatDuration(7_400), "0:07");
  assert.equal(formatDuration(65_000), "1:05");
  assert.equal(formatDuration(3_723_000), "1:02:03");
  assert.equal(formatDuration(-5), "0:00");
});

test("sizes use the unit a person would", () => {
  assert.equal(formatBytes(512), "512 B");
  assert.equal(formatBytes(1_240), "1.2 KB");
  assert.equal(formatBytes(34_400_000), "34 MB");
  assert.equal(formatBytes(999_900), "1 MB");
  assert.equal(formatBytes(1_500_000_000), "1.5 GB");
});

test("times are relative for a week, then a date", () => {
  const now = new Date(2026, 10, 15, 12, 0).getTime();
  const min = 60_000;
  assert.equal(formatWhen(now - 20_000, now), "just now");
  assert.equal(formatWhen(now - 5 * min, now), "5 min ago");
  assert.equal(formatWhen(now - 3 * 60 * min, now), "3 h ago");
  assert.equal(formatWhen(now - 30 * 60 * min, now), "yesterday");
  assert.equal(formatWhen(now - 4 * 24 * 60 * min, now), "4 days ago");
  assert.equal(formatWhen(new Date(2026, 9, 3).getTime(), now), "2026-10-03");
});

test("costs keep the digits that matter", () => {
  assert.equal(formatUsd(0), "$0");
  assert.equal(formatUsd(0.0004), "< $0.001");
  assert.equal(formatUsd(0.0114), "$0.011");
  assert.equal(formatUsd(0.03), "$0.03");
  assert.equal(formatUsd(0.095), "$0.095");
  assert.equal(formatUsd(1.2), "$1.20");
  assert.equal(formatCost({ typical: 0.033, worst: 0.11 }), "about $0.033 (at most about $0.11)");
});

test("search matches every word, in any case", () => {
  const all = [project("How Coffee is made"), project("西安钟楼"), project("coffee break")];
  assert.deepEqual(filterProjects(all, "  ").map((p) => p.name), all.map((p) => p.name));
  assert.deepEqual(filterProjects(all, "coffee").map((p) => p.name), ["How Coffee is made", "coffee break"]);
  assert.deepEqual(filterProjects(all, "COFFEE made").map((p) => p.name), ["How Coffee is made"]);
  assert.deepEqual(filterProjects(all, "钟楼").map((p) => p.name), ["西安钟楼"]);
  assert.deepEqual(filterProjects(all, "tea"), []);
});

test("asset filters and counts", () => {
  const assets = [asset("assets/a.png", "image", ["intro"]), asset("assets/b.png", "image"), asset("assets/c.mp4", "video"), asset("assets/s.svg", "icon", ["Brand"])];
  assert.deepEqual(countAssets(assets), { all: 4, image: 2, icon: 1, video: 1, audio: 0, font: 0, unused: 2 });
  assert.deepEqual(filterAssets(assets, "image").map((a) => a.name), ["a.png", "b.png"]);
  assert.deepEqual(filterAssets(assets, "unused").map((a) => a.name), ["b.png", "c.mp4"]);
  assert.equal(filterAssets(assets, "all").length, 4);
});

test("asset facts and usage read naturally", () => {
  assert.equal(assetFacts(asset("a.mp4", "video", [], { width: 1920, height: 1080, durationMs: 4000 })), "1920×1080 · 0:04 · 2.1 MB");
  assert.equal(assetFacts(asset("a.wav", "audio", [], { durationMs: 61_000 })), "1:01 · 2.1 MB");
  assert.equal(assetFacts(asset("a.woff2", "font")), "2.1 MB");
  assert.equal(usedByLabel([]), "Not used");
  assert.equal(usedByLabel(["intro", "Brand"]), "Used by intro, Brand");
  assert.equal(usedByLabel(["a", "b", "c", "d"]), "Used by a, b and 2 more");
});

test("file kinds follow the extensions the project accepts", () => {
  assert.equal(kindOfFile("Photo.JPG"), "image");
  assert.equal(kindOfFile("star.svg"), "icon");
  assert.equal(kindOfFile("clip.webm"), "video");
  assert.equal(kindOfFile("voice.m4a"), "audio");
  assert.equal(kindOfFile("Brand.woff2"), "font");
  assert.equal(kindOfFile("notes.txt"), null);
  assert.equal(kindOfFile("noextension"), null);
});

test("a title card's colour is stable for its name", () => {
  assert.equal(cardHue("Demo"), cardHue("Demo"));
  assert.ok(cardHue("Demo") >= 0 && cardHue("Demo") < 360);
});
