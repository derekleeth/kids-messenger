import { describe, expect, it } from "vitest";
import { fitWithin } from "./images";
import { applyDelta, emptyState, mergeMessages, readBy, unreadCount } from "./state";
import type { Chat, Message } from "./types";

function msg(id: string, createdAt: number, extra: Partial<Message> = {}): Message {
  return {
    id,
    chatId: "c1",
    senderId: "kid",
    body: { kind: "text", text: id },
    createdAt,
    reactions: [],
    link: null,
    rev: 0,
    pending: false,
    error: null,
    ...extra,
  };
}

const chat: Chat = { id: "c1", kind: "family", memberIds: ["dad", "kid"], readUpTo: { dad: "B" }, createdAt: 0 };

describe("state", () => {
  it("keeps messages in time order, waiting ones last", () => {
    const merged = mergeMessages({}, [msg("C", 30), msg("Z", 5, { pending: true }), msg("A", 10)]);
    expect(merged.c1.map((m) => m.id)).toEqual(["A", "C", "Z"]);
  });

  it("replaces the outbox copy when the sent message arrives", () => {
    const before = mergeMessages({}, [msg("A", 10, { pending: true })]);
    const after = mergeMessages(before, [msg("A", 10, { rev: 7 })]);
    expect(after.c1).toHaveLength(1);
    expect(after.c1[0].pending).toBe(false);
    expect(before.c1[0].pending).toBe(true); // not changed in place
  });

  it("applies deltas, with reset starting over", () => {
    let s = applyDelta(emptyState, { chats: [chat], messages: [msg("A", 1)] });
    expect(s.loaded).toBe(true);
    s = applyDelta(s, { notes: [{ id: "n1" } as never] });
    expect(s.notes).toHaveLength(1);
    s = applyDelta(s, { deletedNotes: ["n1"] });
    expect(s.notes).toHaveLength(0);
    s = applyDelta(s, { reset: true, account: null });
    expect(s.chats).toHaveLength(0);
    expect(s.messages).toEqual({});
  });

  it("counts unread messages from others", () => {
    const list = [
      msg("A", 1),
      msg("B", 2),
      msg("C", 3),
      msg("D", 4, { senderId: "dad" }),
      msg("E", 5, { pending: true }),
    ];
    expect(unreadCount(chat, list, "dad")).toBe(1);
  });

  it("knows who else has seen a message", () => {
    expect(readBy(chat, "A", "kid")).toEqual(["dad"]);
    expect(readBy(chat, "B", "kid")).toEqual(["dad"]);
    expect(readBy(chat, "C", "kid")).toEqual([]);
    expect(readBy(chat, "A", "dad")).toEqual([]); // not counting yourself
  });

  it("shrinks pictures keeping their shape", () => {
    expect(fitWithin(4000, 3000)).toEqual({ width: 1920, height: 1440 });
    expect(fitWithin(800, 600)).toEqual({ width: 800, height: 600 });
    expect(fitWithin(10000, 1, 1920)).toEqual({ width: 1920, height: 1 });
  });
});
