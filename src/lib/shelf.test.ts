import { describe, expect, it } from "vitest";
import { linkEntries, matches, onShelf } from "./shelf";
import type { Message, MessageBody } from "./types";

function m(id: string, body: MessageBody, link: Message["link"] = null): Message {
  return {
    id,
    chatId: "c",
    senderId: "s",
    body,
    createdAt: 0,
    reactions: [],
    link,
    rev: 1,
    pending: false,
    error: null,
  };
}

describe("shelf", () => {
  it("keeps links and shared lists only", () => {
    expect(onShelf(m("a", { kind: "text", text: "look www.example.com!" }))).toBe(true);
    expect(onShelf(m("b", { kind: "text", text: "http is a protocol" }))).toBe(false);
    expect(onShelf(m("c", { kind: "sticker", sticker: "wow" }))).toBe(false);
    expect(onShelf(m("d", { kind: "note", noteId: "n", title: "List", emoji: "📝", preview: [], shared: true }))).toBe(
      true,
    );
  });

  it("makes one entry per distinct link, with the preview on its own link", () => {
    const preview = {
      url: "https://b.example/x",
      title: "Bee",
      description: "",
      site: "b.example",
      imageMediaId: null,
    };
    const entries = linkEntries([
      m("a", { kind: "text", text: "https://a.example and https://b.example/x and https://a.example" }, preview),
    ]);
    expect(entries.map((e) => [e.url, e.site, e.title])).toEqual([
      ["https://a.example", "a.example", ""],
      ["https://b.example/x", "b.example", "Bee"],
    ]);
  });

  it("matches every word, in any field, ignoring case", () => {
    expect(matches("lego PARK", "LEGOLAND Parks", "lego.com")).toBe(true);
    expect(matches("lego zoo", "LEGOLAND Parks", "lego.com")).toBe(false);
    expect(matches("  ", "anything")).toBe(true);
  });
});
