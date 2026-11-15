// Editing notes and lists. Every change stamps the block (or the note's title and colour) with the time, which is
// how the hub combines edits from two people on a family list (crates/kit-protocol/src/notes.rs). Deleting a line
// leaves it marked deleted rather than removing it, so it can't come back from someone else's older copy.

import type { Block, BlockKind, Note } from "./types";
import { ulid } from "./ulid";

export interface Template {
  id: string;
  label: string;
  title: string;
  emoji: string;
  color: string;
  blocks: { kind: BlockKind; text: string }[];
}

export const TEMPLATES: Template[] = [
  { id: "list", label: "My list", title: "My list", emoji: "📝", color: "sun", blocks: [{ kind: "check", text: "" }] },
  {
    id: "sites",
    label: "Cool sites",
    title: "Cool sites",
    emoji: "🌐",
    color: "sky",
    blocks: [{ kind: "link", text: "" }],
  },
  {
    id: "idea",
    label: "Big idea!",
    title: "My big idea",
    emoji: "💡",
    color: "grape",
    blocks: [
      { kind: "heading", text: "What is it?" },
      { kind: "text", text: "" },
      { kind: "heading", text: "What I need" },
      { kind: "bullet", text: "" },
    ],
  },
  { id: "blank", label: "Blank note", title: "", emoji: "✏️", color: "mint", blocks: [{ kind: "text", text: "" }] },
];

export const NOTE_EMOJI = [
  "📝",
  "🌐",
  "💡",
  "🎂",
  "🛒",
  "🎮",
  "📚",
  "🚀",
  "🎨",
  "⚽",
  "🦖",
  "🌋",
  "🎁",
  "🐱",
  "⭐",
  "🏰",
];

function block(kind: BlockKind, text: string, pos: number, now: number): Block {
  return { id: ulid(now), kind, text, checked: false, url: "", pos, updatedAt: now, deleted: false };
}

export function newNote(ownerId: string, template: Template, now = Date.now()): Note {
  return {
    id: ulid(now),
    ownerId,
    title: template.title,
    emoji: template.emoji,
    color: template.color,
    shared: false,
    blocks: template.blocks.map((b, i) => block(b.kind, b.text, i + 1, now)),
    createdAt: now,
    metaUpdatedAt: now,
    rev: 0,
  };
}

export function visibleBlocks(note: Note): Block[] {
  return note.blocks
    .filter((b) => !b.deleted)
    .sort((a, b) => a.pos - b.pos || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
}

export function setMeta(
  note: Note,
  patch: Partial<Pick<Note, "title" | "emoji" | "color" | "shared">>,
  now = Date.now(),
): Note {
  return { ...note, ...patch, metaUpdatedAt: Math.max(now, note.metaUpdatedAt + 1) };
}

export function updateBlock(
  note: Note,
  id: string,
  patch: Partial<Pick<Block, "text" | "checked" | "url" | "kind">>,
  now = Date.now(),
): Note {
  return {
    ...note,
    blocks: note.blocks.map((b) => (b.id === id ? { ...b, ...patch, updatedAt: Math.max(now, b.updatedAt + 1) } : b)),
  };
}

/** Adds a line after `afterId` (or at the end). Returns the note and the new line's ID. */
export function insertAfter(note: Note, afterId: string | null, kind: BlockKind, now = Date.now()): [Note, string] {
  const blocks = visibleBlocks(note);
  const at = afterId ? blocks.findIndex((b) => b.id === afterId) : blocks.length - 1;
  const before = at >= 0 ? blocks[at].pos : 0;
  const after = at + 1 < blocks.length ? blocks[at + 1].pos : before + 2;
  const added = block(kind, "", (before + after) / 2, now);
  return [{ ...note, blocks: [...note.blocks, added] }, added.id];
}

export function removeBlock(note: Note, id: string, now = Date.now()): Note {
  return {
    ...note,
    blocks: note.blocks.map((b) =>
      b.id === id ? { ...b, deleted: true, updatedAt: Math.max(now, b.updatedAt + 1) } : b,
    ),
  };
}

/** Moves a line up (-1) or down (+1) past its neighbour. */
export function moveBlock(note: Note, id: string, dir: -1 | 1, now = Date.now()): Note {
  const blocks = visibleBlocks(note);
  const at = blocks.findIndex((b) => b.id === id);
  const target = at + dir;
  if (at < 0 || target < 0 || target >= blocks.length) return note;
  // The new position sits between the neighbour and the one beyond it.
  const beyond = target + dir;
  const edge = blocks[target].pos + dir * 2;
  const far = beyond >= 0 && beyond < blocks.length ? blocks[beyond].pos : edge;
  const pos = (blocks[target].pos + far) / 2;
  return {
    ...note,
    blocks: note.blocks.map((b) => (b.id === id ? { ...b, pos, updatedAt: Math.max(now, b.updatedAt + 1) } : b)),
  };
}

export function progress(note: Note): { done: number; total: number } {
  const checks = visibleBlocks(note).filter((b) => b.kind === "check" && (b.text.trim() || b.checked));
  return { done: checks.filter((b) => b.checked).length, total: checks.length };
}

/** The first few lines, for a card. */
export function previewLines(note: Note, lines = 4): string[] {
  return visibleBlocks(note)
    .map((b) => {
      if (b.kind === "check") return `${b.checked ? "☑" : "☐"} ${b.text}`;
      if (b.kind === "bullet") return `• ${b.text}`;
      if (b.kind === "link") return b.text || b.url;
      return b.text;
    })
    .filter((l) => l.replace(/[☑☐•]/g, "").trim())
    .slice(0, lines);
}

export function isEmpty(note: Note): boolean {
  return !note.title.trim() && visibleBlocks(note).every((b) => !b.text.trim() && !b.url.trim());
}
