// Things shared into KitChat from another app (Android's Share button): a screenshot, a photo, a video, a link. Asks
// which chat it's for, then opens that chat with the photos and videos waiting to send, or sends the text straight
// away.

import { useCallback, useEffect, useState } from "react";
import { api, errorText, isTauri } from "../lib/api";
import { type Attachment, fromItems } from "../lib/outgoing";
import { Kit } from "./Kit";
import { ShareToChat } from "./NoteEditor";
import { toast } from "./ui";

export interface Incoming {
  media: Attachment[];
  text: string;
}

/** Watches for shares and, when one arrives, shows the chat picker. */
export function ShareSheet({ onPicked }: { onPicked: (chatId: string, media: Attachment[]) => void }) {
  const [incoming, setIncoming] = useState<Incoming | null>(null);

  const check = useCallback(async () => {
    try {
      const shared = await api.takeShared();
      if (shared.problem) toast(shared.problem, "error");
      const text = shared.items
        .filter((i) => i.kind === "text" && i.text.trim())
        .map((i) => i.text.trim())
        .join("\n");
      const got = { media: fromItems(shared.items), text };
      if (got.media.length || got.text) setIncoming(got);
    } catch (e) {
      void api.log("warn", `take_shared: ${errorText(e)}`);
    }
  }, []);

  useEffect(() => {
    if (!isTauri) return;
    void check();
    window.__kitSharedIn = () => void check();
    const vis = () => document.visibilityState === "visible" && void check();
    document.addEventListener("visibilitychange", vis);
    return () => {
      window.__kitSharedIn = undefined;
      document.removeEventListener("visibilitychange", vis);
    };
  }, [check]);

  if (!incoming) return null;

  const pick = async (chatId: string) => {
    const got = incoming;
    setIncoming(null);
    try {
      if (got.text) {
        await api.sendMessage(chatId, { kind: "text", text: got.text });
        toast("Sent! 🎉");
      }
      onPicked(chatId, got.media);
    } catch (e) {
      toast(errorText(e), "error");
    }
  };

  const n = incoming.media.length;
  const videos = incoming.media.every((m) => m.kind === "video");
  const what = n === 0 ? "this" : n === 1 ? (videos ? "this video" : "this picture") : `these ${n}`;

  return (
    <ShareToChat
      title={
        <span className="flex items-center gap-2">
          <Kit mood="camera" size={40} /> Send {what} to…
        </span>
      }
      onPick={(id) => void pick(id)}
      onClose={() => setIncoming(null)}
    />
  );
}
