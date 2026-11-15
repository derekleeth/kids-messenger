// The same shapes as crates/kit-protocol and src-tauri (camelCase on the wire).

export type Role = "parent" | "kid";

export interface Member {
  id: string;
  name: string;
  role: Role;
  color: string;
  avatar: string;
}

export interface Device {
  id: string;
  memberId: string;
  name: string;
  createdAt: number;
  lastSeen: number;
}

export interface Chat {
  id: string;
  kind: "family" | "direct";
  memberIds: string[];
  readUpTo: Record<string, string>;
  createdAt: number;
}

export type MessageBody =
  | { kind: "text"; text: string }
  | { kind: "image"; mediaId: string; width: number; height: number; caption: string }
  | { kind: "sticker"; sticker: string }
  | { kind: "note"; noteId: string; title: string; emoji: string; preview: string[]; shared: boolean };

export interface Reaction {
  memberId: string;
  emoji: string;
}

export interface LinkPreview {
  url: string;
  title: string;
  description: string;
  site: string;
  imageMediaId: string | null;
}

export interface Message {
  id: string;
  chatId: string;
  senderId: string;
  body: MessageBody;
  createdAt: number;
  reactions: Reaction[];
  link: LinkPreview | null;
  rev: number;
  pending: boolean;
  error: string | null;
}

export type BlockKind = "heading" | "text" | "check" | "bullet" | "link";

export interface Block {
  id: string;
  kind: BlockKind;
  text: string;
  checked: boolean;
  url: string;
  pos: number;
  updatedAt: number;
  deleted: boolean;
}

export interface Note {
  id: string;
  ownerId: string;
  title: string;
  emoji: string;
  color: string;
  shared: boolean;
  blocks: Block[];
  createdAt: number;
  metaUpdatedAt: number;
  rev: number;
}

export interface Account {
  hubUrl: string;
  deviceId: string;
  me: Member;
  familyName: string;
}

export type ConnState = "signedOut" | "connecting" | "online" | "offline";

export interface Status {
  state: ConnState;
  detail: string | null;
}

/** What the Rust side sends when something changes (src-tauri/src/engine.rs `Delta`). */
export interface Delta {
  reset?: boolean;
  account?: Account | null;
  members?: Member[];
  chats?: Chat[];
  messages?: Message[];
  notes?: Note[];
  deletedNotes?: string[];
  status?: Status;
}

export interface Hello {
  app: string;
  protocol: number;
  familyName: string;
  needsSetup: boolean;
}

export interface JoinCode {
  code: string;
  expiresAt: number;
  hubUrl: string;
  link: string;
  qrSvg: string;
}

export interface SharedItem {
  kind: "image" | "text";
  path: string;
  mime: string;
  name: string;
  text: string;
}

export const COLORS = ["sun", "sky", "bubblegum", "mint", "grape", "tangerine"] as const;
export type ColorName = (typeof COLORS)[number];

export const STICKERS = [
  "wave",
  "love",
  "lol",
  "wow",
  "sleepy",
  "party",
  "thinking",
  "thumbsup",
  "hug",
  "cool",
] as const;
export type StickerName = (typeof STICKERS)[number];
