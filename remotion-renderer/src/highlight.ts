// Keyword highlights inside a word, and explicit line breaks, for the scene text (AnimatedText.tsx).
// Pure, no runtime imports: the Node tests import it (test/highlight.test.ts).
//
// The scene text is split into words at whitespace; Chinese or Japanese without spaces is one
// "word" per run, so a highlight such as 文字 in 输入文字就能 never matched a whole word. A word
// that matches a highlight as a whole keeps the old treatment (the whole word glows). Otherwise a
// highlight may match a part of the word when CJK is involved: the highlight has a wide character,
// or each side of the match is the word's edge or a wide character (so "AI" lights up in 用AI生成,
// "cat" never does in "concatenate"). Words without a wide character never match a part, so Latin
// text renders exactly as before. Overlapping matches merge into one highlighted run.

// CJK ideographs, kana, hangul, CJK punctuation and full-width forms (as `isWide` in
// textPages.ts, which this module cannot import: the Node tests load it without a bundler).
const WIDE_RE = /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}\u3000-\u303f\uff00-\uff60\uffe0-\uffe6]/u;
const isWide = (c: string): boolean => WIDE_RE.test(c);

/** A line break in the word list (the scene text draws it as `<br>`). */
export const LINE_BREAK = "\n";

/** Words of `text` split at whitespace, with LINE_BREAK between lines typed with a line break
 * (a run of breaks counts once; breaks before the first or after the last word are dropped). */
export const splitWordsAndBreaks = (text: string): string[] => {
  const out: string[] = [];
  for (const line of text.split("\n")) {
    const words = line.split(/\s+/).filter(Boolean);
    if (!words.length) continue;
    if (out.length) out.push(LINE_BREAK);
    out.push(...words);
  }
  return out;
};

export type WordPart = { text: string; highlight: boolean };

const chars = (s: string): string[] => Array.from(s);

/** The parts of `word` covered by `highlights` (see the module comment); one unhighlighted part
 * when nothing matches inside it. Matching ignores case. */
export const highlightParts = (word: string, highlights: readonly string[]): WordPart[] => {
  const w = chars(word);
  const lower = w.map((c) => c.toLowerCase());
  const hasWide = w.some(isWide);
  const covered = new Array<boolean>(w.length).fill(false);
  if (hasWide) {
    for (const raw of highlights) {
      const h = chars(raw.trim()).map((c) => c.toLowerCase());
      if (!h.length || h.length > w.length) continue;
      const wideKey = h.some(isWide);
      for (let i = 0; i + h.length <= w.length; i++) {
        if (!h.every((c, k) => lower[i + k] === c)) continue;
        const end = i + h.length;
        const leftOk = i === 0 || isWide(w[i - 1]) || isWide(w[i]);
        const rightOk = end === w.length || isWide(w[end]) || isWide(w[end - 1]);
        if (wideKey || (leftOk && rightOk)) for (let k = i; k < end; k++) covered[k] = true;
      }
    }
  }
  const out: WordPart[] = [];
  w.forEach((c, i) => {
    const last = out[out.length - 1];
    if (last && last.highlight === covered[i]) last.text += c;
    else out.push({ text: c, highlight: covered[i] });
  });
  return out.length ? out : [{ text: word, highlight: false }];
};
