// The spoken line (台词) of a scene's narration. Pure (type imports only), so it is unit-tested
// with Node (test/editor-narration.test.ts). Mirrors `Scene::spoken_text` in vimmer-dsl/src/model.rs.
import type { Narration, Scene } from "../../src/types/timeline";

const said = (t: string | null | undefined): string | null => (t && t.trim() ? t : null);

/** What the narration says: its spoken line when set, else the scene text; null without narration. */
export const spokenText = (s: Pick<Scene, "text" | "narration">): string | null =>
  s.narration ? said(s.narration.text) ?? said(s.text) : null;

/**
 * `n` with the spoken line `line`. A blank line removes the field, so the narration speaks the
 * on-screen text again and the saved JSON is exactly what it was before a line was typed.
 */
export const withSpokenLine = (n: Narration, line: string): Narration => {
  const { text: _old, ...rest } = n;
  return line.trim() ? { ...rest, text: line } : rest;
};

/** `n` with another voice, keeping its spoken line. */
export const withVoice = (n: Narration, voice: string | null): Narration => ({ ...n, voice });

/** Mirrors the Rust rule: a spoken line that is blank, over a scene without text, says nothing. */
export const nothingToSpeak = (s: Pick<Scene, "text" | "narration" | "audio">): boolean =>
  !!s.narration && s.narration.text !== undefined && !s.audio && spokenText(s) === null;
