// Links & lists: every web link and shared list sent in a chat, newest first, with a search box. Beside the chat on
// a wide screen (the desktop app), a sheet over it on a phone.

import { Check, Copy, ExternalLink, Link2, ListChecks, Search, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { api, errorText, openLink } from "../lib/api";
import { shortWhen } from "../lib/format";
import { splitLinks } from "../lib/links";
import { type LinkEntry, linkEntries, matches } from "../lib/shelf";
import { memberById, useAppState } from "../lib/state";
import type { Message } from "../lib/types";
import { Avatar, IconButton, inputClass, Sheet, toast } from "./ui";

type Tab = "links" | "lists";

async function copy(text: string) {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    toast("Couldn't copy that.", "error");
    return false;
  }
}

function useShelf(chatId: string) {
  const [items, setItems] = useState<Message[] | null>(null);
  // Load again whenever the chat gets a new message (or one changes), so the panel keeps up while open.
  const newest = useAppState((s) => {
    const list = s.messages[chatId];
    const last = list?.[list.length - 1];
    return last ? `${last.id}:${last.rev}:${list.length}` : "";
  });
  useEffect(() => {
    let live = true;
    api
      .chatShelf(chatId)
      .then((got) => live && setItems(got))
      .catch((e) => {
        if (live) {
          setItems([]);
          toast(errorText(e), "error");
        }
      });
    return () => {
      live = false;
    };
  }, [chatId, newest]);
  return items;
}

function LinkRow({ entry }: { entry: LinkEntry }) {
  const members = useAppState((s) => s.members);
  const sender = memberById(members, entry.message.senderId);
  const [copied, setCopied] = useState(false);
  // The message's own words, without its links (they're the rows).
  const words = splitLinks(entry.text)
    .filter((p) => !p.url)
    .map((p) => p.text)
    .join(" ")
    .replace(/\s+/g, " ")
    .trim();
  return (
    <li className="group flex items-start gap-3 rounded-2xl p-2 hover:bg-bg-2">
      <Avatar member={sender} size={34} />
      <button
        className="min-w-0 flex-1 text-left"
        title={entry.url}
        onClick={() => void openLink(entry.url).catch((e) => toast(errorText(e), "error"))}
      >
        <div className="truncate font-extrabold text-brand underline-offset-2 group-hover:underline">
          {entry.title || entry.site}
        </div>
        <div className="truncate text-sm text-ink-soft">
          {entry.site} · {sender?.name ?? "Someone"} · {shortWhen(entry.message.createdAt)}
        </div>
        {words && <div className="line-clamp-2 text-sm">{words}</div>}
      </button>
      <IconButton
        label={copied ? "Copied" : "Copy link"}
        className="size-10"
        onClick={() =>
          void copy(entry.url).then((ok) => {
            if (!ok) return;
            setCopied(true);
            setTimeout(() => setCopied(false), 1500);
          })
        }
      >
        {copied ? <Check size={18} /> : <Copy size={18} />}
      </IconButton>
      <IconButton label="Open in the browser" className="size-10" onClick={() => void openLink(entry.url)}>
        <ExternalLink size={18} />
      </IconButton>
    </li>
  );
}

function ListRow({ message, onOpen }: { message: Message; onOpen: (noteId: string) => void }) {
  const members = useAppState((s) => s.members);
  if (message.body.kind !== "note") return null;
  const { noteId, title, emoji, preview, shared } = message.body;
  const sender = memberById(members, message.senderId);
  return (
    <li>
      <button
        className="flex w-full items-start gap-3 rounded-2xl p-2 text-left hover:bg-bg-2"
        onClick={() => onOpen(noteId)}
      >
        <span className="grid size-10 shrink-0 place-items-center rounded-xl bg-brand-soft text-xl">{emoji}</span>
        <span className="min-w-0 flex-1">
          <span className="block truncate font-extrabold">{title}</span>
          <span className="block truncate text-sm text-ink-soft">
            {shared ? "Family list" : "Shared copy"} · {sender?.name ?? "Someone"} · {shortWhen(message.createdAt)}
          </span>
          {preview.length > 0 && <span className="line-clamp-2 block text-sm">{preview.join(" · ")}</span>}
        </span>
      </button>
    </li>
  );
}

function Body({ chatId, onOpenNote }: { chatId: string; onOpenNote: (noteId: string) => void }) {
  const items = useShelf(chatId);
  const [tab, setTab] = useState<Tab>("links");
  const [query, setQuery] = useState("");
  const links = useMemo(
    () => linkEntries(items ?? []).filter((e) => matches(query, e.url, e.title, e.text, e.preview?.description ?? "")),
    [items, query],
  );
  const lists = useMemo(
    () => (items ?? []).filter((m) => m.body.kind === "note" && matches(query, m.body.title, m.body.preview.join(" "))),
    [items, query],
  );
  const copyAll = useCallback(async () => {
    if (await copy(links.map((l) => (l.title ? `${l.title} - ${l.url}` : l.url)).join("\n")))
      toast(`Copied ${links.length} link${links.length === 1 ? "" : "s"}`);
  }, [links]);

  const tabButton = (t: Tab, label: string, icon: ReactNode, count: number) => (
    <button
      onClick={() => setTab(t)}
      aria-pressed={tab === t}
      className={`flex flex-1 items-center justify-center gap-1.5 rounded-full px-3 py-1.5 font-extrabold transition ${
        tab === t ? "bg-brand text-white" : "text-ink-soft hover:bg-bg-2"
      }`}
    >
      {icon} {label} {items && <span className="opacity-80">({count})</span>}
    </button>
  );

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3">
      <div className="flex gap-1 rounded-full bg-surface-2 p-1">
        {tabButton("links", "Links", <Link2 size={18} />, links.length)}
        {tabButton("lists", "Lists", <ListChecks size={18} />, lists.length)}
      </div>
      <label className="relative block">
        <Search size={18} className="pointer-events-none absolute top-1/2 left-3.5 -translate-y-1/2 text-ink-soft" />
        <input
          className={`${inputClass} py-2 pl-10 text-base`}
          placeholder={tab === "links" ? "Search links" : "Search lists"}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          aria-label="Search"
        />
      </label>
      {items === null ? (
        <p className="p-4 text-center text-ink-soft">Gathering everything…</p>
      ) : tab === "links" ? (
        links.length === 0 ? (
          <p className="p-4 text-center text-ink-soft">{query ? "No links match." : "No links in this chat yet."}</p>
        ) : (
          <>
            <ul className="flex flex-col gap-0.5">
              {links.map((e) => (
                <LinkRow key={e.key} entry={e} />
              ))}
            </ul>
            <button className="self-start px-2 text-sm font-extrabold text-brand" onClick={() => void copyAll()}>
              Copy all {links.length} link{links.length === 1 ? "" : "s"}
            </button>
          </>
        )
      ) : lists.length === 0 ? (
        <p className="p-4 text-center text-ink-soft">
          {query ? "No lists match." : "No lists shared in this chat yet."}
        </p>
      ) : (
        <ul className="flex flex-col gap-0.5">
          {lists.map((m) => (
            <ListRow key={m.id} message={m} onOpen={onOpenNote} />
          ))}
        </ul>
      )}
      <p className="px-2 text-xs text-ink-soft">Older messages show here once the chat has loaded them.</p>
    </div>
  );
}

export function ShelfPanel({
  chatId,
  docked,
  onClose,
  onOpenNote,
}: {
  chatId: string;
  /** Beside the chat (wide screens), rather than a sheet over it. */
  docked: boolean;
  onClose: () => void;
  onOpenNote: (noteId: string) => void;
}) {
  useEffect(() => {
    if (!docked) return;
    const key = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [docked, onClose]);

  if (!docked) {
    return (
      <Sheet title="Links & lists" onClose={onClose}>
        <Body chatId={chatId} onOpenNote={onOpenNote} />
      </Sheet>
    );
  }
  return (
    <aside className="flex w-[360px] shrink-0 flex-col border-l-2 border-line bg-surface">
      <div className="flex items-center gap-2 px-4 pt-3 pb-2">
        <h2 className="flex-1 text-lg font-black">Links & lists</h2>
        <IconButton label="Close" className="size-10" onClick={onClose}>
          <X size={20} />
        </IconButton>
      </div>
      <div className="scroll-area flex min-h-0 flex-1 flex-col px-3 pb-3">
        <Body chatId={chatId} onOpenNote={onOpenNote} />
      </div>
    </aside>
  );
}
