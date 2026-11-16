import { SCOPE_LABEL, SCOPE_MODES, type SceneOption, type SceneRef, type ScopeMode } from "../scope";

type Props = {
  mode: ScopeMode;
  onMode: (m: ScopeMode) => void;
  /** The scene selected on the timeline. */
  current: SceneRef | null;
  options: SceneOption[];
  chosen: string[];
  onChosen: (ids: string[]) => void;
  listOpen: boolean;
  onListOpen: (open: boolean) => void;
  disabled?: boolean;
};

/**
 * 范围: 全部 / 选几幕 / 当前这一幕, above the message box. 当前这一幕 is greyed out with no scene
 * selected (the message then goes to all scenes, as the old unticked checkbox did). 选几幕 has
 * a small list of scenes to tick.
 */
export const ScopeControl = ({ mode, onMode, current, options, chosen, onChosen, listOpen, onListOpen, disabled }: Props) => {
  // 当前这一幕 without a selected scene acts as 全部.
  const shown: ScopeMode = mode === "one" && !current ? "all" : mode;
  const picked = new Set(chosen);
  return (
    <div className="scope">
      <div className="assistant-row scope-row" role="radiogroup" aria-label="范围 Scope">
        <span className="dim small">范围</span>
        {SCOPE_MODES.map((m) => (
          <button
            key={m}
            role="radio"
            aria-checked={shown === m}
            className={`scope-btn ${shown === m ? "on" : ""}`}
            onClick={() => {
              onMode(m);
              if (m === "selected") {
                if (chosen.length === 0 && current) onChosen([current.id]);
                onListOpen(true);
              }
            }}
            disabled={disabled || (m === "one" && !current)}
            title={
              m === "all" ? "所有场景 · Claude may change any scene"
                : m === "selected" ? "只改勾选的几幕 · Only the ticked scenes"
                  : current ? `只改 ${current.id} · Only the selected scene` : "先在时间线上选中一幕 · Select a scene on the timeline first"
            }
          >
            {SCOPE_LABEL[m]}
            {m === "selected" && chosen.length > 0 && <span className="scope-count"> {chosen.length}</span>}
            {m === "one" && current && <span className="mono scope-id"> {current.id}</span>}
          </button>
        ))}
        {shown === "selected" && (
          <button className="link small" onClick={() => onListOpen(!listOpen)} aria-expanded={listOpen}>
            {listOpen ? "收起" : "改选"}
          </button>
        )}
      </div>
      {shown === "selected" && listOpen && (
        <div className="scope-list" role="group" aria-label="选几幕 Scenes">
          <div className="assistant-row">
            <button className="link small" onClick={() => onChosen(options.map((o) => o.id))}>全选</button>
            <button className="link small" onClick={() => onChosen([])}>清空</button>
            <span className="dim small">已选 {chosen.length} / {options.length}</span>
          </div>
          {options.map((o) => (
            <label key={o.id} className="check scope-item" title={o.preview}>
              <input
                type="checkbox"
                checked={picked.has(o.id)}
                onChange={(e) => onChosen(e.target.checked ? options.filter((x) => x.id === o.id || picked.has(x.id)).map((x) => x.id) : chosen.filter((id) => id !== o.id))}
              />
              <span className="scope-num">第 {o.index + 1} 幕</span>
              <span className="mono small">{o.id}</span>
              {o.preview && <span className="dim small scope-preview">{o.preview}</span>}
            </label>
          ))}
        </div>
      )}
    </div>
  );
};
