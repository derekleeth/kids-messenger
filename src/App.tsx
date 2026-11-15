// The whole app: Welcome until this device is in a family, then three tabs (Chats, Lists, Family). On a wide
// screen the list and the open chat or note sit side by side; on a phone or tablet held upright, one at a time.

import { ListChecks, MessageCircle, Users } from "lucide-react";
import { useCallback, useEffect, useState, useSyncExternalStore } from "react";
import { ChatList } from "./components/ChatList";
import { ChatView } from "./components/ChatView";
import { FamilyView } from "./components/FamilyView";
import { Kit } from "./components/Kit";
import { NoteEditor } from "./components/NoteEditor";
import { NotesView } from "./components/NotesView";
import { ShareSheet } from "./components/ShareSheet";
import { Toasts, toast } from "./components/ui";
import { Welcome } from "./components/Welcome";
import { api, errorText, onDelta } from "./lib/api";
import type { Attachment } from "./lib/outgoing";
import { applyDelta, otherMember, setState, unreadCount, useAppState } from "./lib/state";

type Tab = "chats" | "lists" | "family";

const WIDE = "(min-width: 900px)";

function useWide() {
  return useSyncExternalStore(
    (cb) => {
      const q = window.matchMedia(WIDE);
      q.addEventListener("change", cb);
      return () => q.removeEventListener("change", cb);
    },
    () => window.matchMedia(WIDE).matches,
  );
}

function useBoot() {
  useEffect(() => {
    let stop: (() => void) | undefined;
    let cancelled = false;
    void (async () => {
      // Listen first, so nothing that happens between the snapshot and listening is missed.
      const unlisten = await onDelta((d) => setState((s) => applyDelta(s, d)));
      if (cancelled) return unlisten();
      stop = unlisten;
      try {
        const snap = await api.snapshot();
        setState((s) => applyDelta(s, { ...snap, reset: true }));
      } catch (e) {
        toast(errorText(e), "error");
        setState((s) => ({ ...s, loaded: true }));
      }
    })();
    const vis = () => void api.setForeground(document.visibilityState === "visible").catch(() => {});
    vis();
    document.addEventListener("visibilitychange", vis);
    return () => {
      cancelled = true;
      stop?.();
      document.removeEventListener("visibilitychange", vis);
    };
  }, []);
}

function TabButton({
  active,
  label,
  badge,
  icon: Icon,
  onClick,
  rail,
}: {
  active: boolean;
  label: string;
  badge?: number;
  icon: typeof Users;
  onClick: () => void;
  rail: boolean;
}) {
  return (
    <button
      onClick={onClick}
      aria-current={active ? "page" : undefined}
      className={`relative flex flex-col items-center gap-0.5 rounded-2xl px-3 py-1.5 font-extrabold transition ${
        rail ? "w-20" : "flex-1"
      } ${active ? "text-brand" : "text-ink-soft"}`}
    >
      <span className={`grid h-9 w-14 place-items-center rounded-full transition ${active ? "bg-brand-soft" : ""}`}>
        <Icon size={24} strokeWidth={active ? 2.6 : 2.2} />
      </span>
      <span className="text-sm">{label}</span>
      {!!badge && (
        <span className="absolute top-0 right-[calc(50%-1.6rem)] min-w-6 rounded-full bg-danger px-1.5 text-center text-sm leading-6 text-white">
          {badge > 99 ? "99+" : badge}
        </span>
      )}
    </button>
  );
}

function Empty({ tab }: { tab: Tab }) {
  return (
    <div className="grid h-full place-items-center p-8 text-center text-ink-soft">
      <div className="flex flex-col items-center">
        <Kit mood={tab === "lists" ? "list" : "sleep"} size={170} className="anim-breathe" />
        <p className="mt-2 text-lg font-bold">
          {tab === "lists" ? "Pick a list, or make a new one" : "Pick a chat to start"}
        </p>
      </div>
    </div>
  );
}

export default function App() {
  useBoot();
  const loaded = useAppState((s) => s.loaded);
  const account = useAppState((s) => s.account);
  const chats = useAppState((s) => s.chats);
  const messages = useAppState((s) => s.messages);
  const wide = useWide();
  const [tab, setTab] = useState<Tab>("chats");
  const [chatId, setChatId] = useState<string | null>(null);
  const [noteId, setNoteId] = useState<string | null>(null);
  const [pendingShare, setPendingShare] = useState<{ chatId: string; media: Attachment[] } | null>(null);

  // Signed out (or removed by a parent): forget what was open.
  useEffect(() => {
    if (!account) {
      setChatId(null);
      setNoteId(null);
      setTab("chats");
    }
  }, [account]);

  const openChat = useCallback((id: string) => {
    setTab("chats");
    setChatId(id);
  }, []);
  const openNote = useCallback((id: string) => {
    setTab("lists");
    setNoteId(id);
  }, []);
  const clearPendingShare = useCallback(() => setPendingShare(null), []);

  const openPerson = async (memberId: string) => {
    const me = account?.me.id ?? "";
    const existing = chats.find((c) => c.kind === "direct" && otherMember(c, me) === memberId);
    try {
      openChat(existing ? existing.id : (await api.openDirect(memberId)).id);
    } catch (e) {
      toast(errorText(e), "error");
    }
  };

  if (!loaded) {
    return (
      <div className="grid h-full place-items-center bg-bg">
        <Kit mood="happy" size={140} className="anim-breathe" />
      </div>
    );
  }
  if (!account) {
    return (
      <>
        <Welcome />
        <Toasts />
      </>
    );
  }

  const me = account.me.id;
  const unread = chats.reduce((n, c) => n + unreadCount(c, messages[c.id], me), 0);

  const detail =
    tab === "chats" && chatId ? (
      <ChatView
        key={chatId}
        chatId={chatId}
        onBack={wide ? null : () => setChatId(null)}
        onOpenNote={openNote}
        pendingShare={pendingShare?.chatId === chatId ? pendingShare.media : undefined}
        clearPendingShare={clearPendingShare}
      />
    ) : tab === "lists" && noteId ? (
      <NoteEditor key={noteId} noteId={noteId} onBack={wide ? null : () => setNoteId(null)} />
    ) : null;

  const list =
    tab === "chats" ? (
      <ChatList selected={chatId} onOpen={openChat} />
    ) : tab === "lists" ? (
      <NotesView selected={noteId} onOpen={openNote} />
    ) : (
      <FamilyView onOpenPerson={(id) => void openPerson(id)} />
    );

  const tabs = (rail: boolean) => (
    <>
      <TabButton
        rail={rail}
        active={tab === "chats"}
        label="Chats"
        icon={MessageCircle}
        badge={unread}
        onClick={() => setTab("chats")}
      />
      <TabButton rail={rail} active={tab === "lists"} label="Lists" icon={ListChecks} onClick={() => setTab("lists")} />
      <TabButton rail={rail} active={tab === "family"} label="Family" icon={Users} onClick={() => setTab("family")} />
    </>
  );

  const share = (
    <ShareSheet
      onPicked={(id, media) => {
        if (media.length) setPendingShare({ chatId: id, media });
        openChat(id);
      }}
    />
  );

  if (wide) {
    return (
      <div className="flex h-full bg-bg">
        <nav className="safe-top flex flex-col items-center gap-2 border-r-2 border-line bg-surface py-3">
          <Kit mood="wave" size={56} />
          <div className="mt-2 flex flex-col gap-2">{tabs(true)}</div>
        </nav>
        {tab === "family" ? (
          <main className="scroll-area flex-1">{list}</main>
        ) : (
          <>
            <aside
              className={`scroll-area shrink-0 border-r-2 border-line ${tab === "lists" ? "w-[460px]" : "w-[380px]"}`}
            >
              {list}
            </aside>
            <main className="min-w-0 flex-1">{detail ?? <Empty tab={tab} />}</main>
          </>
        )}
        {share}
        <Toasts />
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col bg-bg">
      {detail ? (
        <div className="min-h-0 flex-1">{detail}</div>
      ) : (
        <>
          <main className="safe-top scroll-area min-h-0 flex-1">{list}</main>
          <nav className="safe-bottom flex border-t-2 border-line bg-surface px-2 pt-1.5 pb-1">{tabs(false)}</nav>
        </>
      )}
      {share}
      <Toasts />
    </div>
  );
}
