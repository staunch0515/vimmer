import { previewMixer } from "./previewMixer";

// A stand-in for `new Audio(url)` in the editor's 试听 / audition buttons, played through the
// preview mixer's Web Audio context instead of a media element (see previewMixer.ts for why).
// Only what those buttons use: src, play(), pause(), onended, onerror, dataset.
export class AudioClip {
  src: string;
  dataset: Record<string, string> = {};
  onended: (() => void) | null = null;
  onerror: (() => void) | null = null;
  /** Ignored: a clip always plays from the start. */
  currentTime = 0;
  private stop: (() => void) | null = null;

  constructor(src = "") {
    this.src = src;
  }

  /** Starts the clip; resolves once it is asked to play (decoding errors call onerror). */
  play(): Promise<void> {
    this.pause();
    if (!this.src) return Promise.reject(new Error("no source"));
    this.stop = previewMixer().playOnce(this.src, (error) => {
      this.stop = null;
      if (error) this.onerror?.();
      else this.onended?.();
    });
    return Promise.resolve();
  }

  pause(): void {
    this.stop?.();
    this.stop = null;
  }
}
