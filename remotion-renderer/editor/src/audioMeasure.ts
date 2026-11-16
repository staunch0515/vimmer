// Measuring an audio file's length without a media element (<audio>/new Audio()).
//
// In the desktop app (WKWebView) a media element loads its file through WebKit's GPU process.
// The page was seen to hang for good in a play() (waiting on the GPU process to start the audio
// session) right after a take was kept, when new files were loading; a load racing the play is
// the suspected trigger, so nothing here creates a media element. Lengths
// are only metadata, so they are read here with fetch: a WAV from its header (the first bytes
// only), any other format by decoding it with an OfflineAudioContext (no playback, no audio
// session).

/**
 * Length in ms of a PCM WAV from its first bytes (the chunks are walked, so a LIST or other chunk
 * before "data" is fine): a number, "more" when the header goes past the bytes given, or null when
 * it is not a WAV. `total` is the whole file's size, used when the data chunk gives no size.
 */
export const wavLengthMs = (head: Uint8Array, total?: number): number | "more" | null => {
  if (head.length < 12) return "more";
  const v = new DataView(head.buffer, head.byteOffset, head.byteLength);
  const tag = (at: number) => String.fromCharCode(...head.subarray(at, at + 4));
  if (tag(0) !== "RIFF" || tag(8) !== "WAVE") return null;
  let byteRate = 0;
  let at = 12;
  for (;;) {
    if (at + 8 > head.length) return "more";
    const id = tag(at);
    const size = v.getUint32(at + 4, true);
    if (id === "fmt ") {
      if (at + 24 > head.length) return "more";
      byteRate = v.getUint32(at + 16, true);
      if (!byteRate) return null;
    } else if (id === "data") {
      if (!byteRate) return null;
      // A streamed WAV leaves the size at 0 or 0xFFFFFFFF: the rest of the file is the data.
      const bytes = size === 0 || size === 0xffffffff ? (total !== undefined ? Math.max(0, total - at - 8) : null) : size;
      return bytes === null ? null : Math.round((bytes / byteRate) * 1000);
    }
    at += 8 + size + (size % 2);
  }
};

/** Bytes of a WAV read at most before giving up on finding its "data" chunk. */
const HEAD_MAX = 1 << 20;

const concat = (parts: Uint8Array[]): Uint8Array => {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.length;
  }
  return out;
};

const decodeMs = async (bytes: Uint8Array): Promise<number | null> => {
  const Ctx = (globalThis as { OfflineAudioContext?: typeof OfflineAudioContext }).OfflineAudioContext;
  if (!Ctx) return null;
  try {
    const buf = await new Ctx(1, 1, 44100).decodeAudioData(bytes.slice().buffer);
    return Number.isFinite(buf.duration) ? Math.round(buf.duration * 1000) : null;
  } catch {
    return null;
  }
};

/** The file's length in ms, or null when it cannot be read or is not audio. */
export const measureAudioFile = async (url: string, fetcher: typeof fetch = fetch): Promise<number | null> => {
  let res: Response;
  try {
    res = await fetcher(url);
  } catch {
    return null;
  }
  if (!res.ok) return null;
  const sizeHeader = Number(res.headers.get("content-length"));
  const total = Number.isFinite(sizeHeader) && sizeHeader > 0 ? sizeHeader : undefined;
  const reader = res.body?.getReader();
  if (!reader) {
    const all = new Uint8Array(await res.arrayBuffer().catch(() => new ArrayBuffer(0)));
    const ms = wavLengthMs(all, all.length);
    return typeof ms === "number" ? ms : ms === null ? decodeMs(all) : null;
  }
  const parts: Uint8Array[] = [];
  let size = 0;
  let wav = true;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (value) {
        parts.push(value);
        size += value.length;
      }
      if (done) break;
      if (!wav) continue; // not a WAV: the whole file is needed to decode it
      const ms = wavLengthMs(concat(parts), total);
      if (typeof ms === "number") {
        void reader.cancel().catch(() => undefined);
        return ms;
      }
      if (ms === "more" && size > HEAD_MAX) {
        void reader.cancel().catch(() => undefined);
        return null;
      }
      if (ms === null) wav = false;
    }
  } catch {
    return null;
  }
  const got = concat(parts);
  const ms = wavLengthMs(got, got.length);
  if (typeof ms === "number") return ms;
  return ms === null ? decodeMs(got) : null;
};
