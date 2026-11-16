// 提示词管理 (prompt management): assistant instructions the user sends often, kept so they can
// be reused with one click. Pure logic (no React, no DOM): add / rename / edit / delete /
// reorder, optional category, search and filter, the {场景} placeholder, storage (through a
// Storage-like object) and strict import validation.
//
// The list is global for the user (not per project): it lives in localStorage under a
// versioned key. Export / import move it between machines as a small JSON file. A saved prompt
// is only text: it never calls the API by itself and nothing in it is ever executed.

import type { ScopeMode } from "./scope";

// Type-only import: the editor's node tests load this file without a bundler.
const isScopeMode = (v: unknown): v is ScopeMode => v === "all" || v === "selected" || v === "one";

export type SavedPrompt = {
  id: string;
  /** Short name shown in the list. */
  name: string;
  /** The instruction, possibly with {场景} / {scene}. */
  text: string;
  /** Default scope set when the prompt is used (全部 / 选几幕 / 当前这一幕). Absent = leave the scope as it is. */
  scope?: ScopeMode;
  /** Optional free-text category / tag, e.g. 「拆分」「节奏」「调色」. Absent = none. */
  category?: string;
};

/** A prompt without its id (what is exported and imported). */
export type PromptData = Omit<SavedPrompt, "id">;

export const STORAGE_KEY = "vimmer.prompts.v1";
export const EXPORT_FORMAT = "vimmer-prompts";
export const EXPORT_VERSION = 1;
export const MAX_PROMPTS = 50;
export const MAX_NAME = 40;
export const MAX_TEXT = 2000;
export const MAX_CATEGORY = 20;
/** Characters of the text used as the default name. */
export const DEFAULT_NAME_CHARS = 12;
/** Largest import file accepted (characters). */
export const MAX_IMPORT_CHARS = 200_000;

/** Starters shown the first time (normal entries: the user can edit or delete them). */
export const STARTERS: ReadonlyArray<PromptData> = [
  { name: "按句子拆幕", text: "把{场景}按句子拆成几幕，每幕一句，用同一张图", scope: "one", category: "拆分" },
  { name: "节奏快一点", text: "{场景}节奏快一点", scope: "one", category: "节奏" },
  { name: "全部淡入转场", text: "给所有场景加淡入转场", scope: "all", category: "转场" },
  { name: "温暖色调", text: "整体调成温暖色调", scope: "all", category: "调色" },
];

export const starterPrompts = (): SavedPrompt[] => STARTERS.map((s, i) => ({ ...s, id: `starter-${i + 1}` }));

let counter = 0;
/** A new id (unique within a list: time + counter + random). */
export const newPromptId = (): string =>
  `p-${Date.now().toString(36)}-${(counter++).toString(36)}-${Math.random().toString(36).slice(2, 6)}`;

const oneLine = (s: string): string => s.trim().replace(/\s+/g, " ");
const firstChars = (s: string, n: number): string => Array.from(s).slice(0, n).join("");

/** The name used when none is given: the first ~12 characters of the text. */
export const defaultName = (text: string): string => {
  const chars = Array.from(oneLine(text));
  return chars.length <= DEFAULT_NAME_CHARS ? chars.join("") : `${chars.slice(0, DEFAULT_NAME_CHARS).join("")}…`;
};

const cleanName = (name: string | undefined, text: string): string => firstChars(oneLine(name ?? "") || defaultName(text), MAX_NAME);

/** A category as stored: one line, at most 20 characters; empty = none (undefined). */
export const cleanCategory = (category: string | undefined): string | undefined => {
  const c = firstChars(oneLine(category ?? ""), MAX_CATEGORY);
  return c || undefined;
};

/** Sets or removes the optional fields (never stored as empty / undefined keys). */
const withOptional = <T extends object>(p: T, category: string | undefined, scope: ScopeMode | undefined): T & { category?: string; scope?: ScopeMode } => {
  const { category: _c, scope: _s, ...rest } = p as T & { category?: string; scope?: ScopeMode };
  return { ...(rest as T), ...(category ? { category } : {}), ...(scope ? { scope } : {}) };
};

export type ListResult = { ok: true; list: SavedPrompt[] } | { ok: false; error: string };

const EMPTY = "提示词是空的 The prompt is empty.";
const TOO_LONG = `提示词太长（最多 ${MAX_TEXT} 字）Too long.`;

export const addPrompt = (
  list: SavedPrompt[],
  input: { name?: string; text: string; scope?: ScopeMode; category?: string },
  id: string = newPromptId(),
): ListResult => {
  const text = input.text.trim();
  if (!text) return { ok: false, error: EMPTY };
  if (text.length > MAX_TEXT) return { ok: false, error: TOO_LONG };
  if (list.length >= MAX_PROMPTS) return { ok: false, error: `最多保存 ${MAX_PROMPTS} 条 At most ${MAX_PROMPTS} prompts.` };
  const added = withOptional({ id, name: cleanName(input.name, text), text }, cleanCategory(input.category), isScopeMode(input.scope) ? input.scope : undefined);
  return { ok: true, list: [...list, added] };
};

/**
 * Changes name, text, scope and/or category of one prompt. An empty name falls back to the
 * default; an empty category removes it; scope `null` removes it.
 */
export const updatePrompt = (
  list: SavedPrompt[],
  id: string,
  change: { name?: string; text?: string; scope?: ScopeMode | null; category?: string },
): ListResult => {
  const old = list.find((c) => c.id === id);
  if (!old) return { ok: false, error: "Not found" };
  const text = change.text === undefined ? old.text : change.text.trim();
  if (!text) return { ok: false, error: EMPTY };
  if (text.length > MAX_TEXT) return { ok: false, error: TOO_LONG };
  const name = change.name === undefined ? old.name : cleanName(change.name, text);
  const scope = change.scope === undefined ? old.scope : change.scope === null ? undefined : change.scope;
  const category = change.category === undefined ? old.category : cleanCategory(change.category);
  return { ok: true, list: list.map((c) => (c.id === id ? withOptional({ ...c, name, text }, category, scope) : c)) };
};

export const renamePrompt = (list: SavedPrompt[], id: string, name: string): ListResult => updatePrompt(list, id, { name });
export const editPromptText = (list: SavedPrompt[], id: string, text: string): ListResult => updatePrompt(list, id, { text });

export type Removed = { prompt: SavedPrompt; index: number };

/** Removes a prompt; `removed` is what `restorePrompt` needs to undo it. */
export const deletePrompt = (list: SavedPrompt[], id: string): { list: SavedPrompt[]; removed: Removed | null } => {
  const index = list.findIndex((c) => c.id === id);
  if (index < 0) return { list, removed: null };
  return { list: list.filter((c) => c.id !== id), removed: { prompt: list[index], index } };
};

/** Undo of a delete: puts the prompt back where it was (no-op if it is back already or the list is full). */
export const restorePrompt = (list: SavedPrompt[], removed: Removed): SavedPrompt[] => {
  if (list.some((c) => c.id === removed.prompt.id) || list.length >= MAX_PROMPTS) return list;
  const at = Math.min(Math.max(removed.index, 0), list.length);
  return [...list.slice(0, at), removed.prompt, ...list.slice(at)];
};

/** Moves a prompt one place up (-1) or down (+1) in the whole list. */
export const movePrompt = (list: SavedPrompt[], id: string, by: -1 | 1): SavedPrompt[] => {
  const i = list.findIndex((c) => c.id === id);
  const j = i + by;
  if (i < 0 || j < 0 || j >= list.length) return list;
  const out = [...list];
  [out[i], out[j]] = [out[j], out[i]];
  return out;
};

// ---- Search and categories -------------------------------------------------------------------

/** The categories in use, in list order, each once. */
export const categoriesOf = (list: SavedPrompt[]): string[] => [...new Set(list.flatMap((p) => (p.category ? [p.category] : [])))];

/** Filter value meaning "prompts without a category". */
export const NO_CATEGORY = "\u0000none";

/**
 * Prompts whose name, text or category contain every word of `query` (case-insensitive), and,
 * when `category` is given, that have that category (`NO_CATEGORY` = those without one). Order kept.
 */
export const filterPrompts = (list: SavedPrompt[], query: string, category: string | null = null): SavedPrompt[] => {
  const words = oneLine(query).toLowerCase().split(" ").filter(Boolean);
  return list.filter((p) => {
    if (category === NO_CATEGORY ? !!p.category : category !== null && p.category !== category) return false;
    const hay = `${p.name}\n${p.text}\n${p.category ?? ""}`.toLowerCase();
    return words.every((w) => hay.includes(w));
  });
};

// ---- Storage ---------------------------------------------------------------------------------

type StorageLike = Pick<Storage, "getItem" | "setItem">;

/**
 * Checks one entry strictly; null if anything is wrong. `category` and `scope` are optional.
 * An older entry's `useScene` (true = about the selected scene) becomes scope "one" / "all".
 */
const validEntry = (v: unknown): PromptData | null => {
  if (typeof v !== "object" || v === null || Array.isArray(v)) return null;
  const o = v as Record<string, unknown>;
  if (typeof o.name !== "string" || typeof o.text !== "string") return null;
  if (o.category !== undefined && typeof o.category !== "string") return null;
  if (o.scope !== undefined && !isScopeMode(o.scope)) return null;
  if (o.useScene !== undefined && typeof o.useScene !== "boolean") return null;
  const scope: ScopeMode | undefined = isScopeMode(o.scope) ? o.scope : o.useScene === undefined ? undefined : o.useScene ? "one" : "all";
  const text = o.text.trim();
  const name = oneLine(o.name);
  const category = typeof o.category === "string" ? oneLine(o.category) : "";
  if (!text || text.length > MAX_TEXT || name.length > MAX_NAME || category.length > MAX_CATEGORY) return null;
  return withOptional({ name: name || defaultName(text), text }, category || undefined, scope);
};

const validId = (v: unknown): v is string => typeof v === "string" && /^[A-Za-z0-9_-]{1,64}$/.test(v);

/**
 * The saved list, or null when nothing was ever saved (then the starters are shown).
 * A broken or unreadable value is treated as nothing saved.
 */
export const loadPrompts = (storage: StorageLike | null | undefined): SavedPrompt[] | null => {
  try {
    const raw = storage?.getItem(STORAGE_KEY);
    if (raw == null) return null;
    const data = JSON.parse(raw) as unknown;
    if (!Array.isArray(data)) return null;
    const out: SavedPrompt[] = [];
    const seen = new Set<string>();
    for (const item of data.slice(0, MAX_PROMPTS)) {
      const entry = validEntry(item);
      if (!entry) continue;
      const rawId = (item as { id?: unknown }).id;
      const id = validId(rawId) && !seen.has(rawId) ? rawId : newPromptId();
      seen.add(id);
      out.push({ id, ...entry });
    }
    return out;
  } catch {
    return null;
  }
};

/** Writes the list; false if storage is unavailable (private window, quota, …). */
export const savePrompts = (storage: StorageLike | null | undefined, list: SavedPrompt[]): boolean => {
  try {
    if (!storage) return false;
    storage.setItem(STORAGE_KEY, JSON.stringify(list));
    return true;
  } catch {
    return false;
  }
};

// ---- Export / import -------------------------------------------------------------------------

export const exportJson = (list: SavedPrompt[]): string =>
  JSON.stringify(
    {
      format: EXPORT_FORMAT,
      version: EXPORT_VERSION,
      prompts: list.map(({ name, text, category, scope }) => withOptional({ name, text }, category, scope)),
    },
    null,
    2,
  );

export type ParsedImport = { ok: true; prompts: PromptData[] } | { ok: false; error: string };

/**
 * Reads an exported file. Strict: the format marker, version, and every entry's types and
 * lengths must be right, or the whole file is refused (nothing half-imported).
 */
export const parseImport = (raw: string): ParsedImport => {
  const bad = (why: string): ParsedImport => ({ ok: false, error: `无法导入 Cannot import: ${why}` });
  if (raw.length > MAX_IMPORT_CHARS) return bad("file too large");
  let data: unknown;
  try {
    data = JSON.parse(raw);
  } catch {
    return bad("not JSON");
  }
  if (typeof data !== "object" || data === null || Array.isArray(data)) return bad("not a prompts file");
  const o = data as Record<string, unknown>;
  if (o.format !== EXPORT_FORMAT) return bad("not a prompts file");
  if (o.version !== EXPORT_VERSION) return bad(`unsupported version ${String(o.version)}`);
  if (!Array.isArray(o.prompts)) return bad("no prompt list");
  if (o.prompts.length > MAX_PROMPTS) return bad(`more than ${MAX_PROMPTS} prompts`);
  const prompts: PromptData[] = [];
  for (let i = 0; i < o.prompts.length; i += 1) {
    const entry = validEntry(o.prompts[i]);
    if (!entry) return bad(`entry ${i + 1} is invalid`);
    prompts.push(entry);
  }
  return { ok: true, prompts };
};

/**
 * Adds imported prompts after the existing ones. Same text as an existing prompt = skipped;
 * stops at the maximum. Returns how many were added and skipped.
 */
export const mergeImport = (
  list: SavedPrompt[],
  prompts: PromptData[],
  makeId: () => string = newPromptId,
): { list: SavedPrompt[]; added: number; skipped: number } => {
  const out = [...list];
  const texts = new Set(list.map((c) => c.text));
  let added = 0;
  for (const c of prompts) {
    if (texts.has(c.text) || out.length >= MAX_PROMPTS) continue;
    texts.add(c.text);
    out.push({ ...c, id: makeId() });
    added += 1;
  }
  return { list: out, added, skipped: prompts.length - added };
};
