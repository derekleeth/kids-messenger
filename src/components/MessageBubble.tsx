// One message: text with links, a photo or video, a Kit sticker or a shared list, with its link preview and reactions.

import {
  AlertCircle,
  Check,
  CheckCheck,
  Clock,
  ExternalLink,
  Film,
  Play,
  RotateCcw,
  SmilePlus,
  Trash2,
} from "lucide-react";
import { useState } from "react";
import { api, errorText, openLink } from "../lib/api";
import { timeOf } from "../lib/format";
import { isBigEmoji, site, splitLinks } from "../lib/links";
import { durationText, percent, useMedia, useUpload } from "../lib/media";
import { dropMessage, memberById, readBy, setState, useAppState } from "../lib/state";
import type { LinkPreview, Message } from "../lib/types";
import { Kit, STICKER_MOOD } from "./Kit";
import type { Viewing } from "./MediaViewer";
import { Avatar, toast } from "./ui";

export const QUICK_REACTIONS = ["❤️", "😂", "😮", "👍", "🎉", "😢"];

/** One tick when it's sent, two coloured ticks once everyone else in the chat has seen it. */
function Ticks({ seenBy, everyone }: { seenBy: string[]; everyone: number }) {
  const members = useAppState((s) => s.members);
  const names = seenBy.map((id) => memberById(members, id)?.name).filter(Boolean);
  if (everyone > 0 && seenBy.length >= everyone) {
    return (
      <span role="img" className="text-brand" aria-label="Seen" title={`Seen by ${names.join(", ")}`}>
        <CheckCheck size={20} strokeWidth={2.75} />
      </span>
    );
  }
  return (
    <span role="img" aria-label="Sent" title={names.length ? `Sent · seen by ${names.join(", ")}` : "Sent"}>
      <Check size={20} strokeWidth={2.75} />
    </span>
  );
}

function Linkified({ text }: { text: string }) {
  return (
    <>
      {splitLinks(text).map((part, i) =>
        part.url ? (
          <a
            key={i}
            href={part.url}
            onClick={(e) => {
              e.preventDefault();
              e.stopPropagation();
              void openLink(part.url!);
            }}
            className="font-bold break-all underline decoration-2 underline-offset-2"
          >
            {part.text}
          </a>
        ) : (
          <span key={i}>{part.text}</span>
        ),
      )}
    </>
  );
}

export function MediaImage({
  id,
  width,
  height,
  className = "",
  onOpen,
}: {
  id: string;
  width: number;
  height: number;
  className?: string;
  onOpen?: () => void;
}) {
  const { url, failed } = useMedia(id);
  const ratio = width > 0 && height > 0 ? `${width} / ${height}` : "4 / 3";
  return (
    <div
      className={`overflow-hidden rounded-2xl bg-surface-2 ${className}`}
      style={{ aspectRatio: ratio, maxHeight: 360 }}
      onClick={(e) => {
        if (url && onOpen) {
          e.stopPropagation();
          onOpen();
        }
      }}
    >
      {url ? (
        <img src={url} alt="" className="h-full w-full cursor-zoom-in object-cover" draggable={false} />
      ) : (
        <div className="grid h-full w-full place-items-center text-ink-soft">
          {failed ? "Picture not available" : "…"}
        </div>
      )}
    </div>
  );
}

/** A video in the chat: its still (or a plain card), with a play button and its length. */
function VideoCard({
  posterId,
  width,
  height,
  durationMs,
  onOpen,
}: {
  posterId?: string | null;
  width: number;
  height: number;
  durationMs: number;
  onOpen: () => void;
}) {
  const { url } = useMedia(posterId);
  const ratio = width > 0 && height > 0 ? `${width} / ${height}` : "16 / 9";
  return (
    <button
      className="relative block w-full overflow-hidden rounded-2xl bg-ink/80"
      style={{ aspectRatio: ratio, maxHeight: 360, minWidth: 160 }}
      onClick={(e) => {
        e.stopPropagation();
        onOpen();
      }}
      aria-label="Play video"
    >
      {url ? (
        <img src={url} alt="" className="h-full w-full object-cover" draggable={false} />
      ) : (
        <div className="grid h-full w-full place-items-center text-white/70">
          <Film size={40} />
        </div>
      )}
      <span className="absolute inset-0 grid place-items-center">
        <span className="grid size-14 place-items-center rounded-full bg-black/55 text-white">
          <Play size={28} />
        </span>
      </span>
      <span className="absolute bottom-2 left-2 rounded-full bg-black/60 px-2 text-sm font-bold text-white">
        {durationText(durationMs)}
      </span>
    </button>
  );
}

function LinkCard({ link, mine }: { link: LinkPreview; mine: boolean }) {
  return (
    <button
      onClick={(e) => {
        e.stopPropagation();
        void openLink(link.url);
      }}
      className={`mt-2 block w-full overflow-hidden rounded-2xl text-left ${mine ? "bg-white/15" : "bg-surface-2"}`}
    >
      {link.imageMediaId && <MediaImage id={link.imageMediaId} width={1200} height={630} className="rounded-none" />}
      <div className="p-3">
        <div className={`flex items-center gap-1 text-xs font-bold ${mine ? "opacity-80" : "text-ink-soft"}`}>
          <ExternalLink size={12} /> {link.site || site(link.url)}
        </div>
        {link.title && <div className="font-extrabold leading-snug">{link.title}</div>}
        {link.description && (
          <div className={`line-clamp-2 text-sm ${mine ? "opacity-85" : "text-ink-soft"}`}>{link.description}</div>
        )}
      </div>
    </button>
  );
}

export function MessageBubble({
  message,
  showSender,
  onOpenImage,
  onOpenNote,
}: {
  message: Message;
  showSender: boolean;
  onOpenImage: (viewing: Viewing) => void;
  onOpenNote: (noteId: string) => void;
}) {
  const me = useAppState((s) => s.account?.me.id ?? "");
  const members = useAppState((s) => s.members);
  const notes = useAppState((s) => s.notes);
  const [picking, setPicking] = useState(false);
  const mine = message.senderId === me;
  const sender = memberById(members, message.senderId);
  const body = message.body;
  const myReaction = message.reactions.find((r) => r.memberId === me)?.emoji;
  const uploaded = percent(useUpload(message.pending ? message.id : null));
  const chat = useAppState((s) => s.chats.find((c) => c.id === message.chatId));

  const react = async (emoji: string) => {
    setPicking(false);
    try {
      await api.react(message.id, myReaction === emoji ? "" : emoji);
    } catch (e) {
      toast(errorText(e), "error");
    }
  };

  const discard = async () => {
    await api.discardMessage(message.id);
    setState((s) => dropMessage(s, message.chatId, message.id));
  };

  const bare = body.kind === "sticker" || (body.kind === "text" && isBigEmoji(body.text) && !message.link);
  let content;
  switch (body.kind) {
    case "text":
      content =
        isBigEmoji(body.text) && !message.link ? (
          <div className="text-5xl leading-tight">{body.text}</div>
        ) : (
          <div className="text-[1.05rem] leading-snug break-words whitespace-pre-wrap">
            <Linkified text={body.text} />
          </div>
        );
      break;
    case "image":
      content = (
        <div className="-m-1">
          <MediaImage
            id={body.mediaId}
            width={body.width}
            height={body.height}
            onOpen={() =>
              onOpenImage({
                kind: "image",
                mediaId: body.mediaId,
                fullMediaId: body.fullMediaId,
                fullBytes: body.fullBytes,
              })
            }
          />
          {body.caption && (
            <div className="px-2 pt-2 pb-1">
              <Linkified text={body.caption} />
            </div>
          )}
        </div>
      );
      break;
    case "video":
      content = (
        <div className="-m-1 w-64 max-w-full">
          <VideoCard
            posterId={body.posterMediaId}
            width={body.width}
            height={body.height}
            durationMs={body.durationMs}
            onOpen={() =>
              onOpenImage({
                kind: "video",
                videoMediaId: body.videoMediaId,
                posterMediaId: body.posterMediaId,
                bytes: body.bytes,
                durationMs: body.durationMs,
              })
            }
          />
          {body.caption && (
            <div className="px-2 pt-2 pb-1">
              <Linkified text={body.caption} />
            </div>
          )}
        </div>
      );
      break;
    case "sticker":
      content = <Kit mood={STICKER_MOOD[body.sticker] ?? "happy"} size={130} className="anim-pop" />;
      break;
    case "note": {
      const available = notes.some((n) => n.id === body.noteId);
      content = (
        <button
          className={`block w-60 text-left ${available ? "" : "cursor-default"}`}
          onClick={(e) => {
            e.stopPropagation();
            if (available) onOpenNote(body.noteId);
          }}
        >
          <div className="flex items-center gap-2 text-lg font-black">
            <span className="text-2xl">{body.emoji}</span> {body.title || "A list"}
          </div>
          <div className="mt-1 space-y-0.5 text-[0.95rem]">
            {body.preview.map((line, i) => (
              <div key={i} className="truncate">
                {line}
              </div>
            ))}
          </div>
          <div className={`mt-2 text-sm font-bold ${mine ? "opacity-85" : "text-ink-soft"}`}>
            {available ? (body.shared ? "Family list · tap to open" : "Tap to open") : "Shared list"}
          </div>
        </button>
      );
      break;
    }
  }

  return (
    <div className={`group flex items-end gap-2 ${mine ? "flex-row-reverse" : ""}`}>
      {!mine && <div className="w-9">{showSender && <Avatar member={sender} size={36} />}</div>}
      <div className={`flex max-w-[78%] flex-col ${mine ? "items-end" : "items-start"}`}>
        {showSender && !mine && (
          <span className="mb-0.5 ml-3 text-sm font-extrabold text-ink-soft">{sender?.name}</span>
        )}
        <div className="relative flex items-center gap-1">
          <div
            onClick={() => !message.pending && setPicking((p) => !p)}
            className={
              bare
                ? "cursor-pointer"
                : `cursor-pointer rounded-[22px] px-4 py-2.5 shadow-[0_2px_0_rgba(0,0,0,0.06)] ${
                    mine ? "rounded-br-md bg-[var(--mine)] text-[var(--mine-ink)]" : "rounded-bl-md bg-[var(--theirs)]"
                  } ${message.pending ? "opacity-70" : ""}`
            }
          >
            {content}
            {message.link && body.kind === "text" && <LinkCard link={message.link} mine={mine} />}
          </div>
          {!message.pending && (
            <button
              className={`hidden size-9 shrink-0 place-items-center rounded-full text-ink-soft group-hover:grid hover:bg-brand-soft ${
                mine ? "order-first" : ""
              }`}
              aria-label="React"
              onClick={() => setPicking((p) => !p)}
            >
              <SmilePlus size={20} />
            </button>
          )}
          {picking && (
            <div
              className={`anim-pop card absolute bottom-full z-10 mb-2 flex gap-1 p-1.5 ${mine ? "right-0" : "left-0"}`}
              onMouseLeave={() => setPicking(false)}
            >
              {QUICK_REACTIONS.map((emoji) => (
                <button
                  key={emoji}
                  onClick={() => void react(emoji)}
                  className={`grid size-11 place-items-center rounded-full text-2xl transition hover:scale-125 ${
                    myReaction === emoji ? "bg-brand-soft" : ""
                  }`}
                >
                  {emoji}
                </button>
              ))}
            </div>
          )}
        </div>
        {message.reactions.length > 0 && (
          <div className={`-mt-1.5 flex gap-1 ${mine ? "mr-2" : "ml-2"}`}>
            {Object.entries(
              message.reactions.reduce<Record<string, number>>(
                (acc, r) => ({ ...acc, [r.emoji]: (acc[r.emoji] ?? 0) + 1 }),
                {},
              ),
            ).map(([emoji, count]) => (
              <button
                key={emoji}
                onClick={() => void react(emoji)}
                className={`anim-pop rounded-full border-2 bg-surface px-1.5 text-sm leading-6 ${
                  myReaction === emoji ? "border-brand" : "border-line"
                }`}
                title={message.reactions
                  .filter((r) => r.emoji === emoji)
                  .map((r) => memberById(members, r.memberId)?.name)
                  .join(", ")}
              >
                {emoji}
                {count > 1 && <span className="ml-0.5 font-bold">{count}</span>}
              </button>
            ))}
          </div>
        )}
        <div
          className={`mt-0.5 flex items-center gap-1 px-2 text-xs ${message.error ? "text-danger" : "text-ink-soft"}`}
        >
          {message.error ? (
            <>
              <AlertCircle size={14} /> {message.error}
              <button
                className="ml-1 inline-flex items-center gap-0.5 font-bold underline"
                onClick={() => void api.retryMessage(message.id)}
              >
                <RotateCcw size={12} /> Try again
              </button>
              <button
                className="ml-1 inline-flex items-center gap-0.5 font-bold underline"
                onClick={() => void discard()}
              >
                <Trash2 size={12} /> Delete
              </button>
            </>
          ) : message.pending ? (
            <>
              <Clock size={12} /> {uploaded !== null && uploaded < 100 ? `Sending… ${uploaded}%` : "Sending…"}
            </>
          ) : (
            <>
              <span className="opacity-0 transition group-hover:opacity-100">{timeOf(message.createdAt)}</span>
              {mine && chat && <Ticks seenBy={readBy(chat, message.id, me)} everyone={chat.memberIds.length - 1} />}
            </>
          )}
        </div>
      </div>
    </div>
  );
}
