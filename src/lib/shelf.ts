// The Links & lists panel: every web link and shared list sent in a chat, so a parent at a laptop can find "that
// site he sent on Tuesday" without scrolling back. The same rule as src-tauri/src/store.rs `on_shelf`.

import { site, splitLinks } from "./links";
import type { LinkPreview, Message } from "./types";

export function onShelf(m: Message): boolean {
  if (m.body.kind === "note") return true;
  return m.body.kind === "text" && splitLinks(m.body.text).some((p) => p.url);
}

export interface LinkEntry {
  /** Unique within the panel: a message can hold several links. */
  key: string;
  url: string;
  site: string;
  /** The title from the hub's preview, for the link the preview is about. */
  title: string;
  preview: LinkPreview | null;
  /** The message's words, for context and search. */
  text: string;
  message: Message;
}

/** One entry per link, in the order the messages came (newest first, as given). */
export function linkEntries(messages: Message[]): LinkEntry[] {
  const out: LinkEntry[] = [];
  for (const m of messages) {
    if (m.body.kind !== "text") continue;
    const text = m.body.text;
    const urls = [...new Set(splitLinks(text).flatMap((p) => (p.url ? [p.url] : [])))];
    for (const url of urls) {
      const preview = m.link && m.link.url === url ? m.link : null;
      out.push({ key: `${m.id}:${url}`, url, site: site(url), title: preview?.title ?? "", preview, text, message: m });
    }
  }
  return out;
}

/** Whether every word of the search appears somewhere in the link, its title, site or message. */
export function matches(query: string, ...fields: string[]): boolean {
  const hay = fields.join(" ").toLowerCase();
  return query
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean)
    .every((w) => hay.includes(w));
}
