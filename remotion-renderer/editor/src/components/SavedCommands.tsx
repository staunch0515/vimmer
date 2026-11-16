import { useEffect, useRef, useState, type Dispatch, type SetStateAction } from "react";
import {
  MAX_COMMANDS, MAX_IMPORT_CHARS, MAX_NAME, MAX_TEXT, STORAGE_KEY,
  addCommand, defaultName, deleteCommand, exportJson, generalizeScene, loadCommands, mergeImport, moveCommand,
  needsScene, parseImport, restoreCommand, saveCommands, starterCommands, updateCommand,
  type Removed, type SavedCommand,
} from "../savedCommands";

/** localStorage, or null where reading it throws (private mode, blocked storage). */
const storage = (): Storage | null => {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
};

/**
 * The user's saved commands, kept in localStorage (global, not per project). The starters are
 * shown until something is saved. Other windows of the editor pick up changes.
 */
export const useSavedCommands = (): [SavedCommand[], Dispatch<SetStateAction<SavedCommand[]>>] => {
  const [list, setList] = useState<SavedCommand[]>(() => loadCommands(storage()) ?? starterCommands());
  const changed = useRef(false);
  const set: Dispatch<SetStateAction<SavedCommand[]>> = (v) => {
    changed.current = true;
    setList(v);
  };
  useEffect(() => {
    if (changed.current) saveCommands(storage(), list);
  }, [list]);
  useEffect(() => {
    const onStorage = (e: StorageEvent) => {
      if (e.key !== STORAGE_KEY) return;
      changed.current = false;
      setList(loadCommands(storage()) ?? starterCommands());
    };
    window.addEventListener("storage", onStorage);
    return () => window.removeEventListener("storage", onStorage);
  }, []);
  return [list, set];
};

/** What the "save" form starts from (a sent message, or the text in the input). */
export type SaveDraft = { text: string; useScene: boolean };

type Props = {
  commands: SavedCommand[];
  setCommands: Dispatch<SetStateAction<SavedCommand[]>>;
  /** Fill the input with this command. */
  onUse: (c: SavedCommand) => void;
  /** Send this command right away. */
  onSend: (c: SavedCommand) => void;
  busy: boolean;
  draft: SaveDraft | null;
  onDraftDone: () => void;
};

/**
 * 常用指令: a collapsible list above the message box. Click a command to put it in the box
 * (edit, then Send), ▶ (or Shift+click) to send it at once. 管理 Manage: rename, edit, delete
 * (with undo), move, export / import.
 */
export const SavedCommands = ({ commands, setCommands, onUse, onSend, busy, draft, onDraftDone }: Props) => {
  const [open, setOpen] = useState(false);
  const [manage, setManage] = useState(false);
  const [editing, setEditing] = useState<string | null>(null);
  const [undo, setUndo] = useState<Removed | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const fileInput = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!undo) return;
    const t = window.setTimeout(() => setUndo(null), 10_000);
    return () => window.clearTimeout(t);
  }, [undo]);

  const remove = (id: string) => {
    const r = deleteCommand(commands, id);
    if (!r.removed) return;
    setCommands(r.list);
    setUndo(r.removed);
    if (editing === id) setEditing(null);
  };

  const exportFile = () => {
    try {
      const blob = new Blob([exportJson(commands)], { type: "application/json" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = "vimmer-saved-commands.json";
      document.body.appendChild(a);
      a.click();
      a.remove();
      window.setTimeout(() => URL.revokeObjectURL(url), 1000);
      setNotice(`已导出 ${commands.length} 条 · Exported`);
    } catch {
      setNotice("导出失败，可用「复制」Copy instead");
    }
  };

  const copyJson = async () => {
    try {
      await navigator.clipboard.writeText(exportJson(commands));
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
    const r = mergeImport(commands, parsed.commands);
    setCommands(r.list);
    setNotice(`导入 ${r.added} 条${r.skipped ? `，跳过 ${r.skipped} 条（重复或已满）` : ""} · Imported ${r.added}`);
    setOpen(true);
  };

  return (
    <div className="saved-commands">
      {draft && (
        <SaveForm
          key={draft.text}
          draft={draft}
          full={commands.length >= MAX_COMMANDS}
          onCancel={onDraftDone}
          onSave={(input) => {
            const r = addCommand(commands, input);
            if (!r.ok) return r.error;
            setCommands(r.list);
            setOpen(true);
            setNotice(`已收藏「${r.list[r.list.length - 1].name}」 · Saved`);
            onDraftDone();
            return null;
          }}
        />
      )}
      <div className="assistant-row saved-head">
        <button className="btn ghost small" onClick={() => setOpen(!open)} aria-expanded={open} title="Saved instructions you can reuse">
          常用指令 {open ? "▴" : "▾"} <span className="dim">{commands.length}</span>
        </button>
        {open && (
          <button className={`btn ghost small ${manage ? "active" : ""}`} onClick={() => { setManage(!manage); setEditing(null); }} aria-pressed={manage}>
            {manage ? "完成 Done" : "管理 Manage"}
          </button>
        )}
      </div>

      {open && (
        <div className="saved-list" role="list" aria-label="常用指令 Saved commands">
          {commands.length === 0 && (
            <div className="dim small">
              还没有常用指令。在对话里点「☆ 收藏」或输入框下的「保存为常用指令」。
              <button className="link" onClick={() => setCommands(starterCommands())}>恢复示例 Restore examples</button>
            </div>
          )}
          {commands.map((c, i) =>
            editing === c.id ? (
              <EditForm
                key={c.id}
                command={c}
                onCancel={() => setEditing(null)}
                onSave={(change) => {
                  const r = updateCommand(commands, c.id, change);
                  if (!r.ok) return r.error;
                  setCommands(r.list);
                  setEditing(null);
                  return null;
                }}
              />
            ) : (
              <div key={c.id} className="saved-item" role="listitem">
                <button
                  className="saved-use"
                  onClick={(e) => (e.shiftKey && !busy ? onSend(c) : onUse(c))}
                  title={`${c.text}${c.useScene ? "\n(about the selected scene 关于所选场景)" : ""}\n\nClick: put in the box · Shift+click: send now`}
                >
                  <span className="saved-name">{c.name}</span>
                  {needsScene(c.text) && <span className="saved-tag">场景</span>}
                </button>
                {manage ? (
                  <span className="saved-tools">
                    <button className="btn ghost small" onClick={() => setCommands(moveCommand(commands, c.id, -1))} disabled={i === 0} aria-label={`Move ${c.name} up`} title="上移 Move up">↑</button>
                    <button className="btn ghost small" onClick={() => setCommands(moveCommand(commands, c.id, 1))} disabled={i === commands.length - 1} aria-label={`Move ${c.name} down`} title="下移 Move down">↓</button>
                    <button className="btn ghost small" onClick={() => setEditing(c.id)} aria-label={`Edit ${c.name}`} title="改名 / 编辑 Rename / edit">✎</button>
                    <button className="btn ghost small danger" onClick={() => remove(c.id)} aria-label={`Delete ${c.name}`} title="删除 Delete">✕</button>
                  </span>
                ) : (
                  <button className="btn ghost small" onClick={() => onSend(c)} disabled={busy} aria-label={`Send ${c.name} now`} title="直接发送 Send now (a paid API call)">▶</button>
                )}
              </div>
            ),
          )}
          {manage && (
            <div className="assistant-row saved-io">
              <button className="btn small" onClick={exportFile} disabled={commands.length === 0} title="Download as a JSON file (backup, or copy to another Mac)">导出 Export</button>
              <button className="btn small" onClick={() => void copyJson()} disabled={commands.length === 0} title="Copy the same JSON to the clipboard">复制 Copy</button>
              <button className="btn small" onClick={() => fileInput.current?.click()} title="Add commands from an exported JSON file">导入 Import</button>
              <input
                ref={fileInput}
                type="file"
                accept=".json,application/json"
                hidden
                onChange={(e) => {
                  void importFile(e.target.files?.[0]);
                  e.target.value = "";
                }}
                aria-label="Import saved commands file"
              />
            </div>
          )}
        </div>
      )}

      {undo && (
        <div className="saved-toast" role="status">
          已删除「{undo.command.name}」
          <button className="link" onClick={() => { setCommands((l) => restoreCommand(l, undo)); setUndo(null); }}>撤销 Undo</button>
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

/** Returns an error message, or null when saved. */
type Saver<T> = (input: T) => string | null;

const SaveForm = ({ draft, full, onSave, onCancel }: { draft: SaveDraft; full: boolean; onSave: Saver<{ name: string; text: string; useScene: boolean }>; onCancel: () => void }) => {
  const [text, setText] = useState(draft.text);
  const [name, setName] = useState("");
  const [useScene, setUseScene] = useState(draft.useScene);
  const [error, setError] = useState<string | null>(null);
  const general = generalizeScene(text);
  return (
    <div className="saved-form" role="group" aria-label="保存为常用指令 Save command">
      <div className="saved-form-title">☆ 保存为常用指令</div>
      <input value={name} maxLength={MAX_NAME} placeholder={`名字（可选）${defaultName(text)}`} onChange={(e) => setName(e.target.value)} aria-label="Name 名字" autoFocus />
      <textarea rows={2} value={text} maxLength={MAX_TEXT} onChange={(e) => setText(e.target.value)} aria-label="Instruction 指令" />
      {general !== text && (
        <button className="link small" onClick={() => setText(general)} title="So it works for whichever scene is selected">
          把场景换成 {"{场景}"}（适用于任何一幕）
        </button>
      )}
      <div className="dim small">可以写 {"{场景}"}，使用时换成所选的那一幕。</div>
      <label className="check">
        <input type="checkbox" checked={useScene} onChange={(e) => setUseScene(e.target.checked)} />
        关于所选场景 About the selected scene
      </label>
      {(error || full) && <div className="field-error">{error ?? `最多保存 ${MAX_COMMANDS} 条，请先删除一些`}</div>}
      <div className="assistant-row">
        <button className="btn primary small" disabled={!text.trim() || full} onClick={() => setError(onSave({ name, text, useScene }))}>保存 Save</button>
        <button className="btn small" onClick={onCancel}>取消 Cancel</button>
      </div>
    </div>
  );
};

const EditForm = ({ command, onSave, onCancel }: { command: SavedCommand; onSave: Saver<{ name: string; text: string; useScene: boolean }>; onCancel: () => void }) => {
  const [name, setName] = useState(command.name);
  const [text, setText] = useState(command.text);
  const [useScene, setUseScene] = useState(command.useScene);
  const [error, setError] = useState<string | null>(null);
  return (
    <div className="saved-form" role="group" aria-label={`Edit ${command.name}`}>
      <input value={name} maxLength={MAX_NAME} placeholder={defaultName(text)} onChange={(e) => setName(e.target.value)} aria-label="Name 名字" autoFocus />
      <textarea rows={2} value={text} maxLength={MAX_TEXT} onChange={(e) => setText(e.target.value)} aria-label="Instruction 指令" />
      <label className="check">
        <input type="checkbox" checked={useScene} onChange={(e) => setUseScene(e.target.checked)} />
        关于所选场景 About the selected scene
      </label>
      {error && <div className="field-error">{error}</div>}
      <div className="assistant-row">
        <button className="btn primary small" disabled={!text.trim()} onClick={() => setError(onSave({ name, text, useScene }))}>保存 Save</button>
        <button className="btn small" onClick={onCancel}>取消 Cancel</button>
      </div>
    </div>
  );
};
