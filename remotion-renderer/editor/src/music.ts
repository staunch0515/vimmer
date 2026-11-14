import { invoke } from "@tauri-apps/api/core";
import { inTauri } from "./project";
import type { Music, VideoTimeline } from "../../src/types/timeline";

/** A library track as `vimmer music --json` reports it. */
export type LibraryTrack = {
  id: string;
  title: string;
  source: string;
  moods: string[];
  energy: number;
  bpm: number | null;
  beat_offset_ms: number;
  duration_ms: number | null;
  loopable: boolean;
  license: string;
};

export type MusicLibrary = { moods: string[]; main_moods: string[]; tracks: LibraryTrack[]; missing: string[] };

/** Which track a pick gets for a timeline, and why (`vimmer music pick --json`). */
export type PickedTrack = {
  mood: string;
  energy: number | null;
  track: LibraryTrack;
  reason: string;
  candidates: { id: string; energy_distance: number | null }[];
};

const MUSIC_ENDPOINT = "/__vimmer/music";

type Reply<T> = ({ ok: true } & T) | { ok: false; error: string };

const call = async <T>(body: Record<string, unknown>, tauri: () => Promise<Reply<T>>): Promise<T | { error: string }> => {
  try {
    const reply = inTauri()
      ? await tauri()
      : await fetch(MUSIC_ENDPOINT, {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify(body),
        }).then((r) => {
          if (r.status === 404) return { ok: false, error: "Music by mood needs the editor dev server (npm run editor) or the desktop app." } as Reply<T>;
          return r.json() as Promise<Reply<T>>;
        });
    if (!reply.ok) return { error: reply.error };
    const { ok: _ok, ...rest } = reply;
    return rest as T;
  } catch (e) {
    return { error: `Music library unavailable: ${String(e)}` };
  }
};

/** The library's moods and tracks (public/assets/music/library/library.json). */
export const loadMusicLibrary = (): Promise<MusicLibrary | { error: string }> =>
  call<MusicLibrary>({ action: "list" }, () => invoke("music_library"));

/**
 * The track the timeline's `music.pick` gets, decided by the same Rust code as a render (the
 * choice depends on the scene ids and the first scene's text, so it is asked for, not guessed).
 */
export const pickMusic = (timeline: VideoTimeline): Promise<PickedTrack | { error: string }> => {
  const timelineJson = JSON.stringify(timeline);
  return call<PickedTrack>({ action: "pick", timelineJson }, () => invoke("music_pick", { timelineJson }));
};

/** What the pick depends on: re-ask only when this changes. */
export const pickKey = (t: VideoTimeline): string | null =>
  t.music?.pick ? JSON.stringify([t.music.pick, t.scenes.map((s) => s.id), t.scenes[0]?.text ?? ""]) : null;

/** The music as the render will play it: a pick replaced by the chosen file. */
export const resolvedMusic = (music: Music | null | undefined, picked: PickedTrack | null): Music | null => {
  if (!music) return null;
  if (!music.pick) return music;
  if (!picked) return null;
  const { pick: _pick, ...rest } = music;
  return { ...rest, source: picked.track.source };
};

/**
 * The music for beat markers: like the render, a picked track with beat sync on and no tempo set
 * by hand uses the library's tempo.
 */
export const musicForBeats = (music: Music | null | undefined, picked: PickedTrack | null): Music | null => {
  const resolved = resolvedMusic(music, picked);
  if (!resolved || !music?.pick || !picked?.track.bpm) return resolved;
  const syncOn = !!music.beat_sync && music.beat_sync.enabled !== false;
  return syncOn && !music.beats ? { ...resolved, beats: { bpm: picked.track.bpm, offset_ms: picked.track.beat_offset_ms } } : resolved;
};
