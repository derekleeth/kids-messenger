// What the page shows, kept in one place and updated from the Rust side's deltas (src-tauri/src/engine.rs).

import { useSyncExternalStore } from "react";
import type { Account, Chat, Delta, Member, Message, Note, Status } from "./types";

export interface AppState {
  loaded: boolean;
  account: Account | null;
  members: Member[];
  chats: Chat[];
  /** Messages by chat, oldest first. */
  messages: Record<string, Message[]>;
  notes: Note[];
  status: Status;
}

export const emptyState: AppState = {
  loaded: false,
  account: null,
  members: [],
  chats: [],
  messages: {},
  notes: [],
  status: { state: "signedOut", detail: null },
};

function byTime(a: Message, b: Message) {
  // Waiting messages go after sent ones; within each, by time then ID.
  if (a.pending !== b.pending) return a.pending ? 1 : -1;
  return a.createdAt - b.createdAt || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
}

/** Puts messages into their chats, replacing ones with the same ID (a sent message replaces its outbox copy). */
export function mergeMessages(current: Record<string, Message[]>, incoming: Message[]): Record<string, Message[]> {
  if (incoming.length === 0) return current;
  const next = { ...current };
  const touched = new Set<string>();
  for (const message of incoming) {
    const list = touched.has(message.chatId) ? next[message.chatId] : [...(next[message.chatId] ?? [])];
    touched.add(message.chatId);
    const at = list.findIndex((m) => m.id === message.id);
    if (at >= 0) list[at] = message;
    else list.push(message);
    next[message.chatId] = list;
  }
  for (const chatId of touched) next[chatId].sort(byTime);
  return next;
}

function upsert<T extends { id: string }>(list: T[], items: T[]): T[] {
  if (items.length === 0) return list;
  const next = [...list];
  for (const item of items) {
    const at = next.findIndex((x) => x.id === item.id);
    if (at >= 0) next[at] = item;
    else next.push(item);
  }
  return next;
}

export function applyDelta(state: AppState, delta: Delta): AppState {
  const base: AppState = delta.reset ? { ...emptyState } : state;
  const deleted = new Set(delta.deletedNotes ?? []);
  return {
    loaded: true,
    account: delta.account !== undefined ? delta.account : base.account,
    members: delta.members ?? base.members,
    chats: upsert(base.chats, delta.chats ?? []),
    messages: mergeMessages(base.messages, delta.messages ?? []),
    notes: upsert(base.notes, delta.notes ?? []).filter((n) => !deleted.has(n.id)),
    status: delta.status ?? base.status,
  };
}

/** Removes one message (an unsent one the person discarded). */
export function dropMessage(state: AppState, chatId: string, id: string): AppState {
  const list = state.messages[chatId];
  if (!list) return state;
  return { ...state, messages: { ...state.messages, [chatId]: list.filter((m) => m.id !== id) } };
}

// ---- A tiny store the components subscribe to ----

let current: AppState = emptyState;
const listeners = new Set<() => void>();

export function getState() {
  return current;
}

export function setState(update: (s: AppState) => AppState) {
  const next = update(current);
  if (next === current) return;
  current = next;
  for (const l of listeners) l();
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useAppState<T>(select: (s: AppState) => T): T {
  return useSyncExternalStore(subscribe, () => select(current));
}

// ---- Derived ----

export function memberById(members: Member[], id: string): Member | undefined {
  return members.find((m) => m.id === id);
}

/** Messages from others after what `me` has seen. */
export function unreadCount(chat: Chat, messages: Message[] | undefined, me: string): number {
  if (!messages) return 0;
  const seen = chat.readUpTo[me] ?? "";
  return messages.filter((m) => !m.pending && m.senderId !== me && m.id > seen).length;
}

/** Who else in the chat has seen a message, by member ID. */
export function readBy(chat: Chat, messageId: string, me: string): string[] {
  return chat.memberIds.filter((id) => id !== me && (chat.readUpTo[id] ?? "") >= messageId);
}

/** The other person in a direct chat. */
export function otherMember(chat: Chat, me: string): string | undefined {
  return chat.memberIds.find((id) => id !== me);
}

export function lastMessage(messages: Message[] | undefined): Message | undefined {
  return messages && messages.length > 0 ? messages[messages.length - 1] : undefined;
}

/** A one-line summary of a message, for chat lists and notifications. */
export function summary(m: Message): string {
  switch (m.body.kind) {
    case "text":
      return m.body.text;
    case "image":
      return m.body.caption ? `📷 ${m.body.caption}` : "📷 Picture";
    case "video":
      return m.body.caption ? `🎬 ${m.body.caption}` : "🎬 Video";
    case "sticker":
      return "Sticker";
    case "note":
      return `📝 ${m.body.title || "A list"}`;
  }
}
