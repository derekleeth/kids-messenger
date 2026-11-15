// Things shared into KitChat from another app (Android's Share button): a screenshot, a photo, a link. Asks which
// chat it's for, then opens that chat with the pictures waiting to send, or sends the text straight away.

import { useCallback, useEffect, useState } from "react";
import { api, errorText, isTauri } from "../lib/api";
import type { SharedItem } from "../lib/types";
import { Kit } from "./Kit";
import { ShareToChat } from "./NoteEditor";
import { toast } from "./ui";

export interface Incoming {
  images: Blob[];
  text: string;
}

async function load(items: SharedItem[]): Promise<Incoming> {
  const images: Blob[] = [];
  const texts: string[] = [];
  for (const item of items) {
    if (item.kind === "image") {
      try {
        const bytes = await api.readShared(item.path);
        images.push(new Blob([bytes], { type: item.mime || "image/jpeg" }));
      } catch (e) {
        void api.log("warn", `shared picture unreadable: ${errorText(e)}`);
      }
    } else if (item.text.trim()) {
      texts.push(item.text.trim());
    }
  }
  return { images, text: texts.join("\n") };
}

/** Watches for shares and, when one arrives, shows the chat picker. */
export function ShareSheet({ onPicked }: { onPicked: (chatId: string, images: Blob[]) => void }) {
  const [incoming, setIncoming] = useState<Incoming | null>(null);

  const check = useCallback(async () => {
    try {
      const items = await api.takeShared();
      if (items.length === 0) return;
      const got = await load(items);
      if (got.images.length || got.text) setIncoming(got);
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
      onPicked(chatId, got.images);
    } catch (e) {
      toast(errorText(e), "error");
    }
  };

  const what = incoming.images.length
    ? incoming.images.length === 1
      ? "this picture"
      : `these ${incoming.images.length} pictures`
    : "this";

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
