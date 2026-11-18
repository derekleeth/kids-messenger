// Calls into the Rust side (src-tauri/src/commands.rs). In a plain browser (`npm run dev`) the same calls go to an
// in-memory pretend family (mockBackend.ts), so the UI can be worked on without the app or a hub.

import type {
  Chat,
  Delta,
  Device,
  Hello,
  HubAddresses,
  Incoming,
  JoinCode,
  Member,
  Message,
  MessageBody,
  Note,
  OutgoingMedia,
  Role,
  SharedItem,
} from "./types";

export const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

type Invoke = <T>(
  cmd: string,
  args?: Record<string, unknown>,
  options?: { headers: Record<string, string> },
) => Promise<T>;

let invokeImpl: Promise<Invoke> | null = null;

function backend(): Promise<Invoke> {
  if (!invokeImpl) {
    invokeImpl = isTauri
      ? import("@tauri-apps/api/core").then((m) => m.invoke as Invoke)
      : import("./mockBackend").then((m) => m.mockInvoke as Invoke);
  }
  return invokeImpl;
}

async function call<T>(cmd: string, args?: Record<string, unknown>, options?: { headers: Record<string, string> }) {
  const invoke = await backend();
  return invoke<T>(cmd, args, options);
}

/** Calls `handler` with each `kit://<name>` event from the Rust side; returns a function that stops listening. */
export async function onEvent<T>(name: string, handler: (payload: T) => void): Promise<() => void> {
  if (!isTauri) return () => {};
  const { listen } = await import("@tauri-apps/api/event");
  return listen<T>(`kit://${name}`, (e) => handler(e.payload));
}

export const isAndroid = typeof navigator !== "undefined" && /Android/i.test(navigator.userAgent);

/** Calls `handler` with every change; returns a function that stops listening. */
export async function onDelta(handler: (delta: Delta) => void): Promise<() => void> {
  if (isTauri) {
    const { listen } = await import("@tauri-apps/api/event");
    return listen<Delta>("kit://delta", (e) => handler(e.payload));
  }
  const mock = await import("./mockBackend");
  return mock.onMockDelta(handler);
}

export const api = {
  snapshot: () => call<Delta>("snapshot"),
  hubHello: (url: string) => call<Hello>("hub_hello", { url }),
  setupFamily: (a: { url: string; setupCode: string; familyName: string; name: string; deviceName: string }) =>
    call<void>("setup_family", a),
  joinFamily: (a: { url: string; alsoUrls: string[]; code: string; deviceName: string }) =>
    call<void>("join_family", a),
  signOut: () => call<void>("sign_out"),
  sendMessage: (chatId: string, body: MessageBody) => call<Message>("send_message", { chatId, body }),
  /** Saves bytes the page has (a shrunk picture, a file picked on desktop) for `sendMedia`; returns the path. */
  stageFile: (bytes: Uint8Array) => call<string>("stage_file", bytes as unknown as Record<string, unknown>),
  sendMedia: (chatId: string, items: OutgoingMedia[]) => call<Message[]>("send_media", { chatId, items }),
  /** Android's photo picker; `null` elsewhere. */
  pickMedia: () => call<SharedItem[] | null>("pick_media"),
  retryMessage: (id: string) => call<void>("retry_message", { id }),
  discardMessage: (id: string) => call<void>("discard_message", { id }),
  /** Where a photo or video is on this device, downloading it first if needed. */
  mediaFile: (id: string) => call<string>("media_file", { id }),
  /** Saves a photo or video to the Downloads folder (computer) or the gallery (Android), downloading it if needed. */
  saveMedia: (id: string) => call<{ place: "downloads" | "gallery"; name: string }>("save_media", { id }),
  olderMessages: (chatId: string, before: string) => call<Message[]>("older_messages", { chatId, before }),
  markRead: (chatId: string, messageId: string) => call<void>("mark_read", { chatId, messageId }),
  react: (messageId: string, emoji: string) => call<void>("react", { messageId, emoji }),
  openDirect: (memberId: string) => call<Chat>("open_direct", { memberId }),
  saveNote: (note: Note) => call<void>("save_note", { note }),
  deleteNote: (id: string) => call<void>("delete_note", { id }),
  addMember: (member: { name: string; role: Role; color: string; avatar: string }) =>
    call<Member>("add_member", { member }),
  updateMember: (id: string, update: { name?: string; color?: string; avatar?: string }) =>
    call<Member>("update_member", { id, update }),
  joinCode: (memberId: string) => call<JoinCode>("join_code", { memberId }),
  devices: () => call<Device[]>("devices"),
  revokeDevice: (id: string) => call<void>("revoke_device", { id }),
  setForeground: (foreground: boolean) => call<void>("set_foreground", { foreground }),
  takeShared: () => call<Incoming>("take_shared"),
  scanQr: () => call<string | null>("scan_qr"),
  deviceNameHint: () => call<string>("device_name_hint"),
  log: (level: "info" | "warn" | "error", message: string) => call<void>("log", { level, message }),
  /** The unread count, for the taskbar button and the tray (desktop). */
  setUnread: (count: number) => call<void>("set_unread", { count }),
  autostartEnabled: () => call<boolean>("autostart_enabled"),
  setAutostart: (on: boolean) => call<boolean>("set_autostart", { on }),
  /** Links and shared lists in a chat, newest first (older than `before`, when given). */
  chatShelf: (chatId: string, before?: string) => call<Message[]>("chat_shelf", { chatId, before: before ?? null }),
  /** The addresses to try for the family's hub, in order. At least one has to work now. */
  setHubAddresses: (urls: string[]) => call<HubAddresses>("set_hub_addresses", { urls }),
};

/** The desktop app (Windows, macOS, Linux), where KitChat sits in the tray and can start with the computer. */
export const isDesktopApp = isTauri && !isAndroid;

/** Opens a web link in the browser, never inside the app. */
export async function openLink(url: string) {
  if (!/^https?:\/\//i.test(url)) return;
  if (isTauri) {
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    await openUrl(url);
  } else {
    window.open(url, "_blank", "noopener,noreferrer");
  }
}

/** The message a failed call should show. */
export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return "Something went wrong.";
}
