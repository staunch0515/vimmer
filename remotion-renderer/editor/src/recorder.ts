// Recording (录音) on the microphone, and saving a take. The pure parts are in recording.ts.
//
// Capture: getUserMedia + Web Audio (a ScriptProcessor copying the input blocks), so the same
// code records in the desktop app's WKWebView and in the browser editor, and the take is plain
// PCM: no codec, no conversion. The context asks for 48 kHz; when the browser runs at another
// rate, the take is resampled. Echo cancellation, noise suppression and auto gain are off: they
// colour the voice and pump the level.
//
// Saving: the desktop app's `save_recording` command (into the open project's
// assets/recordings/), else the editor dev server's /__vimmer/recording (into public/).
import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { inTauri } from "./project";
import { addProjectFile, openProjectDir } from "./studio/session";
import { COUNTDOWN_S, MAX_RECORD_MS, RECORD_RATE, encodeWav, joinBlocks, levels, resample, toBase64 } from "./recording";

export type Phase = "idle" | "asking" | "countdown" | "recording" | "stopped" | "error";

export type Take = { wav: Uint8Array; url: string; ms: number };

export type Recorder = {
  phase: Phase;
  /** Seconds left in the countdown. */
  count: number;
  elapsedMs: number;
  peakDb: number;
  take: Take | null;
  error: string | null;
  start: () => void;
  stop: () => void;
  discard: () => void;
  /** Ref for the take's playback <audio>, so discarding stops it before its file goes away. */
  playbackRef: (el: HTMLAudioElement | null) => void;
};

const BLOCK = 4096;

/** How long a discarded take's blob URL stays valid after its <audio> let go of it. */
const REVOKE_DELAY_MS = 2000;

/**
 * Stops a take's playback element and lets go of its file: pause, drop the source, reset. Done
 * before the blob URL is revoked, so WebKit never loads from (or plays) a URL that is going away
 * while the preview starts playing (see mediaGate.ts).
 */
const detach = (el: HTMLAudioElement | null): void => {
  if (!el) return;
  try {
    el.pause();
    el.removeAttribute("src");
    el.load();
  } catch {
    /* already gone */
  }
};

const micError = (e: unknown): string => {
  const name = (e as { name?: string })?.name ?? "";
  if (name === "NotAllowedError" || name === "SecurityError") {
    return "没有麦克风权限。请在「系统设置 → 隐私与安全性 → 麦克风」里允许 Vimmer Studio（或启动它的终端），然后再试。Microphone access was denied.";
  }
  if (name === "NotFoundError" || name === "OverconstrainedError") return "没有找到麦克风。No microphone was found.";
  if (!navigator.mediaDevices?.getUserMedia) return "这个窗口不能录音（需要 https 或 localhost）。Recording is not available here.";
  return `麦克风打不开：${String(e)}`;
};

/**
 * One take at a time: `start` asks for the microphone, counts down (the meter already moves, so
 * the level can be checked), then records until `stop` (or MAX_RECORD_MS). `onTick` gets the
 * recorded length while recording (the teleprompter moves the preview with it).
 */
export const useRecorder = (onTick?: (elapsedMs: number) => void): Recorder => {
  const [phase, setPhase] = useState<Phase>("idle");
  const [count, setCount] = useState(COUNTDOWN_S);
  const [elapsedMs, setElapsed] = useState(0);
  const [peakDb, setPeak] = useState(Number.NEGATIVE_INFINITY);
  const [take, setTake] = useState<Take | null>(null);
  const [error, setError] = useState<string | null>(null);
  const live = useRef<{ stream: MediaStream; ctx: AudioContext; proc: ScriptProcessorNode; blocks: Float32Array[]; recording: boolean; timer: number | null } | null>(null);
  const tick = useRef(onTick);
  tick.current = onTick;
  const peak = useRef(Number.NEGATIVE_INFINITY);
  const playback = useRef<HTMLAudioElement | null>(null);
  const playbackRef = useCallback((el: HTMLAudioElement | null) => {
    playback.current = el;
  }, []);

  const release = useCallback(() => {
    const l = live.current;
    live.current = null;
    if (!l) return null;
    if (l.timer !== null) window.clearInterval(l.timer);
    l.proc.onaudioprocess = null;
    try {
      l.proc.disconnect();
    } catch {
      /* already */
    }
    l.stream.getTracks().forEach((t) => t.stop());
    void l.ctx.close().catch(() => undefined);
    return l;
  }, []);

  const finish = useCallback(() => {
    const l = release();
    if (!l) return;
    const raw = joinBlocks(l.blocks);
    const samples = resample(raw, l.ctx.sampleRate, RECORD_RATE);
    if (samples.length === 0) {
      setPhase("idle");
      return;
    }
    const wav = encodeWav(samples);
    const url = URL.createObjectURL(new Blob([wav.slice().buffer], { type: "audio/wav" }));
    setTake({ wav, url, ms: Math.round((samples.length / RECORD_RATE) * 1000) });
    setPhase("stopped");
  }, [release]);

  const discard = useCallback(() => {
    release();
    detach(playback.current);
    playback.current = null;
    setTake((t) => {
      // Revoked later, once the <audio> is gone: nothing may still be reading the take.
      if (t) window.setTimeout(() => URL.revokeObjectURL(t.url), REVOKE_DELAY_MS);
      return null;
    });
    setPhase("idle");
    setElapsed(0);
    setError(null);
  }, [release]);

  const start = useCallback(() => {
    discard();
    setPhase("asking");
    void (async () => {
      let stream: MediaStream;
      try {
        stream = await navigator.mediaDevices.getUserMedia({
          audio: { channelCount: 1, echoCancellation: false, noiseSuppression: false, autoGainControl: false },
        });
      } catch (e) {
        setError(micError(e));
        setPhase("error");
        return;
      }
      let ctx: AudioContext;
      try {
        ctx = new AudioContext({ sampleRate: RECORD_RATE });
      } catch {
        ctx = new AudioContext();
      }
      await ctx.resume().catch(() => undefined);
      const source = ctx.createMediaStreamSource(stream);
      const proc = ctx.createScriptProcessor(BLOCK, 1, 1);
      const state = { stream, ctx, proc, blocks: [] as Float32Array[], recording: false, timer: null as number | null };
      live.current = state;
      let recorded = 0;
      proc.onaudioprocess = (e) => {
        const input = e.inputBuffer.getChannelData(0);
        const lv = levels(input);
        peak.current = Math.max(peak.current, lv.peakDb);
        if (!state.recording) return;
        state.blocks.push(new Float32Array(input));
        recorded += input.length;
        const ms = (recorded / ctx.sampleRate) * 1000;
        if (ms >= MAX_RECORD_MS) window.setTimeout(finish, 0);
      };
      source.connect(proc);
      proc.connect(ctx.destination);
      // The meter and the clock update about 15 times a second.
      state.timer = window.setInterval(() => {
        setPeak(peak.current);
        peak.current = Number.NEGATIVE_INFINITY;
        if (state.recording) {
          const ms = Math.round((recorded / ctx.sampleRate) * 1000);
          setElapsed(ms);
          tick.current?.(ms);
        }
      }, 66);
      setPhase("countdown");
      for (let n = COUNTDOWN_S; n > 0; n--) {
        setCount(n);
        await new Promise((r) => window.setTimeout(r, 1000));
        if (live.current !== state) return; // cancelled during the countdown
      }
      state.recording = true;
      setElapsed(0);
      setPhase("recording");
    })();
  }, [discard, finish]);

  const stop = useCallback(() => {
    const l = live.current;
    if (!l) return;
    if (!l.recording) {
      // Stopped during the countdown: nothing recorded.
      release();
      setPhase("idle");
      return;
    }
    finish();
  }, [finish, release]);

  // Leaving the panel stops the microphone.
  useEffect(() => () => void release(), [release]);

  return { phase, count, elapsedMs, peakDb, take, error, start, stop, discard, playbackRef };
};

export type Saved = { path: string } | { error: string };

/** Saves a take as assets/recordings/<stem>-<n>.wav (project or public folder). */
export const saveTake = async (stem: string, wav: Uint8Array): Promise<Saved> => {
  const wavBase64 = toBase64(wav);
  try {
    if (inTauri()) {
      const dir = openProjectDir();
      const got = await invoke<{ path: string }>("save_recording", { args: { dir, stem, wavBase64 } });
      if (dir) addProjectFile(got.path);
      return { path: got.path };
    }
    const res = await fetch("/__vimmer/recording", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ stem, wavBase64 }),
    });
    if (res.status === 404) return { error: "保存录音需要编辑器开发服务器（npm run editor）或桌面应用。" };
    const body = (await res.json()) as { ok: boolean; path?: string; error?: string };
    return body.ok && body.path ? { path: body.path } : { error: body.error ?? "The recording could not be saved." };
  } catch (e) {
    return { error: `录音没有保存：${String(e)}` };
  }
};
