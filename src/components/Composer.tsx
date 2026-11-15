// Writing a message: text, emoji, Kit stickers and pictures (picked, pasted or dropped).

import { ImagePlus, SendHorizontal, Smile, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { api, errorText } from "../lib/api";
import { imageFiles, prepareImage } from "../lib/images";
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

export interface Attachment {
  id: number;
  file: Blob;
  url: string;
}

let nextAttachment = 1;

export function toAttachments(files: Blob[]): Attachment[] {
  return files.map((file) => ({ id: nextAttachment++, file, url: URL.createObjectURL(file) }));
}

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
      for (const [i, a] of attachments.entries()) {
        const prepared = await prepareImage(a.file);
        await api.sendImage(chatId, prepared.bytes, prepared.width, prepared.height, i === 0 ? trimmed : "");
        URL.revokeObjectURL(a.url);
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
              <img src={a.url} alt="" className="size-24 rounded-2xl object-cover" />
              <button
                className="absolute -top-2 -right-2 grid size-8 place-items-center rounded-full bg-ink text-bg"
                aria-label="Remove picture"
                onClick={() => {
                  URL.revokeObjectURL(a.url);
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
        <IconButton label="Send a picture" onClick={() => picker.current?.click()}>
          <ImagePlus />
        </IconButton>
        <input
          ref={picker}
          type="file"
          accept="image/*"
          multiple
          hidden
          onChange={(e) => {
            const files = imageFiles(e.target.files);
            if (files.length) setAttachments((a) => [...a, ...toAttachments(files)]);
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
            const files = imageFiles(e.clipboardData?.items);
            if (files.length) {
              e.preventDefault();
              setAttachments((a) => [...a, ...toAttachments(files)]);
            }
          }}
          placeholder={attachments.length ? "Add a caption…" : "Message"}
          className="max-h-40 min-h-12 flex-1 resize-none rounded-3xl border-2 border-line bg-surface-2 px-4 py-2.5 text-lg outline-none focus:border-brand"
        />
        <button
          type="button"
          aria-label="Send"
          disabled={sending || (!text.trim() && attachments.length === 0)}
          onClick={() => void send()}
          className="grid size-12 shrink-0 place-items-center rounded-full bg-brand text-brand-ink shadow-[0_3px_0_rgba(0,0,0,0.15)] transition active:scale-90 disabled:opacity-40"
        >
          <SendHorizontal />
        </button>
      </div>
    </div>
  );
}
