// Writing a message: text, emoji, Kit stickers, and photos and videos (picked, pasted or dropped), sent at full size.

import { Film, ImagePlus, SendHorizontal, Smile, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { api, errorText, isAndroid } from "../lib/api";
import { mediaFiles } from "../lib/images";
import { durationText } from "../lib/media";
import { type Attachment, fromBlobs, fromItems, release, toOutgoing } from "../lib/outgoing";
import { STICKERS } from "../lib/types";
import { Kit, STICKER_MOOD } from "./Kit";
import { IconButton, toast } from "./ui";

const EMOJI = [
  "😀",
  "😂",
  "🥰",
  "😎",
  "🤩",
  "😮",
  "😢",
  "😡",
  "👍",
  "👏",
  "🙏",
  "💪",
  "❤️",
  "🎉",
  "🔥",
  "⭐",
  "🌈",
  "🍕",
  "🎮",
  "⚽",
  "🐶",
  "🦄",
  "🚀",
  "💡",
];

export function Composer({
  chatId,
  attachments,
  setAttachments,
  onSent,
}: {
  chatId: string;
  attachments: Attachment[];
  setAttachments: (update: (a: Attachment[]) => Attachment[]) => void;
  onSent: () => void;
}) {
  const [text, setText] = useState("");
  const [panel, setPanel] = useState(false);
  const [sending, setSending] = useState(false);
  const [preparing, setPreparing] = useState(false);
  const box = useRef<HTMLTextAreaElement>(null);
  const picker = useRef<HTMLInputElement>(null);

  // Grow the box with what's typed, up to a few lines.
  useEffect(() => {
    const el = box.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 160)}px`;
    el.style.overflowY = el.scrollHeight > 160 ? "auto" : "hidden";
  }, [text]);

  const send = async () => {
    if (sending) return;
    const trimmed = text.trim();
    if (!trimmed && attachments.length === 0) return;
    setSending(true);
    try {
      if (attachments.length) {
        const items = [];
        for (const [i, a] of attachments.entries()) items.push(await toOutgoing(a, i === 0 ? trimmed : ""));
        await api.sendMedia(chatId, items);
        attachments.forEach(release);
      }
      if (trimmed && attachments.length === 0) await api.sendMessage(chatId, { kind: "text", text: trimmed });
      setText("");
      setAttachments(() => []);
      setPanel(false);
      onSent();
    } catch (e) {
      toast(errorText(e), "error");
    } finally {
      setSending(false);
      box.current?.focus();
    }
  };

  const pick = async () => {
    if (!isAndroid) {
      picker.current?.click();
      return;
    }
    setPreparing(true);
    try {
      const items = await api.pickMedia();
      if (items === null) picker.current?.click();
      else if (items.length) setAttachments((a) => [...a, ...fromItems(items)]);
    } catch (e) {
      toast(errorText(e), "error");
    } finally {
      setPreparing(false);
    }
  };

  const sendSticker = async (sticker: string) => {
    setPanel(false);
    try {
      await api.sendMessage(chatId, { kind: "sticker", sticker });
      onSent();
    } catch (e) {
      toast(errorText(e), "error");
    }
  };

  return (
    <div className="safe-bottom border-t-2 border-line bg-surface">
      {attachments.length > 0 && (
        <div className="flex gap-2 overflow-x-auto px-3 pt-3">
          {attachments.map((a) => (
            <div key={a.id} className="anim-pop relative shrink-0">
              {a.kind === "video" && a.blob ? (
                <video src={a.url} muted playsInline preload="metadata" className="size-24 rounded-2xl object-cover" />
              ) : a.url ? (
                <img src={a.url} alt="" className="size-24 rounded-2xl object-cover" />
              ) : (
                <div className="grid size-24 place-items-center rounded-2xl bg-surface-2 text-ink-soft">
                  <Film />
                </div>
              )}
              {a.kind === "video" && (
                <span className="absolute bottom-1 left-1 flex items-center gap-1 rounded-full bg-black/60 px-1.5 text-xs font-bold text-white">
                  <Film size={12} />
                  {a.ready?.durationMs ? durationText(a.ready.durationMs) : "Video"}
                </span>
              )}
              <button
                className="absolute -top-2 -right-2 grid size-8 place-items-center rounded-full bg-ink text-bg"
                aria-label="Remove"
                onClick={() => {
                  release(a);
                  setAttachments((list) => list.filter((x) => x.id !== a.id));
                }}
              >
                <X size={16} />
              </button>
            </div>
          ))}
        </div>
      )}
      {panel && (
        <div className="anim-rise border-b-2 border-line p-3">
          <div className="mb-2 flex flex-wrap gap-1">
            {EMOJI.map((e) => (
              <button
                key={e}
                className="grid size-11 place-items-center rounded-xl text-2xl hover:bg-brand-soft active:scale-90"
                onClick={() => {
                  setText((t) => t + e);
                  box.current?.focus();
                }}
              >
                {e}
              </button>
            ))}
          </div>
          <div className="grid grid-cols-5 gap-2">
            {STICKERS.map((s) => (
              <button
                key={s}
                className="grid place-items-center rounded-2xl p-1 hover:bg-brand-soft active:scale-95"
                onClick={() => void sendSticker(s)}
                title={s}
              >
                <Kit mood={STICKER_MOOD[s]} size={64} />
              </button>
            ))}
          </div>
        </div>
      )}
      <div className="flex items-end gap-1 p-2">
        <IconButton
          label="Emoji and stickers"
          onClick={() => setPanel((p) => !p)}
          className={panel ? "bg-brand-soft" : ""}
        >
          <Smile />
        </IconButton>
        <IconButton
          label="Send photos or videos"
          onClick={() => void pick()}
          className={preparing ? "animate-pulse" : ""}
        >
          <ImagePlus />
        </IconButton>
        <input
          ref={picker}
          type="file"
          accept="image/*,video/*"
          multiple
          hidden
          onChange={(e) => {
            const files = mediaFiles(e.target.files);
            if (files.length) setAttachments((a) => [...a, ...fromBlobs(files)]);
            e.target.value = "";
          }}
        />
        <textarea
          ref={box}
          rows={1}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              void send();
            }
          }}
          onPaste={(e) => {
            const files = mediaFiles(e.clipboardData?.items);
            if (files.length) {
              e.preventDefault();
              setAttachments((a) => [...a, ...fromBlobs(files)]);
            }
          }}
          placeholder={attachments.length ? "Add a caption…" : "Message"}
          className="max-h-40 min-h-12 flex-1 resize-none rounded-3xl border-2 border-line bg-surface-2 px-4 py-2.5 text-lg outline-none focus:border-brand"
        />
        <button
          type="button"
          aria-label="Send"
          disabled={sending || preparing || (!text.trim() && attachments.length === 0)}
          onClick={() => void send()}
          className="grid size-12 shrink-0 place-items-center rounded-full bg-brand text-brand-ink shadow-[0_3px_0_rgba(0,0,0,0.15)] transition active:scale-90 disabled:opacity-40"
        >
          <SendHorizontal />
        </button>
      </div>
    </div>
  );
}
