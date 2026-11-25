// One open chat: its messages, newest at the bottom, and the box to write in. Pictures can be dropped or pasted
// anywhere on it.

import { ArrowLeft, Link2 } from "lucide-react";
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { api } from "../lib/api";
import { useBack } from "../lib/back";
import { dayLabel, sameDay } from "../lib/format";
import { mediaFiles } from "../lib/images";
import { type Attachment, fromBlobs } from "../lib/outgoing";
import { memberById, mergeMessages, otherMember, readBy, setState, useAppState } from "../lib/state";
import type { Message } from "../lib/types";
import { Composer } from "./Composer";
import { Kit } from "./Kit";
import { MediaViewer, type Viewing } from "./MediaViewer";
import { MessageBubble } from "./MessageBubble";
import { ShelfPanel } from "./ShelfPanel";
import { Avatar, IconButton, StatusPill } from "./ui";

const EMPTY: Message[] = [];

export function ChatView({
  chatId,
  onBack,
  onOpenNote,
  pendingShare,
  clearPendingShare,
}: {
  chatId: string;
  onBack: (() => void) | null;
  onOpenNote: (noteId: string) => void;
  pendingShare?: Attachment[];
  clearPendingShare?: () => void;
}) {
  const chat = useAppState((s) => s.chats.find((c) => c.id === chatId));
  const messages = useAppState((s) => s.messages[chatId] ?? EMPTY);
  const members = useAppState((s) => s.members);
  const me = useAppState((s) => s.account?.me.id ?? "");
  const status = useAppState((s) => s.status);
  const [viewer, setViewer] = useState<Viewing | null>(null);
  const [attachments, setAttachments] = useState<Attachment[]>([]);
  const [dragging, setDragging] = useState(false);
  const [hop, setHop] = useState(0);
  const [loadingOlder, setLoadingOlder] = useState(false);
  const [noMore, setNoMore] = useState(false);
  const [shelfOpen, setShelfOpen] = useState(false);
  // Beside the chat when there's room (a wide window with the chat list already showing), otherwise a sheet.
  const docked = !onBack && typeof window !== "undefined" && window.matchMedia("(min-width: 1180px)").matches;
  const scroller = useRef<HTMLDivElement>(null);
  const stickToBottom = useRef(true);

  useBack(!!onBack && !viewer, () => onBack?.());
  useBack(!!viewer, () => setViewer(null));

  // Pictures shared in from another app, waiting to be sent here.
  useEffect(() => {
    if (pendingShare && pendingShare.length) {
      setAttachments((a) => [...a, ...pendingShare]);
      clearPendingShare?.();
    }
  }, [pendingShare, clearPendingShare]);

  // Kit hops when a message goes, then leaves so the newest message's ticks show.
  useEffect(() => {
    if (!hop) return;
    const t = setTimeout(() => setHop(0), 1200);
    return () => clearTimeout(t);
  }, [hop]);

  // Keep to the newest message as they arrive, unless scrolled up to read older ones.
  useLayoutEffect(() => {
    const el = scroller.current;
    if (el && stickToBottom.current) el.scrollTop = el.scrollHeight;
  }, [messages.length, chatId]);

  useEffect(() => {
    stickToBottom.current = true;
    setNoMore(false);
  }, [chatId]);

  // Pictures and link cards load after the first layout; stay at the bottom while they grow, if already there.
  const content = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = scroller.current;
    const inner = content.current;
    if (!el || !inner) return;
    const watch = new ResizeObserver(() => {
      if (stickToBottom.current) el.scrollTop = el.scrollHeight;
    });
    watch.observe(inner);
    return () => watch.disconnect();
  }, []);

  // "Seen": the newest message from someone else, while this chat is on screen.
  const lastFromOthers = [...messages].reverse().find((m) => !m.pending && m.senderId !== me);
  const seen = chat?.readUpTo[me] ?? "";
  useEffect(() => {
    if (!lastFromOthers || lastFromOthers.id <= seen) return;
    if (document.visibilityState !== "visible") return;
    void api.markRead(chatId, lastFromOthers.id).catch(() => {});
  }, [chatId, lastFromOthers, seen]);

  const loadOlder = useCallback(async () => {
    const oldest = messages.find((m) => !m.pending);
    if (!oldest || loadingOlder || noMore) return;
    setLoadingOlder(true);
    const el = scroller.current;
    const before = el ? el.scrollHeight - el.scrollTop : 0;
    try {
      const older = await api.olderMessages(chatId, oldest.id);
      if (older.length === 0) setNoMore(true);
      setState((s) => ({ ...s, messages: mergeMessages(s.messages, older) }));
      requestAnimationFrame(() => {
        if (el) el.scrollTop = el.scrollHeight - before;
      });
    } finally {
      setLoadingOlder(false);
    }
  }, [chatId, messages, loadingOlder, noMore]);

  if (!chat) return null;
  const isFamily = chat.kind === "family";
  const other = isFamily ? undefined : memberById(members, otherMember(chat, me) ?? "");
  const title = isFamily ? "Family chat" : (other?.name ?? "Chat");
  const lastMine = [...messages].reverse().find((m) => m.senderId === me && !m.pending);
  // Who saw it shows only in the family chat: in a 1:1 chat the ticks say it.
  const seenBy = lastMine && isFamily ? readBy(chat, lastMine.id, me).map((id) => memberById(members, id)) : [];

  return (
    <div className="flex h-full min-h-0">
      <div
        className="relative flex h-full min-h-0 min-w-0 flex-1 flex-col bg-bg"
        onDragOver={(e) => {
          if (Array.from(e.dataTransfer.items).some((i) => i.kind === "file")) {
            e.preventDefault();
            setDragging(true);
          }
        }}
        onDragLeave={(e) => e.currentTarget === e.target && setDragging(false)}
        onDrop={(e) => {
          e.preventDefault();
          setDragging(false);
          const files = mediaFiles(e.dataTransfer.files);
          if (files.length) setAttachments((a) => [...a, ...fromBlobs(files)]);
        }}
      >
        <header className="safe-top flex items-center gap-2 border-b-2 border-line bg-surface px-2 py-2">
          {onBack && (
            <IconButton label="Back" onClick={onBack}>
              <ArrowLeft />
            </IconButton>
          )}
          {isFamily ? <Kit mood="hug" size={44} /> : <Avatar member={other} size={44} />}
          <div className="min-w-0 flex-1 px-1">
            <div className="truncate text-xl font-black">{title}</div>
            {isFamily && (
              <div className="truncate text-sm text-ink-soft">
                {chat.memberIds
                  .map((id) => memberById(members, id)?.name)
                  .filter(Boolean)
                  .join(", ")}
              </div>
            )}
          </div>
          <StatusPill status={status} />
          <IconButton label="Links & lists" aria-pressed={shelfOpen} onClick={() => setShelfOpen((o) => !o)}>
            <Link2 />
          </IconButton>
        </header>

        <div
          ref={scroller}
          className="scroll-area flex-1 px-3 py-4"
          onScroll={(e) => {
            const el = e.currentTarget;
            stickToBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
            if (el.scrollTop < 60) void loadOlder();
          }}
        >
          {messages.length === 0 && (
            <div className="flex h-full flex-col items-center justify-center text-center text-ink-soft">
              <Kit mood="wave" size={150} className="anim-breathe" />
              <p className="mt-2 text-lg font-bold">Say hi{other ? ` to ${other.name}` : ""}! 👋</p>
              <p>Send a message, a picture, a link or a sticker.</p>
            </div>
          )}
          <div ref={content} className="mx-auto flex max-w-3xl flex-col gap-1.5">
            {messages.map((m, i) => {
              const prev = messages[i - 1];
              const newDay = !prev || !sameDay(prev.createdAt, m.createdAt);
              const showSender =
                newDay || !prev || prev.senderId !== m.senderId || m.createdAt - prev.createdAt > 5 * 60_000;
              return (
                <div key={m.id} className={showSender && i > 0 ? "mt-2" : ""}>
                  {newDay && (
                    <div className="my-3 text-center">
                      <span className="rounded-full bg-bg-2 px-3 py-1 text-sm font-bold text-ink-soft">
                        {dayLabel(m.createdAt)}
                      </span>
                    </div>
                  )}
                  <MessageBubble
                    message={m}
                    showSender={showSender && (isFamily || m.senderId !== me)}
                    onOpenImage={setViewer}
                    onOpenNote={onOpenNote}
                  />
                </div>
              );
            })}
            {seenBy.length > 0 && (
              <div className="flex items-center justify-end gap-1 pr-2 text-xs text-ink-soft">
                Seen{" "}
                {seenBy.map((m) => (
                  <Avatar key={m?.id} member={m} size={18} />
                ))}
              </div>
            )}
          </div>
        </div>

        <div className="relative">
          {hop > 0 && (
            <Kit key={hop} mood="happy" size={48} className="anim-hop pointer-events-none absolute -top-12 right-4" />
          )}
          <Composer
            chatId={chatId}
            attachments={attachments}
            setAttachments={setAttachments}
            onSent={() => {
              stickToBottom.current = true;
              setHop((h) => h + 1);
            }}
          />
        </div>

        {dragging && (
          <div className="pointer-events-none absolute inset-0 z-20 grid place-items-center bg-[rgba(124,92,255,0.18)]">
            <div className="card flex flex-col items-center p-6">
              <Kit mood="camera" size={110} />
              <p className="text-xl font-black">Drop photos or videos to send</p>
            </div>
          </div>
        )}

        {viewer && <MediaViewer viewing={viewer} onClose={() => setViewer(null)} />}
      </div>
      {shelfOpen && (
        <ShelfPanel
          chatId={chatId}
          docked={docked}
          onClose={() => setShelfOpen(false)}
          onOpenNote={(id) => {
            if (!docked) setShelfOpen(false);
            onOpenNote(id);
          }}
        />
      )}
    </div>
  );
}
