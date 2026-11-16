// Holding the preview's play while the audio files it plays are changing.
//
// In the desktop app (WKWebView) the page was seen to hang for good inside the preview's
// HTMLMediaElement.play(): WebKit asked its GPU process to start the audio session and the answer
// never came. It happened seconds after a take was kept, when the preview's audio elements were
// switching to the new file. So keeping a take (or anything else that swaps the timeline's audio
// files) calls `audioChanging()`: the preview pauses, and a play asked for while its media
// elements are still loading waits until they have loaded (or HOLD_MAX_MS passed). Outside that
// window play starts at once, as before.
//
// The pure parts (what counts as loading, when the hold ends) are tested in node; the rest needs
// the DOM.

/** The least a hold lasts, so the Player has mounted the new files before they are checked. */
export const HOLD_MIN_MS = 400;
/** A hold never lasts longer than this, whatever is still loading. */
export const HOLD_MAX_MS = 3000;

/** What the gate reads from a media element. */
export type MediaState = { src: string; networkState: number; readyState: number; error: unknown };

const NETWORK_LOADING = 2;
const HAVE_METADATA = 1;

/** A media element with a file that is being fetched and has no metadata yet. */
export const isLoading = (m: MediaState): boolean =>
  m.src !== "" && m.networkState === NETWORK_LOADING && m.readyState < HAVE_METADATA && !m.error;

/** Whether a play may start now: no hold, or the hold's floor passed and nothing loads, or the cap passed. */
export const mayPlay = (now: number, holdStart: number | null, media: MediaState[]): boolean => {
  if (holdStart === null) return true;
  const held = now - holdStart;
  if (held >= HOLD_MAX_MS) return true;
  return held >= HOLD_MIN_MS && !media.some(isLoading);
};

type Player = { play: () => void; pause: () => void; toggle: () => void; isPlaying: () => boolean };

let player: Player | null = null;
let root: HTMLElement | null = null;
let holdStart: number | null = null;
let waiting: number | null = null;

const now = () => performance.now();

const mediaIn = (el: HTMLElement | null): MediaState[] =>
  el
    ? [...el.querySelectorAll<HTMLMediaElement>("audio, video")].map((m) => ({
        src: m.currentSrc || m.getAttribute("src") || "",
        networkState: m.networkState,
        readyState: m.readyState,
        error: m.error,
      }))
    : [];

/** The preview's Player and the element its media elements are in (null when it unmounts). */
export const registerPreview = (p: Player | null, el: HTMLElement | null): void => {
  player = p;
  root = el;
};

/** The timeline's audio files are about to change: pause the preview and hold the next play. */
export const audioChanging = (): void => {
  try {
    if (player?.isPlaying()) player.pause();
  } catch {
    /* the Player is unmounting */
  }
  holdStart = now();
};

/** True while a play would have to wait (the click is then taken over by `playWhenSettled`). */
export const holding = (): boolean => {
  if (holdStart === null) return false;
  if (mayPlay(now(), holdStart, mediaIn(root))) {
    holdStart = null;
    return false;
  }
  return true;
};

/** Starts the preview once the hold is over (at once when there is none). A second call while waiting does nothing. */
export const playWhenSettled = (): void => {
  if (waiting !== null) return;
  const step = () => {
    waiting = null;
    if (holding()) {
      waiting = window.setTimeout(step, 50);
      return;
    }
    player?.play();
  };
  step();
};

/** Space and the play button: pause at once; play now, or once the hold is over. */
export const togglePlay = (): void => {
  if (!player) return;
  if (waiting !== null) {
    // A second press while waiting cancels the waiting play.
    window.clearTimeout(waiting);
    waiting = null;
    return;
  }
  if (player.isPlaying()) player.pause();
  else if (holding()) playWhenSettled();
  else player.toggle();
};
