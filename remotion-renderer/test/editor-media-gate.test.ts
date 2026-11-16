import assert from "node:assert/strict";
import { test } from "node:test";
import { encodeWav } from "../editor/src/recording.ts";
import { measureAudioFile, wavLengthMs } from "../editor/src/audioMeasure.ts";
import { HOLD_MAX_MS, HOLD_MIN_MS, isLoading, mayPlay, type MediaState } from "../editor/src/mediaGate.ts";

const chunk = (id: string, body: Uint8Array): Uint8Array => {
  const out = new Uint8Array(8 + body.length + (body.length % 2));
  out.set([...id].map((c) => c.charCodeAt(0)));
  new DataView(out.buffer).setUint32(4, body.length, true);
  out.set(body, 8);
  return out;
};
const join = (...parts: Uint8Array[]) => {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let at = 0;
  for (const p of parts) (out.set(p, at), (at += p.length));
  return out;
};

test("a WAV's length is read from its header", () => {
  const wav = encodeWav(new Float32Array(48000 * 2.5));
  assert.equal(wavLengthMs(wav), 2500);
  assert.equal(wavLengthMs(wav.subarray(0, 44)), 2500, "the header alone is enough");
  assert.equal(wavLengthMs(wav.subarray(0, 30)), "more");
  assert.equal(wavLengthMs(new Uint8Array(4)), "more");
  assert.equal(wavLengthMs(new TextEncoder().encode("ID3 not a wave file at all")), null);
});

test("a WAV with other chunks before its data, or a streamed one without a data size", () => {
  const wav = encodeWav(new Float32Array(24000));
  const riff = wav.subarray(0, 12);
  const fmt = wav.subarray(12, 36);
  const data = wav.subarray(36);
  const list = chunk("LIST", new TextEncoder().encode("INFOISFT odd")); // odd size: padded
  assert.equal(wavLengthMs(join(riff, list, fmt, data)), 500);
  const streamed = join(riff, fmt, data);
  new DataView(streamed.buffer).setUint32(40, 0xffffffff, true);
  assert.equal(wavLengthMs(streamed, streamed.length), 500);
  assert.equal(wavLengthMs(streamed), null, "no total, no length");
  const noFmt = join(riff, data);
  assert.equal(wavLengthMs(noFmt), null);
});

const streamed = (bytes: Uint8Array, size = 16, seen = { pulled: 0, cancelled: false }) => {
  let at = 0;
  const body = new ReadableStream<Uint8Array>({
    pull(c) {
      if (at >= bytes.length) return c.close();
      seen.pulled += 1;
      c.enqueue(bytes.slice(at, at + size));
      at += size;
    },
    cancel() {
      seen.cancelled = true;
    },
  });
  return (async () => new Response(body, { headers: { "content-length": String(bytes.length) } })) as unknown as typeof fetch;
};

test("measuring a WAV reads only its first bytes, with no media element", async () => {
  const wav = encodeWav(new Float32Array(48000 * 3));
  const seen = { pulled: 0, cancelled: false };
  assert.equal(await measureAudioFile("asset://x.wav", streamed(wav, 16, seen)), 3000);
  assert.ok(seen.cancelled, "the rest of the file is not read");
  assert.ok(seen.pulled <= 4, `read ${seen.pulled} chunks`);
});

test("a file that is missing or not audio has no length", async () => {
  assert.equal(await measureAudioFile("x", (async () => new Response("no", { status: 404 })) as unknown as typeof fetch), null);
  assert.equal(await measureAudioFile("x", (async () => { throw new Error("offline"); }) as unknown as typeof fetch), null);
  // Not a WAV: decoded with an OfflineAudioContext, which node does not have.
  assert.equal(await measureAudioFile("x", streamed(new TextEncoder().encode("plain text, not audio"))), null);
});

const media = (p: Partial<MediaState>): MediaState => ({ src: "asset://a.wav", networkState: 1, readyState: 4, error: null, ...p });

test("a media element is loading while it fetches a file it has no metadata for", () => {
  assert.equal(isLoading(media({ networkState: 2, readyState: 0 })), true);
  assert.equal(isLoading(media({ networkState: 2, readyState: 1 })), false, "metadata is there");
  assert.equal(isLoading(media({ networkState: 2, readyState: 0, error: { code: 4 } })), false, "failed");
  assert.equal(isLoading(media({ src: "", networkState: 2, readyState: 0 })), false, "no file");
  assert.equal(isLoading(media({ networkState: 1, readyState: 0 })), false, "idle (preload none)");
});

test("play is held after a take is kept until the files have loaded, never longer than the cap", () => {
  const loading = [media({}), media({ networkState: 2, readyState: 0 })];
  const loaded = [media({}), media({ readyState: 1 })];
  assert.equal(mayPlay(1000, null, loading), true, "no hold: play at once, as before");
  assert.equal(mayPlay(1000 + HOLD_MIN_MS - 1, 1000, loaded), false, "the Player may not have the new file yet");
  assert.equal(mayPlay(1000 + HOLD_MIN_MS, 1000, loaded), true);
  assert.equal(mayPlay(1000 + HOLD_MIN_MS, 1000, loading), false);
  assert.equal(mayPlay(1000 + HOLD_MAX_MS, 1000, loading), true, "a file that never loads does not block play");
});
