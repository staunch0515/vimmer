// Lengths of the audio files the voice-over track (and scene audio) names, measured in the
// browser from the files' metadata. A file is measured once per URL; a file that fails to load
// stays unknown (the track then treats it as voiceover.ts's UNKNOWN_LENGTH_MS long).
import { useEffect, useMemo, useState } from "react";
import type { VideoTimeline } from "../../src/types/timeline";
import { resolveAsset, type AssetUrl } from "../../src/assets";
import type { Lengths } from "./voiceover";

const measured = new Map<string, Promise<number | null>>();

export const measureAudio = (url: string): Promise<number | null> => {
  let p = measured.get(url);
  if (!p) {
    p = new Promise((resolve) => {
      const a = new Audio();
      a.preload = "metadata";
      a.onloadedmetadata = () => resolve(Number.isFinite(a.duration) ? Math.round(a.duration * 1000) : null);
      a.onerror = () => resolve(null);
      a.src = url;
    });
    measured.set(url, p);
    // A failure is not remembered, so a file added later (a new recording) is measured again.
    void p.then((ms) => ms === null && measured.delete(url));
  }
  return p;
};

/** Forgets a measured file (it was recorded again under the same name). */
export const forgetAudio = (url: string): void => {
  measured.delete(url);
};

export const audioPaths = (t: VideoTimeline): string[] => {
  const paths = new Set<string>();
  (t.voiceover ?? []).forEach((c) => c.source.trim() && paths.add(c.source));
  t.scenes.forEach((s) => s.audio?.source.trim() && paths.add(s.audio.source));
  return [...paths].sort();
};

export const useAudioLengths = (timeline: VideoTimeline, assetUrl: AssetUrl | null, version = 0): Lengths => {
  const key = audioPaths(timeline).join("\n");
  const [lengths, setLengths] = useState<Lengths>({});
  useEffect(() => {
    let live = true;
    const paths = key ? key.split("\n") : [];
    void Promise.all(paths.map(async (p) => [p, await measureAudio(resolveAsset(p, assetUrl))] as const)).then((pairs) => {
      if (!live) return;
      const next: Lengths = {};
      for (const [p, ms] of pairs) if (ms !== null) next[p] = ms;
      setLengths((old) => (JSON.stringify(old) === JSON.stringify(next) ? old : next));
    });
    return () => {
      live = false;
    };
  }, [key, assetUrl, version]);
  return useMemo(() => lengths, [lengths]);
};
