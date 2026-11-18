// A pretend family for working on the UI in a plain browser (`npm run dev`), where there's no Rust side or hub.
//
//   http://localhost:1430/            signed in as Dad (a parent)
//   http://localhost:1430/?as=kid     signed in as Sam (a kid)
//   http://localhost:1430/?welcome    the first-launch screen
//
// Others reply now and then, and links get a preview, so the chat feels alive.

import { onShelf } from "./shelf";
import type { Account, Chat, Delta, Device, Member, Message, MessageBody, Note, OutgoingMedia, Status } from "./types";

type Listener = (d: Delta) => void;
const listeners = new Set<Listener>();

export function onMockDelta(handler: Listener): () => void {
  listeners.add(handler);
  return () => listeners.delete(handler);
}

function emit(delta: Delta) {
  setTimeout(() => listeners.forEach((l) => l(delta)), 0);
}

let seq = 0;
function id(): string {
  // ULID-shaped enough: sorts by time.
  seq += 1;
  return Date.now().toString(36).toUpperCase().padStart(10, "0") + String(seq).padStart(16, "0");
}

const params = new URLSearchParams(typeof location !== "undefined" ? location.search : "");

const members: Member[] = [
  { id: "M-DAD", name: "Dad", role: "parent", color: "sky", avatar: "🦁" },
  { id: "M-MOM", name: "Mom", role: "parent", color: "bubblegum", avatar: "🦋" },
  { id: "M-SAM", name: "Sam", role: "kid", color: "mint", avatar: "🦊" },
  { id: "M-AVA", name: "Ava", role: "kid", color: "grape", avatar: "🐰" },
];

const meId = params.has("as") && params.get("as") === "kid" ? "M-SAM" : "M-DAD";
let account: Account | null = params.has("welcome")
  ? null
  : {
      hubUrl: "http://192.168.1.20:8787",
      hubUrls: ["http://100.101.102.103:8787", "http://192.168.1.20:8787"],
      deviceId: "D-1",
      me: members.find((m) => m.id === meId)!,
      familyName: "The Lynx Family",
    };

const t0 = Date.now() - 1000 * 60 * 60 * 26;
const chats: Chat[] = [
  { id: "C-FAMILY", kind: "family", memberIds: members.map((m) => m.id), readUpTo: {}, createdAt: t0 },
  { id: "C-DAD-SAM", kind: "direct", memberIds: ["M-DAD", "M-SAM"], readUpTo: {}, createdAt: t0 },
];

function msg(
  chatId: string,
  senderId: string,
  body: MessageBody,
  minutesAgo: number,
  extra: Partial<Message> = {},
): Message {
  return {
    id: id(),
    chatId,
    senderId,
    body,
    createdAt: Date.now() - minutesAgo * 60_000,
    reactions: [],
    link: null,
    rev: 1,
    pending: false,
    error: null,
    ...extra,
  };
}

const DEMO_PICTURE = `<svg xmlns="http://www.w3.org/2000/svg" width="640" height="420" viewBox="0 0 640 420">
<defs><linearGradient id="s" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#7dd3fc"/><stop offset="1" stop-color="#e0f2fe"/></linearGradient></defs>
<rect width="640" height="420" fill="url(#s)"/><circle cx="520" cy="90" r="48" fill="#fde047"/>
<rect x="0" y="300" width="640" height="120" fill="#4ade80"/>
<rect x="150" y="190" width="70" height="110" fill="#a16207"/><circle cx="185" cy="170" r="70" fill="#16a34a"/>
<rect x="330" y="230" width="140" height="70" fill="#f87171"/><polygon points="320,232 400,170 480,232" fill="#b91c1c"/>
<rect x="385" y="260" width="30" height="40" fill="#fde68a"/></svg>`;

const media = new Map<string, Uint8Array>([["MEDIA-DEMO", new TextEncoder().encode(DEMO_PICTURE)]]);
/** Files the page staged for `send_media`, by their pretend path. */
const staged = new Map<string, Uint8Array>();

function typeOf(bytes: Uint8Array): string {
  if (bytes[0] === 0xff && bytes[1] === 0xd8) return "image/jpeg";
  if (bytes[0] === 0x89 && bytes[1] === 0x50) return "image/png";
  if (bytes[0] === 0x47 && bytes[1] === 0x49) return "image/gif";
  if (bytes[8] === 0x57 && bytes[9] === 0x45) return "image/webp";
  if (bytes[4] === 0x66 && bytes[5] === 0x74) return "video/mp4";
  if (bytes[0] === 0x1a && bytes[1] === 0x45) return "video/webm";
  if (bytes[0] === 0x3c) return "image/svg+xml";
  return "application/octet-stream";
}

const messages: Message[] = [
  msg("C-FAMILY", "M-MOM", { kind: "text", text: "Dinner at 6! 🍝" }, 60 * 25),
  msg("C-FAMILY", "M-SAM", { kind: "text", text: "I built a house in Minecraft!! look" }, 60 * 3),
  msg(
    "C-FAMILY",
    "M-SAM",
    { kind: "image", mediaId: "MEDIA-DEMO", width: 640, height: 420, caption: "my house" },
    60 * 3 - 1,
  ),
  msg("C-FAMILY", "M-DAD", { kind: "sticker", sticker: "wow" }, 60 * 2),
  msg(
    "C-FAMILY",
    "M-AVA",
    { kind: "text", text: "can we go here this weekend? https://www.lego.com/en-us/legoland" },
    30,
    {
      link: {
        url: "https://www.lego.com/en-us/legoland",
        title: "LEGOLAND Parks",
        description: "Rides, shows and building fun for the whole family.",
        site: "lego.com",
        imageMediaId: null,
      },
    },
  ),
  msg("C-DAD-SAM", "M-SAM", { kind: "text", text: "dad can you help me with my science project tonight" }, 90),
  msg("C-DAD-SAM", "M-DAD", { kind: "text", text: "Of course buddy! After dinner 👍" }, 85),
];
messages[3].reactions = [{ memberId: "M-SAM", emoji: "😂" }];

function block(
  kind: Note["blocks"][number]["kind"],
  text: string,
  i: number,
  extra: Partial<Note["blocks"][number]> = {},
) {
  return {
    id: id(),
    kind,
    text,
    checked: false,
    url: "",
    pos: i,
    updatedAt: t0,
    deleted: false,
    editedBy: "",
    ...extra,
  };
}

const notes: Note[] = [
  {
    id: id(),
    ownerId: "M-SAM",
    title: "Cool sites",
    emoji: "🌐",
    color: "sky",
    shared: true,
    blocks: [
      block("link", "Minecraft", 1, { url: "https://www.minecraft.net", editedBy: "M-SAM" }),
      block("link", "Scratch", 2, { url: "https://scratch.mit.edu", editedBy: "M-MOM" }),
      block("link", "NASA Kids", 3, { url: "https://spaceplace.nasa.gov", editedBy: "M-DAD" }),
    ],
    createdAt: t0,
    metaUpdatedAt: t0,
    rev: 1,
  },
  {
    id: id(),
    ownerId: "M-SAM",
    title: "Birthday wish list",
    emoji: "🎂",
    color: "bubblegum",
    shared: false,
    blocks: [
      block("check", "Lego space shuttle", 1),
      block("check", "Nerf blaster", 2, { checked: true }),
      block("check", "Books about volcanoes", 3),
    ],
    createdAt: t0,
    metaUpdatedAt: t0,
    rev: 1,
  },
  {
    id: id(),
    ownerId: "M-DAD",
    title: "Groceries",
    emoji: "🛒",
    color: "mint",
    shared: true,
    blocks: [
      block("check", "Milk", 1, { editedBy: "M-DAD" }),
      block("check", "Apples", 2, { editedBy: "M-AVA" }),
      block("check", "Pasta", 3, { checked: true, editedBy: "M-MOM" }),
    ],
    createdAt: t0,
    metaUpdatedAt: t0,
    rev: 1,
  },
];

// Things Sam sent Dad, for the Links & lists panel.
messages.push(
  msg(
    "C-DAD-SAM",
    "M-SAM",
    { kind: "text", text: "this volcano video is so cool https://www.youtube.com/watch?v=volcano" },
    60 * 20,
  ),
  msg(
    "C-DAD-SAM",
    "M-SAM",
    {
      kind: "note",
      noteId: notes[1].id,
      title: notes[1].title,
      emoji: notes[1].emoji,
      preview: ["Lego space shuttle", "Books about volcanoes"],
      shared: false,
    },
    60 * 19,
  ),
  msg(
    "C-DAD-SAM",
    "M-SAM",
    {
      kind: "note",
      noteId: notes[0].id,
      title: notes[0].title,
      emoji: notes[0].emoji,
      preview: ["Minecraft", "Scratch", "NASA Kids"],
      shared: true,
    },
    60 * 4,
  ),
  msg(
    "C-DAD-SAM",
    "M-SAM",
    {
      kind: "text",
      text: "can I get this? https://www.lego.com/en-us/product/space-shuttle and also https://scratch.mit.edu/projects/123",
    },
    120,
  ),
);
messages.sort((a, b) => a.createdAt - b.createdAt);

let autostart = false;

let status: Status = { state: account ? "online" : "signedOut", detail: null };

function visibleNotes(): Note[] {
  if (!account) return [];
  return notes.filter((n) => n.ownerId === account!.me.id || n.shared);
}

function visibleChats(): Chat[] {
  if (!account) return [];
  return chats.filter((c) => c.memberIds.includes(account!.me.id));
}

function snapshot(): Delta {
  const mine = new Set(visibleChats().map((c) => c.id));
  return {
    reset: true,
    account,
    members: account ? members : [],
    chats: visibleChats(),
    messages: messages.filter((m) => mine.has(m.chatId)),
    notes: visibleNotes(),
    status,
  };
}

const REPLIES = ["That's so cool!", "Love it ❤️", "Haha 😂", "Wow!!", "Tell me more!", "👍", "Can't wait!"];

function someoneReplies(chatId: string) {
  const chat = chats.find((c) => c.id === chatId);
  if (!chat || !account) return;
  const others = chat.memberIds.filter((m) => m !== account!.me.id);
  const who = others[Math.floor(Math.random() * others.length)];
  setTimeout(() => {
    const body: MessageBody =
      Math.random() < 0.25
        ? { kind: "sticker", sticker: ["love", "lol", "wow", "party"][Math.floor(Math.random() * 4)] }
        : { kind: "text", text: REPLIES[Math.floor(Math.random() * REPLIES.length)] };
    const m = msg(chatId, who, body, 0);
    messages.push(m);
    emit({ messages: [m] });
  }, 1800);
}

function sent(chatId: string, body: MessageBody): Message {
  const pending = { ...msg(chatId, account!.me.id, body, 0), pending: true };
  setTimeout(() => {
    const done = { ...pending, pending: false };
    messages.push(done);
    emit({ messages: [done] });
    const url = body.kind === "text" ? body.text.match(/https?:\/\/\S+/)?.[0] : undefined;
    if (url) {
      setTimeout(() => {
        const site = new URL(url).hostname.replace(/^www\./, "");
        done.link = {
          url,
          title: `A page on ${site}`,
          description: "A preview fetched by the family hub.",
          site,
          imageMediaId: null,
        };
        emit({ messages: [{ ...done }] });
      }, 700);
    }
    if (Math.random() < 0.6) someoneReplies(chatId);
  }, 350);
  return pending;
}

function wait(ms = 120) {
  return new Promise((r) => setTimeout(r, ms));
}

export async function mockInvoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  await wait(40);
  const a = args as Record<string, never>;
  switch (cmd) {
    case "snapshot":
      return snapshot() as T;
    case "hub_hello":
      await wait(400);
      if (!String(a.url).trim()) throw "Type the family hub's address.";
      return {
        app: "kitchat-hub",
        protocol: 1,
        familyName: "The Lynx Family",
        needsSetup: String(a.url).includes("new"),
      } as T;
    case "setup_family":
    case "join_family": {
      await wait(500);
      const code = String(cmd === "setup_family" ? a.setupCode : a.code);
      if (code.replace(/\W/g, "").length < 6)
        throw "That code didn't work. Codes work once, for 10 minutes: ask a parent for a new one.";
      account = {
        hubUrl: String(a.url),
        hubUrls: [String(a.url), ...((a.alsoUrls as string[] | undefined) ?? [])],
        deviceId: "D-NEW",
        me: members.find((m) => m.id === meId)!,
        familyName: "The Lynx Family",
      };
      status = { state: "online", detail: null };
      emit(snapshot());
      return undefined as T;
    }
    case "sign_out":
      account = null;
      status = { state: "signedOut", detail: null };
      emit(snapshot());
      return undefined as T;
    case "send_message":
      return sent(String(a.chatId), a.body as MessageBody) as T;
    case "stage_file": {
      const path = `staged-${id()}`;
      staged.set(path, new Uint8Array(args as unknown as Uint8Array));
      return path as T;
    }
    case "send_media": {
      const out: Message[] = [];
      for (const item of a.items as OutgoingMedia[]) {
        const original = staged.get(item.path);
        const preview = item.previewPath ? staged.get(item.previewPath) : undefined;
        if (!original) throw "That file isn't one this app made.";
        const keep = (bytes: Uint8Array) => {
          const mediaId = id();
          media.set(mediaId, bytes);
          return mediaId;
        };
        if (item.kind === "video") {
          out.push(
            sent(String(a.chatId), {
              kind: "video",
              videoMediaId: keep(original),
              posterMediaId: preview ? keep(preview) : null,
              width: item.width,
              height: item.height,
              durationMs: item.durationMs,
              bytes: original.length,
              caption: item.caption,
            }),
          );
        } else {
          out.push(
            sent(String(a.chatId), {
              kind: "image",
              mediaId: keep(preview ?? original),
              width: item.width,
              height: item.height,
              caption: item.caption,
              fullMediaId: preview ? keep(original) : null,
              fullBytes: preview ? original.length : 0,
            }),
          );
        }
      }
      return out as T;
    }
    case "pick_media":
      return null as T;
    case "retry_message":
    case "discard_message":
    case "set_foreground":
    case "set_unread":
    case "log":
      return undefined as T;
    case "media_file": {
      const bytes = media.get(String(a.id));
      if (!bytes) throw "That picture is gone.";
      return URL.createObjectURL(new Blob([bytes.slice()], { type: typeOf(bytes) })) as T;
    }
    case "save_media": {
      // The browser's own download.
      const bytes = media.get(String(a.id));
      if (!bytes) throw "That picture is gone.";
      const type = typeOf(bytes);
      const ext =
        {
          "image/jpeg": "jpg",
          "image/png": "png",
          "image/gif": "gif",
          "image/webp": "webp",
          "video/mp4": "mp4",
          "video/webm": "webm",
          "image/svg+xml": "svg",
        }[type] ?? "bin";
      const name = `KitChat-${a.id}.${ext}`;
      const link = document.createElement("a");
      link.href = URL.createObjectURL(new Blob([bytes.slice()], { type }));
      link.download = name;
      link.click();
      setTimeout(() => URL.revokeObjectURL(link.href), 10_000);
      return { place: "downloads", name } as T;
    }
    case "older_messages":
      return [] as T;
    case "mark_read": {
      const chat = chats.find((c) => c.id === a.chatId);
      if (chat && account) {
        chat.readUpTo = { ...chat.readUpTo, [account.me.id]: String(a.messageId) };
        emit({ chats: [{ ...chat }] });
      }
      return undefined as T;
    }
    case "react": {
      const m = messages.find((x) => x.id === a.messageId);
      if (m && account) {
        m.reactions = m.reactions.filter((r) => r.memberId !== account!.me.id);
        if (a.emoji) m.reactions.push({ memberId: account.me.id, emoji: String(a.emoji) });
        emit({ messages: [{ ...m }] });
      }
      return undefined as T;
    }
    case "open_direct": {
      const me = account!.me.id;
      let chat = chats.find(
        (c) => c.kind === "direct" && c.memberIds.includes(me) && c.memberIds.includes(String(a.memberId)),
      );
      if (!chat) {
        chat = { id: id(), kind: "direct", memberIds: [me, String(a.memberId)], readUpTo: {}, createdAt: Date.now() };
        chats.push(chat);
        emit({ chats: [chat] });
      }
      return chat as T;
    }
    case "save_note": {
      const note = a.note as unknown as Note;
      const at = notes.findIndex((n) => n.id === note.id);
      if (at >= 0) notes[at] = note;
      else notes.push(note);
      emit({ notes: [note] });
      return undefined as T;
    }
    case "delete_note": {
      const at = notes.findIndex((n) => n.id === a.id);
      if (at >= 0) notes.splice(at, 1);
      emit({ deletedNotes: [String(a.id)] });
      return undefined as T;
    }
    case "add_member": {
      const m = { ...(a.member as unknown as Omit<Member, "id">), id: id() };
      members.push(m);
      chats[0].memberIds.push(m.id);
      emit({ members: [...members], chats: [{ ...chats[0] }] });
      return m as T;
    }
    case "update_member": {
      const m = members.find((x) => x.id === a.id)!;
      Object.assign(m, a.update);
      if (account && account.me.id === m.id) account = { ...account, me: { ...m } };
      emit({ members: [...members], account });
      return m as T;
    }
    case "join_code":
      await wait(300);
      return {
        code: "K7P3QX",
        expiresAt: Date.now() + 10 * 60_000,
        hubUrl: account?.hubUrls[0] ?? "",
        hubUrls: account?.hubUrls ?? [],
        link: "kitchat://join?hub=http%3A%2F%2F192.168.1.20%3A8787&code=K7P3QX",
        qrSvg: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 21 21" width="240" height="240"><rect width="21" height="21" fill="#fff"/><path d="M1 1h7v7H1zM13 1h7v7h-7zM1 13h7v7H1z" fill="none" stroke="#1f1147" stroke-width="1"/><path d="M3 3h3v3H3zM15 3h3v3h-3zM3 15h3v3H3zM10 2h1v2h-1zM10 10h2v1h-2zM14 12h2v2h-2zM12 16h1v3h-1zM17 15h2v1h-2z" fill="#1f1147"/></svg>`,
      } as T;
    case "devices":
      return [
        { id: "D-1", memberId: "M-DAD", name: "Desktop", createdAt: t0, lastSeen: Date.now() },
        { id: "D-2", memberId: "M-SAM", name: "Sam's tablet", createdAt: t0, lastSeen: Date.now() - 3600_000 },
        { id: "D-3", memberId: "M-MOM", name: "Mom's phone", createdAt: t0, lastSeen: Date.now() - 600_000 },
      ] satisfies Device[] as T;
    case "revoke_device":
      return undefined as T;
    case "take_shared":
      return { items: [] } as T;
    case "scan_qr":
      throw "Scanning needs a camera. Type the code instead.";
    case "device_name_hint":
      return "My computer" as T;
    case "autostart_enabled":
      return autostart as T;
    case "set_autostart":
      autostart = Boolean(a.on);
      return autostart as T;
    case "chat_shelf": {
      const all = messages
        .filter((m) => m.chatId === a.chatId && onShelf(m))
        .sort((x, y) => y.createdAt - x.createdAt || (y.id < x.id ? -1 : 1));
      const from = a.before ? all.findIndex((m) => m.id === a.before) + 1 : 0;
      return all.slice(from, from + 100) as T;
    }
    case "set_hub_addresses": {
      await wait(400);
      const urls = (a.urls as string[])
        .map((url) => url.trim())
        .filter(Boolean)
        .map((url) => (/^https?:\/\//.test(url) ? url : `http://${url}`));
      if (!urls.length) throw "Type the family hub's address.";
      account = { ...account!, hubUrl: urls[0], hubUrls: urls };
      emit({ account });
      return { urls, unreachable: [] } as T;
    }
    default:
      throw `mock backend: no ${cmd}`;
  }
}
