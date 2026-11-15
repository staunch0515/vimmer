import { test } from "node:test";
import assert from "node:assert/strict";
import {
  SOFT_SHADOW,
  isEmptyStyle,
  layerTextStyle,
  outlineCss,
  resolveTextStyle,
  shadowCss,
  splitAlpha,
  styleScale,
  withGlow,
} from "../src/textStyle.ts";
import type { Brand } from "../src/types/timeline.ts";

test("nothing set resolves to an empty style (the old code path)", () => {
  assert.ok(isEmptyStyle(resolveTextStyle(undefined, null)));
  assert.ok(isEmptyStyle(resolveTextStyle(null, { colors: { text: "#fff" } })));
  assert.ok(isEmptyStyle(layerTextStyle({ background: null }, null)));
  // The scene text keeps its exact built-in shadow string.
  assert.equal(shadowCss(undefined, 1, SOFT_SHADOW), "0 4px 32px rgba(0, 0, 0, 0.6)");
  assert.equal(shadowCss(undefined, 1), undefined);
  assert.deepEqual(outlineCss(undefined, 1), {});
});

test("element parts win over the brand one by one", () => {
  const brand: Brand = { text_style: { outline: { color: "#111", width: 3 }, shadow: "soft", background: { color: "#000000aa" } } };
  const s = resolveTextStyle({ shadow: "none" }, brand);
  assert.equal(s.shadow, "none");
  assert.equal(s.outline?.color, "#111");
  assert.equal(s.background?.color, "#000000aa");
  // A text layer's own null background still falls back to the brand's box.
  assert.equal(layerTextStyle({ background: null, outline: { width: 0 } }, brand).background?.color, "#000000aa");
  assert.deepEqual(outlineCss(layerTextStyle({ outline: { width: 0 } }, brand).outline, 1), {});
});

test("outline css: stroke twice the width under the fill, scaled", () => {
  assert.deepEqual(outlineCss({ color: "#000000", width: 4 }, 1), { WebkitTextStroke: "8px #000000", paintOrder: "stroke fill" });
  assert.deepEqual(outlineCss({}, 2), { WebkitTextStroke: "16px #000000", paintOrder: "stroke fill" });
  assert.equal(styleScale(1920, 1080), 1);
  assert.equal(styleScale(1080, 1920), 1);
  assert.equal(styleScale(3840, 2160), 2);
});

test("shadow presets and custom", () => {
  assert.equal(shadowCss("none", 1, SOFT_SHADOW), "none");
  assert.equal(shadowCss("soft", 1), SOFT_SHADOW);
  assert.equal(shadowCss("soft", 2), "0 8px 64px rgba(0, 0, 0, 0.6)");
  assert.match(shadowCss("strong", 1) ?? "", /^0 2px 4px rgba\(0, 0, 0, 0.85\), /);
  assert.equal(shadowCss({ color: "#000000b3", x: 1, y: 4, blur: 12 }, 1), "1px 4px 12px #000000b3");
  assert.equal(withGlow("0 0 24px red", "none"), "0 0 24px red");
  assert.equal(withGlow("0 0 24px red", SOFT_SHADOW), `0 0 24px red, ${SOFT_SHADOW}`);
});

test("box color splits into solid + alpha", () => {
  assert.deepEqual(splitAlpha("#09090bcc"), { solid: "#09090b", alpha: 0xcc / 255 });
  assert.deepEqual(splitAlpha("#fff"), { solid: "#ffffff", alpha: 1 });
});

test("box border: none by default, px scaled, absent fill is transparent", async () => {
  const { borderCss, borderPx, TRANSPARENT } = await import("../src/textStyle.ts");
  assert.deepEqual(borderCss({ color: "#000" }, 1), {});
  assert.equal(borderPx(null, 1), 0);
  assert.deepEqual(borderCss({ border: { color: "#ffffff" } }, 1), { border: "2px solid #ffffff" });
  assert.deepEqual(borderCss({ border: { color: "#fff", width: 3 } }, 2), { border: "6px solid #fff" });
  assert.deepEqual(borderCss({ border: { color: "#fff", width: 0 } }, 1), {});
  assert.equal(splitAlpha(TRANSPARENT).alpha, 0);
});
