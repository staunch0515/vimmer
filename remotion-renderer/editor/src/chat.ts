// The assistant's chat: what was said, what was proposed and what the user did with it.
// Pure state (no React, no backend), so it is tested on its own. In memory only: it belongs to
// the open document and is cleared when another one is opened.
//
// Each request still sends the CURRENT timeline. The chat adds the recent exchanges as compact
// text (`Conversation`): the user's words, the assistant's answer and the change summary, and
// whether each proposed edit was accepted. Old timelines are never resent. This is not the
// backend's retry `history` (the rejected attempts of one request); it is a separate field.

import type { AssistantModel, EditFailure, EditProposal, EditResult } from "./assistant";

/** The model a chat starts with (the user's choice is kept across New chat and documents). */
export const DEFAULT_MODEL: AssistantModel = "claude-opus-5-5";

export type Decision = "pending" | "accepted" | "rejected" | "superseded";

/** One message from the user and the assistant's answer to it. */
export type Exchange = {
  id: number;
  instruction: string;
  sceneId: string | null;
  /** null while the request runs. */
  result: EditResult | null;
  /** The timeline (serialized) the request was made against. */
  base: string;
  /** What the user did with an `edited` proposal; "pending" for everything else. */
  decision: Decision;
  /** The model that answered (the chat's model when the message was sent). */
  model: AssistantModel;
};

export type ChatState = {
  /** The document this chat belongs to (see `openDocument`). */
  doc: number;
  /** Bumped when the chat is cleared: an answer to an older request is dropped. */
  generation: number;
  nextId: number;
  exchanges: Exchange[];
  /** One model per chat: every message of this chat goes to it. */
  model: AssistantModel;
};

export type ChatAction =
  | { type: "send"; instruction: string; sceneId: string | null; base: string }
  | { type: "answer"; id: number; generation: number; result: EditResult }
  | { type: "accept"; id: number }
  | { type: "reject"; id: number }
  | { type: "clear" }
  | { type: "openDocument"; doc: number }
  /**
   * Switch the chat's model. An empty chat just switches. A chat with messages switches only
   * when `confirmed` (the user agreed to start over), and is then cleared like New chat.
   * Ignored while a request runs.
   */
  | { type: "setModel"; model: AssistantModel; confirmed?: boolean };

export const initialChat = (doc = 0, model: AssistantModel = DEFAULT_MODEL): ChatState => ({
  doc, generation: 0, nextId: 1, exchanges: [], model,
});

export const busy = (s: ChatState): boolean => s.exchanges.some((e) => e.result === null);

/** The exchange a new message gets (for the caller to know its id before dispatching). */
export const nextExchangeId = (s: ChatState): number => s.nextId;

/** Whether switching to `model` would clear this chat (so the user has to confirm it). */
export const modelChangeClears = (s: ChatState, model: AssistantModel): boolean =>
  model !== s.model && s.exchanges.length > 0;

const isEdit = (e: Exchange): e is Exchange & { result: EditProposal } => !!e.result?.ok && e.result.status === "edited";

const decide = (s: ChatState, id: number, decision: "accepted" | "rejected"): ChatState => ({
  ...s,
  exchanges: s.exchanges.map((e) => (e.id === id && isEdit(e) && e.decision === "pending" ? { ...e, decision } : e)),
});

export const chatReducer = (s: ChatState, a: ChatAction): ChatState => {
  switch (a.type) {
    case "send": {
      if (busy(s)) return s;
      // Moving on without deciding: an open proposal is no longer offered.
      const exchanges = s.exchanges.map((e) => (isEdit(e) && e.decision === "pending" ? { ...e, decision: "superseded" as const } : e));
      const added: Exchange = { id: s.nextId, instruction: a.instruction, sceneId: a.sceneId, result: null, base: a.base, decision: "pending", model: s.model };
      return { ...s, nextId: s.nextId + 1, exchanges: [...exchanges, added] };
    }
    case "answer":
      if (a.generation !== s.generation) return s;
      return { ...s, exchanges: s.exchanges.map((e) => (e.id === a.id && e.result === null ? { ...e, result: a.result } : e)) };
    case "accept":
      return decide(s, a.id, "accepted");
    case "reject":
      return decide(s, a.id, "rejected");
    case "clear":
      return { ...s, generation: s.generation + 1, exchanges: [] };
    case "openDocument":
      return a.doc === s.doc ? s : { ...initialChat(a.doc, s.model), generation: s.generation + 1, nextId: s.nextId };
    case "setModel":
      if (a.model === s.model || busy(s)) return s;
      if (s.exchanges.length === 0) return { ...s, model: a.model };
      if (!a.confirmed) return s;
      return { ...s, model: a.model, generation: s.generation + 1, exchanges: [] };
  }
};

/** The proposal of an exchange that can still be accepted, or null. */
export const openProposal = (e: Exchange): EditProposal | null => (isEdit(e) && e.decision === "pending" ? e.result : null);

/** An answered exchange's failure, if it failed. */
export const failureOf = (e: Exchange): EditFailure | null => (e.result && !e.result.ok ? e.result : null);

// ---- What is sent to Claude ---------------------------------------------------------------

export type ConversationTurn = {
  user: string;
  status: EditProposal["status"];
  reply: string;
  changes: string[];
  decision?: "accepted" | "rejected" | "ignored";
};

/** Mirrors `vimmer_storyboard::edit_claude::Conversation`. */
export type Conversation = { turns: ConversationTurn[]; dropped: number };

/** Most earlier exchanges sent with one request. */
export const MAX_TURNS = 6;
/** Most characters of earlier chat sent with one request (the backend allows 16000). */
export const MAX_CHARS = 6000;
const MAX_USER = 600;
const MAX_REPLY = 600;
const MAX_CHANGE = 200;
const MAX_CHANGES = 8;

const clip = (text: string, n: number): string => {
  const t = text.trim().replace(/\s+/g, " ");
  return t.length <= n ? t : `${t.slice(0, n - 1)}…`;
};

const turnOf = (e: Exchange): ConversationTurn | null => {
  if (!e.result?.ok) return null; // still running, or failed: nothing was answered
  const p = e.result;
  const turn: ConversationTurn = {
    user: clip(e.instruction, MAX_USER),
    status: p.status,
    reply: clip(p.explanation, MAX_REPLY),
    changes: p.outcome.summary.slice(0, MAX_CHANGES).map((c) => clip(c, MAX_CHANGE)),
  };
  if (p.outcome.summary.length > MAX_CHANGES) turn.changes.push(`(${p.outcome.summary.length - MAX_CHANGES} more changes)`);
  if (p.status === "edited") {
    turn.decision = e.decision === "accepted" ? "accepted" : e.decision === "rejected" ? "rejected" : "ignored";
  }
  return turn;
};

const size = (t: ConversationTurn): number => t.user.length + t.reply.length + t.changes.reduce((n, c) => n + c.length, 0);

/**
 * The recent chat to send with the next request: answered exchanges only, the newest
 * `maxTurns`, and fewer when they would pass `maxChars`. `dropped` counts the older answered
 * exchanges left out (shown in the panel and told to Claude).
 */
export const buildConversation = (exchanges: Exchange[], maxTurns = MAX_TURNS, maxChars = MAX_CHARS): Conversation => {
  const all = exchanges.map(turnOf).filter((t): t is ConversationTurn => t !== null);
  const turns: ConversationTurn[] = [];
  let chars = 0;
  for (let i = all.length - 1; i >= 0 && turns.length < maxTurns; i -= 1) {
    chars += size(all[i]);
    if (chars > maxChars && turns.length > 0) break;
    turns.unshift(all[i]);
  }
  return { turns, dropped: all.length - turns.length };
};
