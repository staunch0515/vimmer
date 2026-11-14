import { createContext, useContext, useEffect, useState } from "react";
import { AbsoluteFill, Img, cancelRender, continueRender, delayRender, useCurrentFrame, useVideoConfig } from "remotion";
import { AssetUrlContext, resolveAsset } from "../assets";
import { logoBox, logoShownOn } from "../brand";
import { layoutScenes } from "../layout";
import type { Brand, BrandFont, VideoTimeline } from "../types/timeline";

// The timeline's brand for every component below the Timeline. `null` = the built-in look.
export const BrandContext = createContext<Brand | null>(null);
export const useBrand = (): Brand | null => useContext(BrandContext);

// One FontFace per (family, url) for the page's lifetime: Remotion renders many frames (and the
// editor many previews) in the same page, and a face must be added only once.
const loaded = new Map<string, Promise<void>>();

const loadFont = (font: BrandFont, url: string): Promise<void> => {
  const key = `${font.family}\n${url}`;
  let p = loaded.get(key);
  if (!p) {
    // One file serves every weight: declaring the full range keeps the browser from faking
    // bold or thin versions of it.
    const face = new FontFace(font.family, `url("${url}")`, { weight: "100 900" });
    p = face.load().then((f) => {
      document.fonts.add(f);
    });
    loaded.set(key, p);
  }
  return p;
};

// Loads the brand fonts that have a file. No frame is captured until they are ready
// (delayRender), so the first frame already uses them; a font that fails to load fails the render.
const BrandFonts: React.FC<{ brand: Brand }> = ({ brand }) => {
  const assetUrl = useContext(AssetUrlContext);
  const fonts = [brand.fonts?.heading, brand.fonts?.body].filter((f): f is BrandFont => Boolean(f?.src && f.family));
  const key = fonts.map((f) => `${f.family}|${f.src}`).join(",");
  const [handle] = useState(() => (fonts.length > 0 ? delayRender(`Loading brand fonts ${key}`) : null));
  useEffect(() => {
    if (handle == null) return;
    Promise.all(fonts.map((f) => loadFont(f, resolveAsset(f.src as string, assetUrl))))
      .then(() => continueRender(handle))
      .catch((e: unknown) => cancelRender(new Error(`brand font failed to load (${key}): ${String(e)}`)));
    // Fonts are loaded once per mounted timeline; a changed brand remounts through `key`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [handle]);
  return null;
};

// The logo in its corner, above the scenes' layers and below the subtitles. Hidden on the
// scenes listed in `hide_on` (the scene is the one whose frame range holds the current frame).
const BrandLogo: React.FC<{ brand: Brand; timeline: VideoTimeline }> = ({ brand, timeline }) => {
  const frame = useCurrentFrame();
  const { width, height } = useVideoConfig();
  const assetUrl = useContext(AssetUrlContext);
  const box = logoBox(brand, width, height);
  if (!box || !brand.logo) return null;
  const layout = layoutScenes(timeline).find((l) => frame >= l.from && frame < l.from + l.durationInFrames);
  if (!logoShownOn(brand, layout?.scene.id)) return null;
  const { height: h, opacity, ...pos } = box;
  return (
    <AbsoluteFill style={{ pointerEvents: "none" }}>
      <Img
        src={resolveAsset(brand.logo.src, assetUrl)}
        style={{ position: "absolute", height: h, width: "auto", opacity, ...pos }}
      />
    </AbsoluteFill>
  );
};

// Font loading and the logo. Renders nothing without a brand.
export const BrandOverlay: React.FC<{ timeline: VideoTimeline }> = ({ timeline }) => {
  const brand = timeline.brand;
  if (!brand) return null;
  const fontKey = [brand.fonts?.heading, brand.fonts?.body].map((f) => `${f?.family}|${f?.src}`).join(",");
  return (
    <>
      <BrandFonts key={fontKey} brand={brand} />
      <BrandLogo brand={brand} timeline={timeline} />
    </>
  );
};
