// Saved assistant commands (常用指令): instructions the user sends often, kept so they can be
// reused with one click. Pure logic (no React, no DOM): add / rename / edit / delete / reorder,
// the {场景} placeholder, storage (through a Storage-like object) and strict import validation.
//
// The list is global for the user (not per project): it lives in localStorage under a
// versioned key. Export / import move it between machines as a small JSON file. A saved command
// is only text: it never calls the API by itself and nothing in it is ever executed.

export type SavedCommand = {
  id: string;
  /** Short name shown in the list. */
  name: string;
  /** The instruction, possibly with {场景} / {scene}. */
  text: string;
  /** Whether "about the selected scene" was ticked when it was saved. */
  useScene: boolean;
};

export const STORAGE_KEY = "vimmer.savedCommands.v1";
export const EXPORT_FORMAT = "vimmer-saved-commands";
export const EXPORT_VERSION = 1;
export const MAX_COMMANDS = 50;
export const MAX_NAME = 40;
export const MAX_TEXT = 2000;
/** Characters of the text used as the default name. */
export const DEFAULT_NAME_CHARS = 12;
/** Largest import file accepted (characters). */
export const MAX_IMPORT_CHARS = 200_000;

/** Starters shown the first time (normal entries: the user can edit or delete them). */
export const STARTERS: ReadonlyArray<Omit<SavedCommand, "id">> = [
  { name: "按句子拆幕", text: "把{场景}按句子拆成几幕，每幕一句，用同一张图", useScene: true },
  { name: "节奏快一点", text: "{场景}节奏快一点", useScene: true },
  { name: "全部淡入转场", text: "给所有场景加淡入转场", useScene: false },
  { name: "温暖色调", text: "整体调成温暖色调", useScene: false },
];

export const starterCommands = (): SavedCommand[] => STARTERS.map((s, i) => ({ ...s, id: `starter-${i + 1}` }));

let counter = 0;
/** A new id (unique within a list: time + counter + random). */
export const newCommandId = (): string =>
  `cmd-${Date.now().toString(36)}-${(counter++).toString(36)}-${Math.random().toString(36).slice(2, 6)}`;

const oneLine = (s: string): string => s.trim().replace(/\s+/g, " ");

/** The name used when none is given: the first ~12 characters of the text. */
export const defaultName = (text: string): string => {
  const chars = Array.from(oneLine(text));
  return chars.length <= DEFAULT_NAME_CHARS ? chars.join("") : `${chars.slice(0, DEFAULT_NAME_CHARS).join("")}…`;
};

const cleanName = (name: string | undefined, text: string): string => {
  const n = oneLine(name ?? "");
  return Array.from(n || defaultName(text)).slice(0, MAX_NAME).join("");
};

export type ListResult = { ok: true; list: SavedCommand[] } | { ok: false; error: string };

export const addCommand = (
  list: SavedCommand[],
  input: { name?: string; text: string; useScene: boolean },
  id: string = newCommandId(),
): ListResult => {
  const text = input.text.trim();
  if (!text) return { ok: false, error: "指令是空的 The instruction is empty." };
  if (text.length > MAX_TEXT) return { ok: false, error: `指令太长（最多 ${MAX_TEXT} 字）Too long.` };
  if (list.length >= MAX_COMMANDS) return { ok: false, error: `最多保存 ${MAX_COMMANDS} 条 At most ${MAX_COMMANDS} commands.` };
  return { ok: true, list: [...list, { id, name: cleanName(input.name, text), text, useScene: !!input.useScene }] };
};

/** Changes name, text and/or the scene flag of one command. An empty name falls back to the default. */
export const updateCommand = (
  list: SavedCommand[],
  id: string,
  change: { name?: string; text?: string; useScene?: boolean },
): ListResult => {
  const old = list.find((c) => c.id === id);
  if (!old) return { ok: false, error: "Not found" };
  const text = change.text === undefined ? old.text : change.text.trim();
  if (!text) return { ok: false, error: "指令是空的 The instruction is empty." };
  if (text.length > MAX_TEXT) return { ok: false, error: `指令太长（最多 ${MAX_TEXT} 字）Too long.` };
  const name = change.name === undefined ? old.name : cleanName(change.name, text);
  const useScene = change.useScene === undefined ? old.useScene : change.useScene;
  return { ok: true, list: list.map((c) => (c.id === id ? { ...c, name, text, useScene } : c)) };
};

export const renameCommand = (list: SavedCommand[], id: string, name: string): ListResult => updateCommand(list, id, { name });
export const editCommandText = (list: SavedCommand[], id: string, text: string): ListResult => updateCommand(list, id, { text });

export type Removed = { command: SavedCommand; index: number };

/** Removes a command; `removed` is what `restoreCommand` needs to undo it. */
export const deleteCommand = (list: SavedCommand[], id: string): { list: SavedCommand[]; removed: Removed | null } => {
  const index = list.findIndex((c) => c.id === id);
  if (index < 0) return { list, removed: null };
  return { list: list.filter((c) => c.id !== id), removed: { command: list[index], index } };
};

/** Undo of a delete: puts the command back where it was (no-op if it is back already or the list is full). */
export const restoreCommand = (list: SavedCommand[], removed: Removed): SavedCommand[] => {
  if (list.some((c) => c.id === removed.command.id) || list.length >= MAX_COMMANDS) return list;
  const at = Math.min(Math.max(removed.index, 0), list.length);
  return [...list.slice(0, at), removed.command, ...list.slice(at)];
};

/** Moves a command one place up (-1) or down (+1). */
export const moveCommand = (list: SavedCommand[], id: string, by: -1 | 1): SavedCommand[] => {
  const i = list.findIndex((c) => c.id === id);
  const j = i + by;
  if (i < 0 || j < 0 || j >= list.length) return list;
  const out = [...list];
  [out[i], out[j]] = [out[j], out[i]];
  return out;
};

// ---- The scene placeholder -----------------------------------------------------------------

const PLACEHOLDER = /\{\s*(?:scene|场景)\s*\}/gi;

export const needsScene = (text: string): boolean => new RegExp(PLACEHOLDER.source, "i").test(text);

/** How a scene is named in an instruction: 「第 3 幕 (scene-03)」 (number as on the timeline, plus the id). */
export const sceneLabel = (index: number, id: string): string => `第 ${index + 1} 幕 (${id})`;

export type SceneRef = { index: number; id: string };

export type Filled = { ok: true; text: string } | { ok: false; reason: "needsScene" };

/** Replaces {场景} / {scene} with the selected scene's label. Without a scene, a text that needs one is refused. */
export const fillPlaceholders = (text: string, scene: SceneRef | null): Filled => {
  if (!needsScene(text)) return { ok: true, text };
  if (!scene) return { ok: false, reason: "needsScene" };
  return { ok: true, text: text.replace(PLACEHOLDER, sceneLabel(scene.index, scene.id)) };
};

/**
 * The reverse, for saving a sent message: scene labels made by `sceneLabel` («第 3 幕 (scene-03)»)
 * become {场景} again, so the saved command works for any scene. Other text is left alone.
 */
export const generalizeScene = (text: string): string => text.replace(/第 \d+ 幕 \([A-Za-z0-9_-]{1,64}\)/g, "{场景}");

export const NEEDS_SCENE_HINT = "这条指令需要一个场景：先在时间线上选中一幕 · Select a scene first ({场景} needs one).";

// ---- Storage ---------------------------------------------------------------------------------

type StorageLike = Pick<Storage, "getItem" | "setItem">;

/** Checks one entry strictly; null if anything is wrong. */
const validEntry = (v: unknown): Omit<SavedCommand, "id"> | null => {
  if (typeof v !== "object" || v === null || Array.isArray(v)) return null;
  const o = v as Record<string, unknown>;
  if (typeof o.name !== "string" || typeof o.text !== "string" || typeof o.useScene !== "boolean") return null;
  const text = o.text.trim();
  const name = oneLine(o.name);
  if (!text || text.length > MAX_TEXT || name.length > MAX_NAME) return null;
  return { name: name || defaultName(text), text, useScene: o.useScene };
};

const validId = (v: unknown): v is string => typeof v === "string" && /^[A-Za-z0-9_-]{1,64}$/.test(v);

/**
 * The saved list, or null when nothing was ever saved (then the starters are shown).
 * A broken or unreadable value is treated as nothing saved.
 */
export const loadCommands = (storage: StorageLike | null | undefined): SavedCommand[] | null => {
  try {
    const raw = storage?.getItem(STORAGE_KEY);
    if (raw == null) return null;
    const data = JSON.parse(raw) as unknown;
    if (!Array.isArray(data)) return null;
    const out: SavedCommand[] = [];
    const seen = new Set<string>();
    for (const item of data.slice(0, MAX_COMMANDS)) {
      const entry = validEntry(item);
      if (!entry) continue;
      const rawId = (item as { id?: unknown }).id;
      const id = validId(rawId) && !seen.has(rawId) ? rawId : newCommandId();
      seen.add(id);
      out.push({ id, ...entry });
    }
    return out;
  } catch {
    return null;
  }
};

/** Writes the list; false if storage is unavailable (private window, quota, …). */
export const saveCommands = (storage: StorageLike | null | undefined, list: SavedCommand[]): boolean => {
  try {
    if (!storage) return false;
    storage.setItem(STORAGE_KEY, JSON.stringify(list));
    return true;
  } catch {
    return false;
  }
};

// ---- Export / import -------------------------------------------------------------------------

export const exportJson = (list: SavedCommand[]): string =>
  JSON.stringify(
    { format: EXPORT_FORMAT, version: EXPORT_VERSION, commands: list.map(({ name, text, useScene }) => ({ name, text, useScene })) },
    null,
    2,
  );

export type ParsedImport = { ok: true; commands: Omit<SavedCommand, "id">[] } | { ok: false; error: string };

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
  if (typeof data !== "object" || data === null || Array.isArray(data)) return bad("not a saved-commands file");
  const o = data as Record<string, unknown>;
  if (o.format !== EXPORT_FORMAT) return bad("not a saved-commands file");
  if (o.version !== EXPORT_VERSION) return bad(`unsupported version ${String(o.version)}`);
  if (!Array.isArray(o.commands)) return bad("no command list");
  if (o.commands.length > MAX_COMMANDS) return bad(`more than ${MAX_COMMANDS} commands`);
  const commands: Omit<SavedCommand, "id">[] = [];
  for (let i = 0; i < o.commands.length; i += 1) {
    const entry = validEntry(o.commands[i]);
    if (!entry) return bad(`entry ${i + 1} is invalid`);
    commands.push(entry);
  }
  return { ok: true, commands };
};

/**
 * Adds imported commands after the existing ones. Same text as an existing command = skipped;
 * stops at the maximum. Returns how many were added and skipped.
 */
export const mergeImport = (
  list: SavedCommand[],
  commands: Omit<SavedCommand, "id">[],
  makeId: () => string = newCommandId,
): { list: SavedCommand[]; added: number; skipped: number } => {
  const out = [...list];
  const texts = new Set(list.map((c) => c.text));
  let added = 0;
  for (const c of commands) {
    if (texts.has(c.text) || out.length >= MAX_COMMANDS) continue;
    texts.add(c.text);
    out.push({ ...c, id: makeId() });
    added += 1;
  }
  return { list: out, added, skipped: commands.length - added };
};
