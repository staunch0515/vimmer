import assert from "node:assert/strict";
import { test } from "node:test";
import {
  DEFAULT_MODEL, MAX_CHARS, MAX_TURNS, buildConversation, busy, chatReducer, initialChat, modelChangeClears, nextExchangeId,
  openProposal,
  type ChatAction, type ChatState,
} from "../editor/src/chat.ts";
import type { EditFailure, EditProposal } from "../editor/src/assistant.ts";
import { cliArgs, helpHasChat, helpHasEdit, parseArgs } from "../editor/assistant-server.ts";
import type { VideoTimeline } from "../src/types/timeline.ts";

const usage = { input_tokens: 1, output_tokens: 1, cache_creation_input_tokens: 0, cache_read_input_tokens: 0, cost_usd: 0.001 };
const timeline = { fps: 30, scenes: [] } as unknown as VideoTimeline;

const proposal = (status: EditProposal["status"], explanation: string, summary: string[] = []): EditProposal => ({
  ok: true,
  status,
  explanation,
  patch: [],
  outcome: { timeline, summary, warnings: [], changed: status === "edited" },
  usage,
  retries: 0,
});

const failed: EditFailure = { ok: false, error: { kind: "rate_limit", message: "slow down" } };

const run = (s: ChatState, ...actions: ChatAction[]) => actions.reduce(chatReducer, s);

/** Sends a message and answers it. */
const say = (s: ChatState, text: string, answer: EditProposal | EditFailure, base = "T0"): ChatState => {
  const id = nextExchangeId(s);
  const sent = chatReducer(s, { type: "send", instruction: text, sceneId: null, base });
  return chatReducer(sent, { type: "answer", id, generation: sent.generation, result: answer });
};

test("a message is in flight until answered; a second one waits", () => {
  const s = chatReducer(initialChat(), { type: "send", instruction: "慢一点", sceneId: "scene-02", base: "T0" });
  assert.equal(busy(s), true);
  assert.equal(chatReducer(s, { type: "send", instruction: "again", sceneId: null, base: "T0" }), s);
  const done = chatReducer(s, { type: "answer", id: 1, generation: 0, result: proposal("edited", "ok", ["scene-02: duration 4000 ms -> 5000 ms"]) });
  assert.equal(busy(done), false);
  assert.equal(done.exchanges[0].sceneId, "scene-02");
  assert.ok(openProposal(done.exchanges[0]));
});

test("accept and reject mark the proposal; only an open edit can be decided", () => {
  let s = say(initialChat(), "慢一点", proposal("edited", "ok", ["a"]));
  s = run(s, { type: "accept", id: 1 });
  assert.equal(s.exchanges[0].decision, "accepted");
  assert.equal(openProposal(s.exchanges[0]), null);
  assert.equal(run(s, { type: "reject", id: 1 }).exchanges[0].decision, "accepted");

  let r = say(initialChat(), "慢一点", proposal("edited", "ok", ["a"]));
  r = run(r, { type: "reject", id: 1 });
  assert.equal(r.exchanges[0].decision, "rejected");

  const q = run(say(initialChat(), "加转场", proposal("clarify", "哪种？")), { type: "accept", id: 1 });
  assert.equal(q.exchanges[0].decision, "pending");
  assert.equal(openProposal(q.exchanges[0]), null);
});

test("sending a new message supersedes an open proposal", () => {
  let s = say(initialChat(), "慢一点", proposal("edited", "ok", ["a"]));
  s = chatReducer(s, { type: "send", instruction: "再慢一点", sceneId: null, base: "T0" });
  assert.equal(s.exchanges[0].decision, "superseded");
  assert.equal(openProposal(s.exchanges[0]), null);
});

test("New chat clears the log and drops an answer that arrives afterwards", () => {
  let s = say(initialChat(), "慢一点", proposal("edited", "ok", ["a"]));
  s = chatReducer(s, { type: "send", instruction: "还要", sceneId: null, base: "T0" });
  const cleared = chatReducer(s, { type: "clear" });
  assert.deepEqual(cleared.exchanges, []);
  assert.equal(busy(cleared), false);
  const late = chatReducer(cleared, { type: "answer", id: 2, generation: s.generation, result: proposal("edited", "late", ["b"]) });
  assert.deepEqual(late.exchanges, []);
  assert.deepEqual(buildConversation(late.exchanges), { turns: [], dropped: 0 });
});

test("opening another document clears the chat; the same document keeps it", () => {
  let s = say(initialChat(1), "慢一点", proposal("edited", "ok", ["a"]));
  assert.equal(chatReducer(s, { type: "openDocument", doc: 1 }), s);
  const pendingGen = s.generation;
  s = chatReducer(s, { type: "send", instruction: "还要", sceneId: null, base: "T0" });
  const other = chatReducer(s, { type: "openDocument", doc: 2 });
  assert.equal(other.doc, 2);
  assert.deepEqual(other.exchanges, []);
  // The old document's request answers late: ignored.
  const late = chatReducer(other, { type: "answer", id: 2, generation: pendingGen, result: proposal("clarify", "?") });
  assert.deepEqual(late.exchanges, []);
  // Ids keep counting, so no answer can land on a new exchange by accident.
  assert.ok(nextExchangeId(other) > 2);
});

test("one model per chat: an empty chat just switches, a busy one never does", () => {
  const s = initialChat();
  assert.equal(s.model, DEFAULT_MODEL);
  assert.equal(DEFAULT_MODEL, "claude-opus-5-5");
  assert.equal(modelChangeClears(s, "claude-haiku-5-5"), false);
  const switched = chatReducer(s, { type: "setModel", model: "claude-haiku-5-5" });
  assert.equal(switched.model, "claude-haiku-5-5");
  assert.equal(switched.generation, s.generation);
  // The same model is a no-op.
  assert.equal(chatReducer(switched, { type: "setModel", model: "claude-haiku-5-5", confirmed: true }), switched);
  // Every message records the chat's model.
  const sent = chatReducer(switched, { type: "send", instruction: "慢一点", sceneId: null, base: "T0" });
  assert.equal(sent.exchanges[0].model, "claude-haiku-5-5");
  // In flight: the model cannot change, even confirmed.
  assert.equal(chatReducer(sent, { type: "setModel", model: "claude-opus-5-5", confirmed: true }), sent);
});

test("switching model in a chat with messages needs confirmation and starts a new chat", () => {
  let s = say(initialChat(), "慢一点", proposal("edited", "ok", ["a"]));
  assert.equal(s.exchanges[0].model, "claude-opus-5-5");
  assert.equal(modelChangeClears(s, "claude-sonnet-5-5"), true);
  assert.equal(modelChangeClears(s, "claude-opus-5-5"), false);
  // Not confirmed: nothing changes, the old model stays.
  assert.equal(chatReducer(s, { type: "setModel", model: "claude-sonnet-5-5" }), s);
  assert.equal(chatReducer(s, { type: "setModel", model: "claude-sonnet-5-5", confirmed: false }), s);
  // Confirmed: cleared like New chat, and the new model is used.
  const switched = chatReducer(s, { type: "setModel", model: "claude-sonnet-5-5", confirmed: true });
  assert.deepEqual(switched.exchanges, []);
  assert.equal(switched.model, "claude-sonnet-5-5");
  assert.ok(switched.generation > s.generation);
  s = say(switched, "再慢一点", proposal("clarify", "?"));
  assert.equal(s.exchanges[0].model, "claude-sonnet-5-5");
  assert.ok(nextExchangeId(s) > 2);
});

test("an answer to a request from before a model switch is dropped", () => {
  // Busy chats cannot switch; but an answer racing a clear-then-switch is still dropped.
  let s = chatReducer(say(initialChat(), "a", proposal("clarify", "?")), { type: "send", instruction: "b", sceneId: null, base: "T0" });
  const gen = s.generation;
  s = chatReducer(s, { type: "clear" });
  s = chatReducer(s, { type: "setModel", model: "claude-haiku-5-5" });
  assert.equal(s.model, "claude-haiku-5-5");
  const late = chatReducer(s, { type: "answer", id: 2, generation: gen, result: proposal("edited", "late", ["x"]) });
  assert.deepEqual(late.exchanges, []);
});

test("the model is kept across New chat and opening another document", () => {
  let s = chatReducer(initialChat(1), { type: "setModel", model: "claude-sonnet-5-5" });
  s = say(s, "慢一点", proposal("clarify", "?"));
  const cleared = chatReducer(s, { type: "clear" });
  assert.equal(cleared.model, "claude-sonnet-5-5");
  const other = chatReducer(s, { type: "openDocument", doc: 2 });
  assert.equal(other.model, "claude-sonnet-5-5");
  assert.deepEqual(other.exchanges, []);
});

test("the conversation marks decisions and skips failures and the message in flight", () => {
  let s = say(initialChat(), "加个转场", proposal("clarify", "要哪种转场？"));
  s = say(s, "淡入", proposal("edited", "第二段加了淡入。", ["scene-02: transition none -> fade 500 ms"]));
  s = run(s, { type: "accept", id: 2 });
  s = say(s, "第三段慢一点", proposal("edited", "延长。", ["scene-03: duration 4000 ms -> 5000 ms"]));
  s = run(s, { type: "reject", id: 3 });
  s = say(s, "第四段也是", proposal("edited", "延长。", ["scene-04: duration"]));
  s = say(s, "oops", failed);
  s = say(s, "nope", proposal("refused", "做不到"));
  s = chatReducer(s, { type: "send", instruction: "再快一点", sceneId: null, base: "T1" });
  const c = buildConversation(s.exchanges);
  assert.equal(c.dropped, 0);
  assert.deepEqual(
    c.turns.map((t) => [t.user, t.status, t.decision]),
    [
      ["加个转场", "clarify", undefined],
      ["淡入", "edited", "accepted"],
      ["第三段慢一点", "edited", "rejected"],
      ["第四段也是", "edited", "ignored"],
      ["nope", "refused", undefined],
    ],
  );
  assert.deepEqual(c.turns[1], {
    user: "淡入",
    status: "edited",
    reply: "第二段加了淡入。",
    changes: ["scene-02: transition none -> fade 500 ms"],
    decision: "accepted",
  });
  // No timeline in the conversation: the request carries the current one.
  assert.ok(!JSON.stringify(c).includes("fps"));
});

test("the conversation keeps the newest turns and counts the dropped ones", () => {
  let s = initialChat();
  for (let i = 1; i <= MAX_TURNS + 3; i += 1) s = say(s, `msg ${i}`, proposal("clarify", `q ${i}`));
  const c = buildConversation(s.exchanges);
  assert.equal(c.turns.length, MAX_TURNS);
  assert.equal(c.dropped, 3);
  assert.equal(c.turns[0].user, "msg 4");
  assert.equal(c.turns[MAX_TURNS - 1].user, `msg ${MAX_TURNS + 3}`);
});

test("the character cap drops older turns and clips long text", () => {
  let s = initialChat();
  const long = "长".repeat(5000);
  for (let i = 0; i < 4; i += 1) s = say(s, long, proposal("edited", long, Array.from({ length: 20 }, () => "x".repeat(500))));
  const c = buildConversation(s.exchanges);
  const chars = c.turns.reduce((n, t) => n + t.user.length + t.reply.length + t.changes.join("").length, 0);
  assert.ok(chars <= MAX_CHARS, `${chars}`);
  assert.ok(c.turns.length >= 1 && c.dropped === 4 - c.turns.length && c.dropped > 0);
  const t = c.turns[0];
  assert.ok(t.user.length <= 600 && t.user.endsWith("…"));
  assert.equal(t.changes.length, 9);
  assert.equal(t.changes[8], "(12 more changes)");
  assert.ok(t.changes[0].length <= 200);
  // Tighter caps passed in are honoured too.
  assert.equal(buildConversation(s.exchanges, 2).turns.length, 2);
});

test("dev endpoint: a request without a chat runs exactly the old command", () => {
  const base = { timelineJson: "{}", instruction: "make it slower", sceneId: "scene-01", model: "claude-sonnet-5-5" };
  const plain = parseArgs(JSON.stringify(base));
  assert.notEqual(typeof plain, "string");
  const old = ["edit", "--dry-run", "--json", "--public-dir", "/p", "--model", "claude-sonnet-5-5", "--scene", "scene-01", "--", "-", "make it slower"];
  assert.deepEqual(cliArgs(plain as Exclude<typeof plain, string>, "/p"), old);
  const empty = parseArgs(JSON.stringify({ ...base, conversation: { turns: [], dropped: 2 } }));
  assert.deepEqual(cliArgs(empty as Exclude<typeof empty, string>, "/p"), old);
  assert.equal((empty as { conversation?: unknown }).conversation, undefined);

  const chat = { turns: [{ user: "a", status: "clarify", reply: "b", changes: [] }], dropped: 1 };
  const withChat = parseArgs(JSON.stringify({ ...base, conversation: chat }));
  assert.deepEqual((withChat as { conversation?: unknown }).conversation, chat);
  assert.deepEqual(cliArgs(withChat as Exclude<typeof withChat, string>, "/p", "/tmp/c.json").slice(0, 11), [
    "edit", "--dry-run", "--json", "--public-dir", "/p", "--model", "claude-sonnet-5-5", "--scene", "scene-01", "--conversation", "/tmp/c.json",
  ]);
  assert.equal(parseArgs(JSON.stringify({ ...base, conversation: { turns: "x" } })), "conversation.turns must be a list");
  assert.match(String(parseArgs(JSON.stringify({ ...base, conversation: { turns: Array(13).fill({}) } }))), /more than 12/);
});

test("dev endpoint: an older CLI still serves single requests, not follow-ups", () => {
  const old = "--dry-run --json --public-dir --scene";
  assert.equal(helpHasEdit(old), true);
  assert.equal(helpHasChat(old), false);
  assert.equal(helpHasChat(`${old} --conversation`), true);
});
