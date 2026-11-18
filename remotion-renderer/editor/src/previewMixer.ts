// The preview's sound through Web Audio, with no media element (<audio>, new Audio()).
//
// Why: in the desktop app (WKWebView) every media element is an AVPlayer in WebKit's GPU process,
// and its file bytes are served back by the page's own process. On the iMac the app froze for good
// with the page's main thread in HTMLMediaElement.play() waiting (synchronously) for the GPU process
// to start the audio session, while the GPU process's main thread was stuck inside AVFoundation
// waiting on a seek that waited for bytes from the page: a cycle across the two processes. The
// Player seeks and plays its <Audio> tags all the time, so each scrub or play could close it.
//
// Here each file is fetched and decoded once into an AudioBuffer (cached by URL, decoded lazily,
// least recently used evicted past a byte budget), and AudioBufferSourceNodes are scheduled
// against the Player's frame: started on play, restarted on seek and rate change, stopped on pause.
// Decoding runs in the page; nothing goes through AVFoundation's player. The composition draws
// no <Audio> in the preview (PreviewAudioContext in src/audioPlan.ts), so the sounds and gains are
// exactly the plan's.

import type { AudioItem } from "../../src/audioPlan";

// ---------------------------------------------------------------- scheduling math (pure)

/** A source node to start: `when` and `stopAt` are seconds after the anchor, `offset` into the file. */
export type VoicePlan = {
  when: number;
  offset: number;
  stopAt: number | null;
  /** Looping: loop the buffer's first `loopEnd` seconds. */
  loopEnd: number | null;
  /** First timeline frame it sounds on. */
  startFrame: number;
};

export type Clock = {
  /** Timeline frame at the anchor (may be fractional). */
  frame: number;
  fps: number;
  /** Player playback rate (1 = normal). */
  rate: number;
  /** The composition's length in frames: everything stops there. */
  endFrame: number;
};

/** Remotion loops `<Audio loop>` every floor(file length × fps) frames. */
export const loopFrames = (bufferSec: number, fps: number): number => Math.max(1, Math.floor(bufferSec * fps));

/** Where `item` must be in its file at a timeline frame, in frames (looping wraps it). */
export const fileFrameAt = (item: AudioItem, frame: number, bufferSec: number, fps: number): number => {
  const rel = frame - item.from + item.fileStart;
  if (!item.loop) return rel;
  const period = loopFrames(bufferSec, fps);
  return ((rel % period) + period) % period;
};

/**
 * How to play `item` from the clock's frame on, or null when it does not sound at or after it.
 * `bufferSec` is the decoded file's length.
 */
export const planVoice = (item: AudioItem, clock: Clock, bufferSec: number): VoicePlan | null => {
  if (!(clock.rate > 0) || !(clock.fps > 0)) return null; // reverse play: silent (the Player plays media forward only)
  const speed = clock.fps * clock.rate; // timeline frames per second of audio clock
  const end = Math.min(item.until ?? Infinity, clock.endFrame);
  const start = Math.max(item.from, clock.frame);
  if (start >= end) return null;
  const fileFrame = fileFrameAt(item, start, bufferSec, clock.fps);
  const offset = fileFrame / clock.fps;
  if (!item.loop && offset >= bufferSec) return null;
  return {
    when: (start - clock.frame) / speed,
    offset,
    stopAt: Number.isFinite(end) ? (end - clock.frame) / speed : null,
    loopEnd: item.loop ? loopFrames(bufferSec, clock.fps) / clock.fps : null,
    startFrame: start,
  };
};

/** The item's gain at a timeline frame (before the Player's volume), as Remotion evaluates it. */
export const gainAt = (item: AudioItem, frame: number, bufferSec: number, fps: number): number => {
  if (typeof item.gain === "number") return Math.max(0, item.gain);
  const f = item.loop ? fileFrameAt(item, frame, bufferSec, fps) : frame - item.from + item.fileStart;
  return Math.max(0, item.gain(f));
};

/** Does the item sound anywhere in [from, to) (frames)? Unknown file lengths count as sounding. */
export const overlaps = (item: AudioItem, from: number, to: number): boolean =>
  item.from < to && (item.until ?? Infinity) > from;

// ---------------------------------------------------------------- decoded-buffer cache

export type BufferLike = { duration: number; length: number; numberOfChannels: number };

/** Decoded PCM bytes (Float32 per sample per channel). */
export const bufferBytes = (b: BufferLike): number => b.length * b.numberOfChannels * 4;

/** About 6 min of 48 kHz stereo; a long music bed is the biggest single entry. */
export const DEFAULT_CACHE_BYTES = 128 * 1024 * 1024;

/** A failed load is not retried for this long (a missing file would otherwise be fetched every tick). */
const RETRY_MS = 5000;

/** Decoded buffers by URL: loaded once (concurrent asks share the load), evicted least recently used. */
export class BufferCache<B extends BufferLike> {
  private entries = new Map<string, { buf: B; bytes: number; used: number }>();
  private loading = new Map<string, Promise<B | null>>();
  private failed = new Map<string, number>();
  private tick = 0;
  private load: (url: string) => Promise<B>;
  private maxBytes: number;
  private now: () => number;
  constructor(load: (url: string) => Promise<B>, maxBytes = DEFAULT_CACHE_BYTES, now: () => number = () => Date.now()) {
    this.load = load;
    this.maxBytes = maxBytes;
    this.now = now;
  }

  /** The buffer if decoded (marks it used). */
  get(url: string): B | undefined {
    const e = this.entries.get(url);
    if (e) e.used = ++this.tick;
    return e?.buf;
  }

  has(url: string): boolean {
    return this.entries.has(url);
  }

  /** Decodes the file unless it is cached or loading; resolves to null when it cannot be loaded. */
  ensure(url: string, keep: ReadonlySet<string> = new Set()): Promise<B | null> {
    const cached = this.get(url);
    if (cached) return Promise.resolve(cached);
    const pending = this.loading.get(url);
    if (pending) return pending;
    const failedAt = this.failed.get(url);
    if (failedAt !== undefined && this.now() - failedAt < RETRY_MS) return Promise.resolve(null);
    const p = this.load(url).then(
      (buf) => {
        this.loading.delete(url);
        this.failed.delete(url);
        this.entries.set(url, { buf, bytes: bufferBytes(buf), used: ++this.tick });
        this.evict(new Set([...keep, url]));
        return buf;
      },
      () => {
        this.loading.delete(url);
        this.failed.set(url, this.now());
        return null;
      },
    );
    this.loading.set(url, p);
    return p;
  }

  /** Total decoded bytes held. */
  bytes(): number {
    let n = 0;
    for (const e of this.entries.values()) n += e.bytes;
    return n;
  }

  /** Drops least recently used buffers not in `keep` until the total fits the budget. */
  evict(keep: ReadonlySet<string>): void {
    let total = this.bytes();
    if (total <= this.maxBytes) return;
    const order = [...this.entries.entries()].filter(([url]) => !keep.has(url)).sort((a, b) => a[1].used - b[1].used);
    for (const [url, e] of order) {
      if (total <= this.maxBytes) break;
      this.entries.delete(url);
      total -= e.bytes;
    }
  }

  clear(): void {
    this.entries.clear();
    this.failed.clear();
  }
}

// ---------------------------------------------------------------- the mixer

// Minimal Web Audio surface, so tests can pass a fake.
export type ParamLike = { value: number; setValueAtTime(v: number, t: number): unknown; cancelScheduledValues(t: number): unknown };
export type GainLike = { gain: ParamLike; connect(dest: unknown): unknown; disconnect(): void };
export type SourceLike = {
  buffer: unknown;
  loop: boolean;
  loopStart: number;
  loopEnd: number;
  playbackRate: { value: number };
  onended: (() => void) | null;
  connect(dest: unknown): unknown;
  disconnect(): void;
  start(when: number, offset: number): void;
  stop(when?: number): void;
};
export type ContextLike = {
  currentTime: number;
  state: string;
  destination: unknown;
  createGain(): GainLike;
  createBufferSource(): SourceLike;
  resume(): Promise<void>;
  close?(): Promise<void>;
};

/** Seconds ahead that sources are started and gain steps scheduled. */
export const LOOKAHEAD_SEC = 1.5;
/** Seconds ahead of the playhead that files are decoded, so playing finds them ready. */
export const PREFETCH_SEC = 20;
/** The Player's frame and the audio clock may differ this much before the sound is re-anchored. */
export const DRIFT_SEC = 0.2;
/** Scheduler tick while playing. */
export const TICK_MS = 250;

type Voice = { item: AudioItem; src: SourceLike; gain: GainLike; gainUntil: number; bufferSec: number };

export type MixerDeps<B extends BufferLike> = {
  createContext: () => ContextLike;
  /** Fetches and decodes a file in the given context. */
  decode: (ctx: ContextLike, url: string) => Promise<B>;
  maxBytes?: number;
  setTimer?: (fn: () => void, ms: number) => unknown;
  clearTimer?: (id: unknown) => void;
};

export class PreviewMixer<B extends BufferLike = BufferLike> {
  private ctx: ContextLike | null = null;
  private master: GainLike | null = null;
  private cache: BufferCache<B>;
  private items: AudioItem[] = [];
  private fps = 30;
  private endFrame = 0;
  private rate = 1;
  private volume = 1;
  private muted = false;
  private playing = false;
  /** Audio-clock anchor: at ctx time `at` the timeline was at `frame`. */
  private anchor: { at: number; frame: number } | null = null;
  /** Paused playhead. */
  private headFrame = 0;
  private voices = new Map<string, Voice>();
  private timer: unknown = null;
  private generation = 0;

  private deps: MixerDeps<B>;

  constructor(deps: MixerDeps<B>) {
    this.deps = deps;
    this.cache = new BufferCache<B>((url) => this.decodeIn(url), deps.maxBytes);
  }

  private decodeIn(url: string): Promise<B> {
    const ctx = this.context();
    return this.deps.decode(ctx, url);
  }

  /** The audio context, made on first use. It is resumed once (unlock) and then left running. */
  private context(): ContextLike {
    if (!this.ctx) {
      this.ctx = this.deps.createContext();
      this.master = this.ctx.createGain();
      this.master.connect(this.ctx.destination);
      this.applyVolume();
    }
    return this.ctx;
  }

  /**
   * Starts the context (call from a user gesture). It is never suspended afterwards, so the audio
   * session is activated once, not on every play.
   */
  unlock(): void {
    const ctx = this.context();
    if (ctx.state !== "running") void ctx.resume().catch(() => undefined);
  }

  /** The plan to play (from audioPlan), the frame rate and the composition length. */
  setPlan(items: AudioItem[], fps: number, endFrame: number): void {
    const same = fps === this.fps && endFrame === this.endFrame && planKey(items) === planKey(this.items);
    this.items = items;
    this.fps = fps;
    this.endFrame = endFrame;
    if (same) {
      // Gain functions may be new closures with the same values: keep the playing voices.
      for (const v of this.voices.values()) v.item = items.find((i) => i.key === v.item.key) ?? v.item;
      return;
    }
    if (this.playing) this.reanchor(this.clockFrame());
    else this.prefetch(this.headFrame);
  }

  play(frame: number): void {
    this.unlock();
    this.playing = true;
    this.reanchor(frame);
    this.startTimer();
  }

  pause(frame?: number): void {
    if (frame !== undefined) this.headFrame = frame;
    else if (this.playing) this.headFrame = this.clockFrame();
    this.playing = false;
    this.anchor = null;
    this.stopAll();
    this.stopTimer();
    this.prefetch(this.headFrame);
  }

  /** The Player jumped (scrub, click on the timeline). */
  seek(frame: number): void {
    this.headFrame = frame;
    if (this.playing) this.reanchor(frame);
    else this.prefetch(frame);
  }

  /** The Player drew a frame: re-anchor if the sound drifted away from the picture. */
  frame(frame: number): void {
    if (!this.playing) {
      this.headFrame = frame;
      return;
    }
    if (Math.abs(this.clockFrame() - frame) > DRIFT_SEC * this.fps * this.rate) this.reanchor(frame);
  }

  setRate(rate: number): void {
    if (rate === this.rate) return;
    const at = this.playing ? this.clockFrame() : this.headFrame;
    this.rate = rate;
    if (this.playing) this.reanchor(at);
  }

  /** The Player's volume slider and mute button. */
  setVolume(volume: number, muted: boolean): void {
    this.volume = volume;
    this.muted = muted;
    this.applyVolume();
  }

  /**
   * Plays a whole file once, outside the timeline (试听 / audition), straight to the speakers (the
   * Player's volume does not apply). `done` is called when it ends or cannot be played; the returned
   * function stops it (and then `done` is not called).
   */
  playOnce(url: string, done: (error?: string) => void): () => void {
    this.unlock();
    let over = false;
    let src: SourceLike | null = null;
    void this.cache.ensure(url).then((buf) => {
      if (over) return;
      if (!buf) {
        over = true;
        done(`Could not play ${url}`);
        return;
      }
      const ctx = this.context();
      const s = ctx.createBufferSource();
      src = s;
      s.buffer = buf;
      s.connect(ctx.destination);
      s.onended = () => {
        if (over) return;
        over = true;
        s.disconnect();
        done();
      };
      s.start(ctx.currentTime, 0);
    });
    return () => {
      if (over) return;
      over = true;
      if (!src) return;
      src.onended = null;
      try {
        src.stop();
      } catch {
        // not started
      }
      src.disconnect();
    };
  }

  /** Stops everything and closes the context. */
  dispose(): void {
    this.playing = false;
    this.stopAll();
    this.stopTimer();
    this.cache.clear();
    void this.ctx?.close?.().catch(() => undefined);
    this.ctx = null;
    this.master = null;
  }

  // ---- for tests and the browser check
  isPlaying(): boolean {
    return this.playing;
  }
  activeKeys(): string[] {
    return [...this.voices.keys()];
  }
  cachedBytes(): number {
    return this.cache.bytes();
  }

  // ---- internals

  private applyVolume(): void {
    if (this.master) this.master.gain.value = this.muted ? 0 : Math.max(0, this.volume);
  }

  /** Timeline frame now, by the audio clock. */
  private clockFrame(): number {
    if (!this.anchor || !this.ctx) return this.headFrame;
    return this.anchor.frame + (this.ctx.currentTime - this.anchor.at) * this.fps * this.rate;
  }

  private reanchor(frame: number): void {
    this.generation++;
    this.stopAll();
    const ctx = this.context();
    this.anchor = { at: ctx.currentTime, frame };
    this.headFrame = frame;
    this.tick();
  }

  private startTimer(): void {
    if (this.timer != null) return;
    const set = this.deps.setTimer ?? ((fn, ms) => setInterval(fn, ms));
    this.timer = set(() => this.tick(), TICK_MS);
  }

  private stopTimer(): void {
    if (this.timer == null) return;
    (this.deps.clearTimer ?? ((id) => clearInterval(id as number)))(this.timer);
    this.timer = null;
  }

  private stopAll(): void {
    for (const v of this.voices.values()) stopVoice(v);
    this.voices.clear();
  }

  /** URLs that must stay decoded: what plays now and what is about to. */
  private wanted(frame: number): Set<string> {
    const to = frame + PREFETCH_SEC * this.fps * this.rate;
    return new Set(this.items.filter((i) => overlaps(i, frame, to)).map((i) => i.url));
  }

  private prefetch(frame: number): void {
    if (!this.ctx) return; // nothing decodes before the first play (no context yet)
    const keep = this.wanted(frame);
    for (const url of keep) void this.cache.ensure(url, keep);
  }

  /** Starts the sources due within the lookahead and schedules their gain steps. */
  private tick(): void {
    if (!this.playing || !this.anchor || !this.ctx) return;
    const ctx = this.ctx;
    const now = this.clockFrame();
    const speed = this.fps * this.rate;
    const horizon = now + LOOKAHEAD_SEC * speed;
    const keep = this.wanted(now);
    const gen = this.generation;
    // Voices that ended are dropped (their onended also removes them).
    for (const item of this.items) {
      if (this.voices.has(item.key) || !overlaps(item, now, horizon)) continue;
      const buf = this.cache.get(item.url);
      if (!buf) {
        // Not decoded yet: start it (late, at the right offset) once it is.
        void this.cache.ensure(item.url, keep).then((b) => {
          if (b && gen === this.generation) this.tick();
        });
        continue;
      }
      const clock: Clock = { frame: now, fps: this.fps, rate: this.rate, endFrame: this.endFrame };
      const p = planVoice(item, clock, buf.duration);
      if (!p || p.when > LOOKAHEAD_SEC) continue;
      const at = ctx.currentTime;
      const src = ctx.createBufferSource();
      src.buffer = buf;
      src.playbackRate.value = this.rate;
      if (p.loopEnd != null) {
        src.loop = true;
        src.loopStart = 0;
        src.loopEnd = p.loopEnd;
      }
      const gain = ctx.createGain();
      src.connect(gain);
      gain.connect(this.master!);
      const voice: Voice = { item, src, gain, gainUntil: p.startFrame, bufferSec: buf.duration };
      gain.gain.value = gainAt(item, p.startFrame, buf.duration, this.fps);
      src.onended = () => {
        if (this.voices.get(item.key) === voice) {
          this.voices.delete(item.key);
          stopVoice(voice);
        }
      };
      src.start(at + p.when, p.offset);
      if (p.stopAt != null) src.stop(at + p.stopAt);
      this.voices.set(item.key, voice);
    }
    // Gain functions (music fades) step once per frame, as the Player sets the volume per frame.
    for (const v of this.voices.values()) {
      if (typeof v.item.gain === "number") continue;
      const last = Math.min(Math.floor(horizon), this.endFrame);
      for (let f = Math.max(Math.ceil(v.gainUntil), Math.ceil(now)); f <= last; f++) {
        const t = ctx.currentTime + Math.max(0, (f - now) / speed);
        v.gain.gain.setValueAtTime(gainAt(v.item, f, v.bufferSec, this.fps), t);
      }
      v.gainUntil = Math.max(v.gainUntil, last + 1);
    }
    // Decode what comes within the prefetch window, so it is ready when it is due.
    for (const url of keep) if (!this.cache.has(url)) void this.cache.ensure(url, keep);
    this.cache.evict(new Set([...keep, ...[...this.voices.values()].map((v) => v.item.url)]));
  }
}

const stopVoice = (v: Voice) => {
  v.src.onended = null;
  try {
    v.src.stop();
  } catch {
    // never started or already stopped
  }
  v.src.disconnect();
  v.gain.disconnect();
};

/** What makes two plans sound the same (gain functions compared by a few samples). */
export const planKey = (items: AudioItem[]): string =>
  JSON.stringify(
    items.map((i) => [i.key, i.url, i.from, i.until, i.fileStart, i.loop, typeof i.gain === "number" ? i.gain : [0, 1, 10, 100, 1000, 10000].map((f) => (i.gain as (f: number) => number)(f))]),
  );

// ---------------------------------------------------------------- the browser's Web Audio

/** Fetches a file and decodes it in the context (no media element, no AVPlayer). */
export const fetchAndDecode = async (ctx: ContextLike, url: string): Promise<AudioBuffer> => {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`${res.status} ${url}`);
  const bytes = await res.arrayBuffer();
  return (ctx as unknown as AudioContext).decodeAudioData(bytes);
};

let shared: PreviewMixer<AudioBuffer> | null = null;

/** The one mixer of the page (its AudioContext is shared with the 试听 / audition players). */
export const previewMixer = (): PreviewMixer<AudioBuffer> => {
  shared ??= new PreviewMixer<AudioBuffer>({
    createContext: () => new AudioContext() as unknown as ContextLike,
    decode: (ctx, url) => fetchAndDecode(ctx, url),
  });
  return shared;
};
