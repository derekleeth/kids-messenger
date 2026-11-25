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
  | {
      kind: "image";
      /** What the chat shows: at most 1920 pixels across. */
      mediaId: string;
      width: number;
      height: number;
      caption: string;
      /** The photo at full size, opened when it's tapped. */
      fullMediaId?: string | null;
      fullBytes?: number;
    }
  | {
      kind: "video";
      videoMediaId: string;
      posterMediaId?: string | null;
      width: number;
      height: number;
      durationMs: number;
      bytes: number;
      caption: string;
    }
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
  /** The member who last changed this line. Empty for lines from before the hub kept track. */
  editedBy: string;
  /** The page the line links to, fetched by the hub. Only shown while it's still for the line's link (`linkPreview`). */
  link: LinkPreview | null;
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
  /** The address in use, or last used. */
  hubUrl: string;
  /** Every address for the hub, in the order they're tried. */
  hubUrls: string[];
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
  hubUrls: string[];
  link: string;
  qrSvg: string;
}

export interface HubAddresses {
  urls: string[];
  /** Saved, but couldn't be reached from here just now. */
  unreachable: string[];
}

/** A photo or video ready to send (copied into the app's cache, with a small preview), or text shared in. */
export interface SharedItem {
  kind: "image" | "video" | "text";
  path: string;
  previewPath?: string | null;
  width: number;
  height: number;
  durationMs: number;
  mime: string;
  text: string;
}

export interface Incoming {
  items: SharedItem[];
  problem?: string | null;
}

/** What `send_media` takes for each photo or video. */
export interface OutgoingMedia {
  kind: "image" | "video";
  path: string;
  previewPath: string | null;
  width: number;
  height: number;
  durationMs: number;
  caption: string;
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
