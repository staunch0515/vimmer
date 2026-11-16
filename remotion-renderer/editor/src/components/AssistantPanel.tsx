import { useEffect, useReducer, useRef, useState, type Dispatch } from "react";
import type { VideoTimeline } from "../../../src/types/timeline";
import type { Selection } from "../model";
import type { Action } from "../state";
import { serialize } from "../project";
import {
  ASSISTANT_MODELS, failureHint, formatCost, proposeEdit,
  type AssistantModel, type EditProposal,
} from "../assistant";
import {
  buildConversation, busy as isBusy, chatReducer, failureOf, initialChat, nextExchangeId, openProposal,
  type Exchange,
} from "../chat";
import type { SavedPrompt } from "../savedPrompts";
import {
  ALL_SCENES, SCOPE_ERROR, needsScene, sceneLabel, sceneList, outOfScope, outOfScopeWarning, pruneChosen, resolveScope, sceneOptions,
  type ResolveError, type SceneRef, type ScopeMode,
} from "../scope";
import { SavedPrompts, useSavedPrompts, type SaveDraft } from "./SavedPrompts";
import { ScopeControl } from "./ScopeControl";

type Props = {
  timeline: VideoTimeline;
  selection: Selection;
  dispatch: Dispatch<Action>;
  /** Changes when another document is opened: the chat starts over. */
  docKey: number;
};

const STATUS_LABEL = { edited: "Proposed edit 修改建议", clarify: "Question 需要确认", refused: "Not possible 无法完成" } as const;

const DECISION_LABEL = {
  accepted: "✓ Accepted 已接受",
  rejected: "Rejected 已拒绝",
  superseded: "Not applied 未采用 (you moved on)",
} as const;

/** "Claude Opus 5.5" -> "Opus 5.5", for the answer cards. */
const shortModel = (id: AssistantModel): string =>
  (ASSISTANT_MODELS.find((m) => m.id === id)?.label ?? id).replace(/^Claude /, "");

const selectedScene = (t: VideoTimeline, s: Selection): SceneRef | null => {
  const index = s.kind === "scene" ? s.index : s.kind === "keyframe" || s.kind === "layer" ? s.sceneIndex : -1;
  const id = t.scenes[index]?.id;
  return id ? { index, id } : null;
};

/** What {场景} becomes with this scope (for the line under the message box). */
const placeholderTarget = (mode: ScopeMode, current: SceneRef | null, chosen: SceneRef[]): string =>
  mode === "selected" ? sceneList(chosen) : mode === "one" && current ? sceneLabel(current.index, current.id) : ALL_SCENES;

/** How a message was scoped (kept beside the chat, by exchange id). */
type SentScope = {
  /** The text before the scope was applied ({场景} kept), for ☆ 收藏. */
  raw: string;
  mode: ScopeMode;
  /** The 选几幕 line put before the text, shown dimmed. */
  line: string | null;
  scopeIds: string[] | null;
  /** The scenes when the message was sent (to tell changed scenes from new ones). */
  knownIds: string[];
};

/**
 * Natural-language editing as a chat. Claude sees the current timeline and the recent
 * exchanges, so follow-ups work. Nothing changes until Accept; Accept is one undo step.
 */
export const AssistantPanel = ({ timeline, selection, dispatch, docKey }: Props) => {
  const [instruction, setInstruction] = useState("");
  /** 范围: 全部 / 选几幕 / 当前这一幕 (当前这一幕 with no scene selected acts as 全部, like the old checkbox). */
  const [scopeMode, setScopeMode] = useState<ScopeMode>("one");
  const [chosenIds, setChosenIds] = useState<string[]>([]);
  const [scopeListOpen, setScopeListOpen] = useState(false);
  const [sentScopes, setSentScopes] = useState<Record<number, SentScope>>({});
  const [chat, chatDispatch] = useReducer(chatReducer, docKey, initialChat);
  const logEnd = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLTextAreaElement>(null);
  const [prompts, setPrompts] = useSavedPrompts();
  const [saveDraft, setSaveDraft] = useState<SaveDraft | null>(null);
  /** Why the last Send did not go out (a scene is needed / none ticked). */
  const [scopeError, setScopeError] = useState<ResolveError | null>(null);
  const model = chat.model;
  const scene = selectedScene(timeline, selection);
  const options = sceneOptions(timeline.scenes);
  const chosen = pruneChosen(chosenIds, timeline.scenes);
  const chosenRefs: SceneRef[] = options.filter((o) => chosen.includes(o.id)).map(({ index, id }) => ({ index, id }));
  const preview = instruction.trim() && needsScene(instruction)
    ? resolveScope({ mode: scopeMode, text: instruction, current: scene, chosen: chosenRefs })
    : null;
  const current = serialize(timeline);
  const busy = isBusy(chat);
  const conversation = buildConversation(chat.exchanges);

  useEffect(() => {
    chatDispatch({ type: "openDocument", doc: docKey });
  }, [docKey]);
  // Braces matter: newer browsers return a Promise from scrollIntoView, and React must get no value back.
  useEffect(() => {
    logEnd.current?.scrollIntoView?.({ block: "nearest" });
  }, [chat.exchanges]);

  /** Sets the scope a prompt was saved with (选几幕 starts from the selected scene if none is ticked). */
  const takeScope = (mode: ScopeMode): SceneRef[] => {
    setScopeMode(mode);
    if (mode !== "selected" || chosenRefs.length > 0 || !scene) return chosenRefs;
    setChosenIds([scene.id]);
    setScopeListOpen(true);
    return [scene];
  };

  /** Sends the input, or a saved prompt directly (`saved`; the input is then left as it is). */
  const send = async (saved?: SavedPrompt) => {
    const raw = (saved ? saved.text : instruction).trim();
    if (!raw || busy) return;
    const mode = saved?.scope ?? scopeMode;
    const picked = saved?.scope ? takeScope(saved.scope) : chosenRefs;
    // The scope decides the scene id and, for 选几幕, a line before the text; {场景} is filled in.
    const r = resolveScope({ mode, text: raw, current: scene, chosen: picked });
    if (!r.ok) {
      setScopeError(r.error);
      if (r.error === "noScenesChosen") setScopeListOpen(true);
      return;
    }
    setScopeError(null);
    const { instruction: text, sceneId: sceneForRequest } = r.value;
    const id = nextExchangeId(chat);
    const generation = chat.generation;
    setSentScopes((m) => ({
      ...m,
      [id]: { raw, mode: r.value.mode, line: r.value.line, scopeIds: r.value.scopeIds, knownIds: timeline.scenes.map((s) => s.id) },
    }));
    chatDispatch({ type: "send", instruction: text, sceneId: sceneForRequest, base: current });
    if (!saved) setInstruction("");
    const result = await proposeEdit({
      timelineJson: current,
      instruction: text,
      sceneId: sceneForRequest,
      model,
      ...(conversation.turns.length ? { conversation } : {}),
    });
    chatDispatch({ type: "answer", id, generation, result });
  };

  /** Puts a saved prompt in the box ({场景} kept; filled in on Send) and sets its scope, for editing before Send. */
  const applyPrompt = (c: SavedPrompt) => {
    setInstruction(c.text);
    setScopeError(null);
    if (c.scope) takeScope(c.scope);
    input.current?.focus();
  };

  const accept = (e: Exchange, p: EditProposal) => {
    if (e.base !== current) return;
    dispatch({ type: "applyAssistant", timeline: p.outcome.timeline });
    chatDispatch({ type: "accept", id: e.id });
  };

  const last = chat.exchanges[chat.exchanges.length - 1];
  const asked = last?.result?.ok && last.result.status === "clarify";

  return (
    <div className="form assistant">
      <div className="assistant-row">
        <div className="section-title assistant-title">Chat with Claude 助手</div>
        <button
          className="btn"
          onClick={() => chatDispatch({ type: "clear" })}
          disabled={busy || chat.exchanges.length === 0}
          title="Forget this conversation and start over"
        >
          新对话 New chat
        </button>
      </div>
      <div className="assistant-row chat-header">
        <label className="chat-model" title="Used for your next message. Each answer shows the model that wrote it.">
          <span className="dim small">模型 Model</span>
          <select value={model} onChange={(e) => chatDispatch({ type: "setModel", model: e.target.value as AssistantModel })} disabled={busy} aria-label="Model 模型">
            {ASSISTANT_MODELS.map((m) => <option key={m.id} value={m.id}>{m.label}</option>)}
          </select>
        </label>
      </div>

      <div className="chat-log" aria-live="polite">
        {chat.exchanges.length === 0 && (
          <div className="dim small">
            Ask for a change, then follow up: “再快一点”, “第 3 幕也这样”, “刚才那个不要了”. Claude remembers this chat
            (not saved; cleared when you open another file).
          </div>
        )}
        {conversation.dropped > 0 && (
          <div className="dim small chat-note">
            {conversation.dropped} older message{conversation.dropped > 1 ? "s are" : " is"} no longer sent to Claude ·
            较早的 {conversation.dropped} 条对话不再发送
          </div>
        )}
        {chat.exchanges.map((e) => (
          <ExchangeView
            key={e.id}
            exchange={e}
            current={current}
            onAccept={accept}
            onReject={(id) => chatDispatch({ type: "reject", id })}
            scope={sentScopes[e.id]}
            onSave={(ex) => {
              const sent = sentScopes[ex.id];
              setSaveDraft(sent ? { text: sent.raw, scope: sent.mode } : { text: ex.instruction, scope: ex.sceneId ? "one" : "all" });
            }}
          />
        ))}
        <div ref={logEnd} />
      </div>

      <SavedPrompts
        prompts={prompts}
        setPrompts={setPrompts}
        onUse={applyPrompt}
        onSend={(c) => void send(c)}
        busy={busy}
        draft={saveDraft}
        onDraftDone={() => setSaveDraft(null)}
      />

      <ScopeControl
        mode={scopeMode}
        onMode={(m) => {
          setScopeMode(m);
          setScopeError(null);
        }}
        current={scene}
        options={options}
        chosen={chosen}
        onChosen={(ids) => {
          setChosenIds(ids);
          setScopeError(null);
        }}
        listOpen={scopeListOpen}
        onListOpen={setScopeListOpen}
        disabled={busy}
      />

      <textarea
        ref={input}
        rows={3}
        value={instruction}
        placeholder={asked ? "Answer the question above · 回答上面的问题" : "e.g. 第二段节奏快一点，加一个淡入转场 · make scene 3 slower · switch to vertical"}
        onChange={(e) => {
          setInstruction(e.target.value);
          setScopeError(null);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            void send();
          }
        }}
        aria-label="Message to the assistant"
      />
      {preview?.ok && !scopeError && (
        <div className="dim small scope-preview-line">{"{场景}"} → {placeholderTarget(preview.value.mode, scene, chosenRefs)}</div>
      )}
      {scopeError && <div className="assistant-warning" role="alert">{SCOPE_ERROR[scopeError]}</div>}
      <div className="assistant-row chat-send">
        <button
          className="link small saved-save"
          onClick={() => setSaveDraft({ text: instruction, scope: scopeMode === "one" && !scene ? "all" : scopeMode })}
          disabled={!instruction.trim()}
          title="Save the text in the box as a reusable prompt"
        >
          ☆ 保存为提示词
        </button>
        <button className="btn primary" onClick={() => void send()} disabled={!instruction.trim() || busy}>
          {busy ? "Thinking… 思考中" : "Send 发送"}
        </button>
      </div>
      <div className="dim small">Each message is a paid API call. Nothing changes until you accept. ⌘↵ sends.</div>
    </div>
  );
};

type ExchangeProps = {
  exchange: Exchange;
  current: string;
  onAccept: (e: Exchange, p: EditProposal) => void;
  onReject: (id: number) => void;
  onSave: (e: Exchange) => void;
  scope?: SentScope;
};

const ExchangeView = ({ exchange: e, current, onAccept, onReject, onSave, scope }: ExchangeProps) => {
  const failure = failureOf(e);
  const open = openProposal(e);
  const stale = open !== null && e.base !== current;
  // The 选几幕 line is shown dimmed above the user's own words.
  const line = scope?.line && e.instruction.startsWith(`${scope.line}\n`) ? scope.line : null;
  const words = line ? e.instruction.slice(line.length + 1) : e.instruction;
  const outside = e.result?.ok && e.result.status === "edited" && scope ? outOfScope(e.result.outcome.summary, scope.scopeIds, scope.knownIds) : [];
  return (
    <>
      <div className="chat-user">
        {line && <div className="dim small chat-scope-line">{line}</div>}
        <div>{words}</div>
        <div className="chat-user-foot">
          {e.sceneId ? <span className="dim small mono">{e.sceneId}</span> : <span />}
          <button className="link small" onClick={() => onSave(e)} title="Save this message as a reusable prompt">☆ 收藏 Save</button>
        </div>
      </div>
      {e.result === null && <div className="assistant-card chat-wait dim">Thinking… 思考中</div>}
      {failure && (
        <div className="assistant-card bad" role="alert">
          <div className="assistant-status">Assistant unavailable 助手不可用</div>
          <div>{failure.error.message}</div>
          {failureHint(failure) && <div className="dim">{failureHint(failure)}</div>}
          <div className="dim mono small">
            {shortModel(e.model)}
            {failure.usage && failure.usage.cost_usd > 0 ? ` · ${formatCost(failure.usage)}` : ""}
          </div>
        </div>
      )}
      {e.result?.ok && (
        <div className={`assistant-card ${e.result.status} ${e.decision}`} data-decision={e.result.status === "edited" ? e.decision : undefined}>
          <div className="assistant-status">{STATUS_LABEL[e.result.status]}</div>
          <div>{e.result.explanation}</div>
          {e.result.outcome.summary.length > 0 && (
            <ul className="assistant-diff">
              {e.result.outcome.summary.map((line) => <li key={line} className="mono">{line}</li>)}
            </ul>
          )}
          {e.result.outcome.warnings.map((w) => <div key={w} className="assistant-warning">⚠ {w}</div>)}
          <div className="dim mono small" title={e.model}>
            {shortModel(e.model)} · {formatCost(e.result.usage)}
            {e.result.retries > 0 ? ` · ${e.result.retries} correction${e.result.retries > 1 ? "s" : ""}` : ""}
          </div>
          {outside.length > 0 && e.decision === "pending" && (
            <div className="assistant-warning scope-outside" role="alert">⚠ {outOfScopeWarning(outside)}</div>
          )}
          {stale && <div className="assistant-warning">The timeline changed since this was proposed. Ask again. 时间线已变，请重新提问。</div>}
          {open && (
            <div className="assistant-row">
              <button className="btn primary" onClick={() => onAccept(e, open)} disabled={stale}>Accept 接受</button>
              <button className="btn" onClick={() => onReject(e.id)}>Reject 拒绝</button>
            </div>
          )}
          {e.result.status === "edited" && e.decision !== "pending" && (
            <div className={`chat-decision ${e.decision}`}>{DECISION_LABEL[e.decision]}</div>
          )}
        </div>
      )}
    </>
  );
};
