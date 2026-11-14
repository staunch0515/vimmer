import { useEffect, useRef, useState } from "react";
import type { Music, MusicPick } from "../../../src/types/timeline";
import type { MusicLibrary, PickedTrack } from "../music";

/** File or By mood. Switching to a file keeps the track the mood chose, so nothing jumps. */
export const MusicModeSwitch = ({
  music, patch, firstMood, pickedTrack,
}: {
  music: Music; patch: (p: Partial<Music>) => void; firstMood: string; pickedTrack: PickedTrack | null;
}) => {
  const byMood = !!music.pick;
  return (
    <div className="segmented" role="radiogroup" aria-label="Music source">
      <button
        role="radio"
        aria-checked={!byMood}
        className={!byMood ? "seg active" : "seg"}
        data-music-mode="file"
        onClick={() => byMood && patch({ pick: undefined, source: pickedTrack?.track.source ?? "" })}
      >
        File
      </button>
      <button
        role="radio"
        aria-checked={byMood}
        className={byMood ? "seg active" : "seg"}
        data-music-mode="mood"
        onClick={() => !byMood && patch({ source: undefined, pick: { mood: firstMood } })}
      >
        By mood
      </button>
    </div>
  );
};

const pct = (v: number) => `${Math.round(v * 100)} %`;

/** Mood, energy, the track the render will play and why, and an audition button. */
export const MoodMusicFields = ({
  pick, patch, library, libraryError, pickedTrack, pickError, audioUrl,
}: {
  pick: MusicPick;
  patch: (p: Partial<Music>) => void;
  library: MusicLibrary | null;
  libraryError: string | null;
  pickedTrack: PickedTrack | null;
  pickError: string | null;
  audioUrl: (path: string) => string;
}) => {
  const setPick = (p: Partial<MusicPick>) => {
    const next: MusicPick = { ...pick, ...p };
    if (next.energy === undefined) delete next.energy;
    if (!next.exclude?.length) delete next.exclude;
    patch({ pick: next });
  };
  const main = library?.main_moods ?? [];
  const others = (library?.moods ?? []).filter((m) => !main.includes(m));
  const known = !library || library.moods.includes(pick.mood.trim().toLowerCase());
  const track = pickedTrack?.track ?? null;
  return (
    <>
      <div className="two-col">
        <label className="field">
          <span className="field-label">Mood</span>
          <select data-music-mood value={pick.mood} onChange={(e) => setPick({ mood: e.target.value })}>
            {!known && <option value={pick.mood}>{pick.mood} (not in the library)</option>}
            {main.map((m) => (
              <option key={m} value={m}>{m}</option>
            ))}
            {others.length > 0 && (
              <optgroup label="Other tags">
                {others.map((m) => (
                  <option key={m} value={m}>{m}</option>
                ))}
              </optgroup>
            )}
          </select>
        </label>
        <div className="field">
          <span className="field-label">
            <input
              type="checkbox"
              data-music-energy-on
              checked={pick.energy !== undefined}
              onChange={(e) => setPick({ energy: e.target.checked ? 0.5 : undefined })}
            />{" "}
            Energy {pick.energy !== undefined ? pct(pick.energy) : "(any)"}
          </span>
          <input
            type="range"
            data-music-energy
            min={0}
            max={1}
            step={0.05}
            disabled={pick.energy === undefined}
            value={pick.energy ?? 0.5}
            onChange={(e) => setPick({ energy: Number(e.target.value) })}
          />
        </div>
      </div>
      {libraryError && <span className="field-error">{libraryError}</span>}
      {pickError && <span className="field-error" data-music-pick-error>{pickError}</span>}
      {track && (
        <div className="picked-track" data-music-picked={track.id}>
          <div className="picked-head">
            <span>
              ♪ <strong>{track.title || track.id}</strong> <span className="dim mono">{track.id}</span>
            </span>
            <Audition src={audioUrl(track.source)} />
          </div>
          <div className="dim small">
            energy {track.energy.toFixed(2)}
            {track.bpm ? ` · ${track.bpm} BPM` : ""}
            {track.duration_ms ? ` · ${(track.duration_ms / 1000).toFixed(1)} s${track.loopable ? " loop" : ""}` : ""}
          </div>
          <div className="dim small">{pickedTrack?.reason}</div>
          {(pickedTrack?.candidates.length ?? 0) > 1 && (
            <button
              className="btn small"
              data-music-another
              title="Exclude this track so the next best one plays"
              onClick={() => setPick({ exclude: [...(pick.exclude ?? []), track.id] })}
            >
              Another track
            </button>
          )}
        </div>
      )}
      {pick.exclude && pick.exclude.length > 0 && (
        <div className="dim small">
          Excluded: {pick.exclude.join(", ")}{" "}
          <button className="link" onClick={() => setPick({ exclude: undefined })}>clear</button>
        </div>
      )}
    </>
  );
};

/** Plays a track from the start; press again to stop. */
const Audition = ({ src }: { src: string }) => {
  const audio = useRef<HTMLAudioElement | null>(null);
  const [playing, setPlaying] = useState(false);
  useEffect(() => {
    const a = new Audio();
    a.onended = () => setPlaying(false);
    audio.current = a;
    return () => {
      a.pause();
      audio.current = null;
    };
  }, []);
  useEffect(() => {
    // A different track: stop the old one.
    audio.current?.pause();
    setPlaying(false);
  }, [src]);
  const toggle = () => {
    const a = audio.current;
    if (!a) return;
    if (playing) {
      a.pause();
      setPlaying(false);
      return;
    }
    a.src = src;
    a.currentTime = 0;
    void a.play().then(
      () => setPlaying(true),
      () => setPlaying(false),
    );
  };
  return (
    <button className="btn small" data-music-audition onClick={toggle}>
      {playing ? "■ Stop" : "▶ Audition"}
    </button>
  );
};
