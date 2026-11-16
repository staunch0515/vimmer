import { useEffect, useId, useRef, useState, type Dispatch, type SetStateAction } from "react";
import {
  MAX_CATEGORY, MAX_IMPORT_CHARS, MAX_NAME, MAX_PROMPTS, MAX_TEXT, NO_CATEGORY, STORAGE_KEY,
  addPrompt, categoriesOf, defaultName, deletePrompt, exportJson, filterPrompts, generalizeScene, loadPrompts,
  mergeImport, movePrompt, needsScene, parseImport, restorePrompt, savePrompts, starterPrompts, updatePrompt,
  type Removed, type SavedPrompt,
} from "../savedPrompts";

/** localStorage, or null where reading it throws (private mode, blocked storage). */
const storage = (): Storage | null => {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
};

/**
 * The user's saved prompts, kept in localStorage (global, not per project). The starters are
 * shown until something is saved. Other windows of the editor pick up changes.
 */
export const useSavedPrompts = (): [SavedPrompt[], Dispatch<SetStateAction<SavedPrompt[]>>] => {
  const [list, setList] = useState<SavedPrompt[]>(() => loadPrompts(storage()) ?? starterPrompts());
  const changed = useRef(false);
  const set: Dispatch<SetStateAction<SavedPrompt[]>> = (v) => {
    changed.current = true;
    setList(v);
  };
  useEffect(() => {
    if (changed.current) savePrompts(storage(), list);
  }, [list]);
  useEffect(() => {
    const onStorage = (e: StorageEvent) => {
      if (e.key !== STORAGE_KEY) return;
      changed.current = false;
      setList(loadPrompts(storage()) ?? starterPrompts());
    };
    window.addEventListener("storage", onStorage);
    return () => window.removeEventListener("storage", onStorage);
  }, []);
  return [list, set];
};

/** What the "save" form starts from (a sent message, or the text in the input). */
export type SaveDraft = { text: string; useScene: boolean };

type Props = {
  prompts: SavedPrompt[];
  setPrompts: Dispatch<SetStateAction<SavedPrompt[]>>;
  /** Fill the input with this prompt. */
  onUse: (p: SavedPrompt) => void;
  /** Send this prompt right away. */
  onSend: (p: SavedPrompt) => void;
  busy: boolean;
  draft: SaveDraft | null;
  onDraftDone: () => void;
};

type Fields = { name: string; text: string; useScene: boolean; category: string };
/** Returns an error message, or null when saved. */
type Saver = (input: Fields) => string | null;

/**
 * 提示词 Prompts: a collapsible picker above the message box (click = put in the box, ▶ or
 * Shift+click = send now) and 提示词管理, the manager: search, category filter, new, rename /
 * edit, delete (with undo), reorder, export / import.
 */
export const SavedPrompts = ({ prompts, setPrompts, onUse, onSend, busy, draft, onDraftDone }: Props) => {
  const [open, setOpen] = useState(false);
  const [manage, setManage] = useState(false);
  const [undo, setUndo] = useState<Removed | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const categories = categoriesOf(prompts);

  useEffect(() => {
    if (!undo) return;
    const t = window.setTimeout(() => setUndo(null), 10_000);
    return () => window.clearTimeout(t);
  }, [undo]);

  const remove = (id: string) => {
    const r = deletePrompt(prompts, id);
    if (!r.removed) return;
    setPrompts(r.list);
    setUndo(r.removed);
  };

  return (
    <div className="saved-commands">
      {draft && (
        <PromptForm
          key={draft.text}
          title="☆ 保存为提示词 Save as prompt"
          initial={{ name: "", text: draft.text, useScene: draft.useScene, category: "" }}
          categories={categories}
          full={prompts.length >= MAX_PROMPTS}
          offerGeneralize
          onCancel={onDraftDone}
          onSave={(input) => {
            const r = addPrompt(prompts, input);
            if (!r.ok) return r.error;
            setPrompts(r.list);
            setOpen(true);
            setNotice(`已保存「${r.list[r.list.length - 1].name}」 · Saved`);
            onDraftDone();
            return null;
          }}
        />
      )}
      <div className="assistant-row saved-head">
        <button className="btn ghost small" onClick={() => { setOpen(!open); setManage(false); }} aria-expanded={open && !manage} title="Saved prompts you can reuse">
          提示词 {open && !manage ? "▴" : "▾"} <span className="dim">{prompts.length}</span>
        </button>
        <button className={`btn ghost small ${manage ? "active" : ""}`} onClick={() => setManage(!manage)} aria-pressed={manage}>
          {manage ? "完成 Done" : "提示词管理"}
        </button>
      </div>

      {manage ? (
        <PromptManager prompts={prompts} setPrompts={setPrompts} categories={categories} onRemove={remove} setNotice={setNotice} onClose={() => setManage(false)} />
      ) : (
        open && (
          <div className="saved-list" role="list" aria-label="提示词 Prompts">
            {prompts.length === 0 && (
              <div className="dim small">
                还没有提示词。在对话里点「☆ 收藏」或输入框下的「保存为提示词」。
                <button className="link" onClick={() => setPrompts(starterPrompts())}>恢复示例 Restore examples</button>
              </div>
            )}
            {prompts.map((p) => (
              <div key={p.id} className="saved-item" role="listitem">
                <button
                  className="saved-use"
                  onClick={(e) => (e.shiftKey && !busy ? onSend(p) : onUse(p))}
                  title={`${p.text}${p.useScene ? "\n(about the selected scene 关于所选场景)" : ""}\n\nClick: put in the box · Shift+click: send now`}
                >
                  <span className="saved-name">{p.name}</span>
                  {p.category && <span className="saved-cat">{p.category}</span>}
                  {needsScene(p.text) && <span className="saved-tag">场景</span>}
                </button>
                <button className="btn ghost small" onClick={() => onSend(p)} disabled={busy} aria-label={`Send ${p.name} now`} title="直接发送 Send now (a paid API call)">▶</button>
              </div>
            ))}
          </div>
        )
      )}

      {undo && (
        <div className="saved-toast" role="status">
          已删除「{undo.prompt.name}」
          <button className="link" onClick={() => { setPrompts((l) => restorePrompt(l, undo)); setUndo(null); }}>撤销 Undo</button>
        </div>
      )}
      {notice && (
        <div className="dim small saved-notice" role="status">
          {notice} <button className="link" onClick={() => setNotice(null)} aria-label="Dismiss">✕</button>
        </div>
      )}
    </div>
  );
};

type ManagerProps = {
  prompts: SavedPrompt[];
  setPrompts: Dispatch<SetStateAction<SavedPrompt[]>>;
  categories: string[];
  onRemove: (id: string) => void;
  setNotice: (s: string | null) => void;
  onClose: () => void;
};

/** 提示词管理: search, category filter, new / edit / delete / move, export / import. */
const PromptManager = ({ prompts, setPrompts, categories, onRemove, setNotice, onClose }: ManagerProps) => {
  const [query, setQuery] = useState("");
  const [category, setCategory] = useState<string | null>(null);
  const [editing, setEditing] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const fileInput = useRef<HTMLInputElement>(null);
  // A category that no longer exists (renamed / deleted) stops filtering.
  const activeCategory = category === null || category === NO_CATEGORY || categories.includes(category) ? category : null;
  const shown = filterPrompts(prompts, query, activeCategory);
  const filtered = shown.length !== prompts.length || query.trim() !== "" || activeCategory !== null;

  const exportFile = () => {
    try {
      const blob = new Blob([exportJson(prompts)], { type: "application/json" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = "vimmer-prompts.json";
      document.body.appendChild(a);
      a.click();
      a.remove();
      window.setTimeout(() => URL.revokeObjectURL(url), 1000);
      setNotice(`已导出 ${prompts.length} 条 · Exported`);
    } catch {
      setNotice("导出失败，可用「复制」Copy instead");
    }
  };

  const copyJson = async () => {
    try {
      await navigator.clipboard.writeText(exportJson(prompts));
      setNotice("已复制到剪贴板 · Copied as JSON");
    } catch {
      setNotice("无法复制 Could not copy");
    }
  };

  const importFile = async (file: File | undefined) => {
    if (!file) return;
    if (file.size > MAX_IMPORT_CHARS * 4) {
      setNotice("无法导入 Cannot import: file too large");
      return;
    }
    const parsed = parseImport(await file.text());
    if (!parsed.ok) {
      setNotice(parsed.error);
      return;
    }
    const r = mergeImport(prompts, parsed.prompts);
    setPrompts(r.list);
    setNotice(`导入 ${r.added} 条${r.skipped ? `，跳过 ${r.skipped} 条（重复或已满）` : ""} · Imported ${r.added}`);
  };

  return (
    <div className="prompt-manager" role="region" aria-label="提示词管理 Prompt manager">
      <div className="assistant-row">
        <div className="saved-form-title prompt-manager-title">提示词管理</div>
        <button className="btn ghost small" onClick={onClose} aria-label="Close 关闭">✕</button>
      </div>
      <div className="assistant-row prompt-filters">
        <input type="search" value={query} placeholder="搜索 Search" onChange={(e) => setQuery(e.target.value)} aria-label="搜索提示词 Search prompts" />
        <select
          value={activeCategory ?? ""}
          onChange={(e) => setCategory(e.target.value === "" ? null : e.target.value)}
          aria-label="分类 Category"
        >
          <option value="">全部分类 All</option>
          {categories.map((c) => <option key={c} value={c}>{c}</option>)}
          <option value={NO_CATEGORY}>未分类 None</option>
        </select>
      </div>
      {filtered && (
        <div className="dim small">
          {shown.length} / {prompts.length} 条
          <button className="link small" onClick={() => { setQuery(""); setCategory(null); }}>清除 Clear</button>
        </div>
      )}

      <div className="saved-list" role="list" aria-label="提示词 Prompts">
        {prompts.length === 0 && (
          <div className="dim small">
            还没有提示词。
            <button className="link" onClick={() => setPrompts(starterPrompts())}>恢复示例 Restore examples</button>
          </div>
        )}
        {prompts.length > 0 && shown.length === 0 && <div className="dim small">没有匹配的提示词 · No match</div>}
        {shown.map((p) => {
          const i = prompts.indexOf(p);
          return editing === p.id ? (
            <PromptForm
              key={p.id}
              initial={{ name: p.name, text: p.text, useScene: p.useScene, category: p.category ?? "" }}
              categories={categories}
              onCancel={() => setEditing(null)}
              onSave={(change) => {
                const r = updatePrompt(prompts, p.id, change);
                if (!r.ok) return r.error;
                setPrompts(r.list);
                setEditing(null);
                return null;
              }}
            />
          ) : (
            <div key={p.id} className="saved-item manager-item" role="listitem">
              <div className="manager-text" title={p.text}>
                <div className="saved-name">
                  <span className="manager-name">{p.name}</span>
                  {p.category && <span className="saved-cat">{p.category}</span>}
                </div>
                <div className="dim small manager-preview">{p.text}</div>
              </div>
              <span className="saved-tools">
                <button className="btn ghost small" onClick={() => setPrompts(movePrompt(prompts, p.id, -1))} disabled={filtered || i === 0} aria-label={`Move ${p.name} up`} title={filtered ? "清除搜索和分类后才能排序 Clear the filter to reorder" : "上移 Move up"}>↑</button>
                <button className="btn ghost small" onClick={() => setPrompts(movePrompt(prompts, p.id, 1))} disabled={filtered || i === prompts.length - 1} aria-label={`Move ${p.name} down`} title={filtered ? "清除搜索和分类后才能排序 Clear the filter to reorder" : "下移 Move down"}>↓</button>
                <button className="btn ghost small" onClick={() => { setEditing(p.id); setAdding(false); }} aria-label={`Edit ${p.name}`} title="改名 / 编辑 Rename / edit">✎</button>
                <button className="btn ghost small danger" onClick={() => { onRemove(p.id); if (editing === p.id) setEditing(null); }} aria-label={`Delete ${p.name}`} title="删除 Delete">✕</button>
              </span>
            </div>
          );
        })}
      </div>

      {adding ? (
        <PromptForm
          title="新建提示词 New prompt"
          initial={{ name: "", text: "", useScene: false, category: activeCategory && activeCategory !== NO_CATEGORY ? activeCategory : "" }}
          categories={categories}
          full={prompts.length >= MAX_PROMPTS}
          onCancel={() => setAdding(false)}
          onSave={(input) => {
            const r = addPrompt(prompts, input);
            if (!r.ok) return r.error;
            setPrompts(r.list);
            setAdding(false);
            return null;
          }}
        />
      ) : (
        <div className="assistant-row saved-io">
          <button className="btn small" onClick={() => { setAdding(true); setEditing(null); }} disabled={prompts.length >= MAX_PROMPTS}>＋ 新建 New</button>
          <button className="btn small" onClick={exportFile} disabled={prompts.length === 0} title="Download as a JSON file (backup, or copy to another Mac)">导出 Export</button>
          <button className="btn small" onClick={() => void copyJson()} disabled={prompts.length === 0} title="Copy the same JSON to the clipboard">复制 Copy</button>
          <button className="btn small" onClick={() => fileInput.current?.click()} title="Add prompts from an exported JSON file">导入 Import</button>
          <input
            ref={fileInput}
            type="file"
            accept=".json,application/json"
            hidden
            onChange={(e) => {
              void importFile(e.target.files?.[0]);
              e.target.value = "";
            }}
            aria-label="Import prompts file"
          />
        </div>
      )}
      <div className="dim small">最多 {MAX_PROMPTS} 条 · 可以写 {"{场景}"}，使用时换成所选的那一幕。</div>
    </div>
  );
};

type FormProps = {
  title?: string;
  initial: Fields;
  categories: string[];
  full?: boolean;
  /** Offer to turn 「第 3 幕 (scene-03)」 back into {场景} (saving a sent message). */
  offerGeneralize?: boolean;
  onSave: Saver;
  onCancel: () => void;
};

/** Name, text, category and scene flag: used to save, add and edit. */
const PromptForm = ({ title, initial, categories, full = false, offerGeneralize = false, onSave, onCancel }: FormProps) => {
  const [name, setName] = useState(initial.name);
  const [text, setText] = useState(initial.text);
  const [category, setCategory] = useState(initial.category);
  const [useScene, setUseScene] = useState(initial.useScene);
  const [error, setError] = useState<string | null>(null);
  const listId = useId();
  const general = generalizeScene(text);
  return (
    <div className="saved-form" role="group" aria-label={title ?? `Edit ${initial.name}`}>
      {title && <div className="saved-form-title">{title}</div>}
      <input value={name} maxLength={MAX_NAME} placeholder={`名字（可选）${defaultName(text)}`} onChange={(e) => setName(e.target.value)} aria-label="Name 名字" autoFocus />
      <textarea rows={2} value={text} maxLength={MAX_TEXT} placeholder="提示词 e.g. 把{场景}节奏放慢一点" onChange={(e) => setText(e.target.value)} aria-label="Prompt 提示词" />
      {offerGeneralize && general !== text && (
        <button className="link small" onClick={() => setText(general)} title="So it works for whichever scene is selected">
          把场景换成 {"{场景}"}（适用于任何一幕）
        </button>
      )}
      <input
        value={category}
        maxLength={MAX_CATEGORY}
        list={listId}
        placeholder="分类（可选）如 拆分 / 节奏 / 调色"
        onChange={(e) => setCategory(e.target.value)}
        aria-label="Category 分类"
      />
      <datalist id={listId}>
        {categories.map((c) => <option key={c} value={c} />)}
      </datalist>
      <label className="check">
        <input type="checkbox" checked={useScene} onChange={(e) => setUseScene(e.target.checked)} />
        关于所选场景 About the selected scene
      </label>
      {(error || full) && <div className="field-error">{error ?? `最多保存 ${MAX_PROMPTS} 条，请先删除一些`}</div>}
      <div className="assistant-row">
        <button className="btn primary small" disabled={!text.trim() || full} onClick={() => setError(onSave({ name, text, useScene, category }))}>保存 Save</button>
        <button className="btn small" onClick={onCancel}>取消 Cancel</button>
      </div>
    </div>
  );
};
