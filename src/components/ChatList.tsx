// The chats: the whole family first, then one with each person.

import { api, errorText } from "../lib/api";
import { shortWhen } from "../lib/format";
import { lastMessage, memberById, summary, unreadCount, useAppState } from "../lib/state";
import type { Chat } from "../lib/types";
import { Kit } from "./Kit";
import { Avatar, toast } from "./ui";

function Badge({ n }: { n: number }) {
  if (n === 0) return null;
  return (
    <span className="anim-pop grid h-7 min-w-7 place-items-center rounded-full bg-[var(--bubblegum)] px-2 text-sm font-black text-white">
      {n > 99 ? "99+" : n}
    </span>
  );
}

export function ChatList({ selected, onOpen }: { selected: string | null; onOpen: (chatId: string) => void }) {
  const account = useAppState((s) => s.account)!;
  const members = useAppState((s) => s.members);
  const chats = useAppState((s) => s.chats);
  const messages = useAppState((s) => s.messages);
  const me = account.me.id;

  const family = chats.find((c) => c.kind === "family");
  const directWith = (memberId: string): Chat | undefined =>
    chats.find((c) => c.kind === "direct" && c.memberIds.includes(memberId) && c.memberIds.includes(me));
  const others = members.filter((m) => m.id !== me);

  const openPerson = async (memberId: string) => {
    const existing = directWith(memberId);
    if (existing) return onOpen(existing.id);
    try {
      const chat = await api.openDirect(memberId);
      onOpen(chat.id);
    } catch (e) {
      toast(errorText(e), "error");
    }
  };

  const familyLast = family ? lastMessage(messages[family.id]) : undefined;

  return (
    <div className="flex flex-col gap-3 p-4">
      {family && (
        <button
          onClick={() => onOpen(family.id)}
          className={`card tint-sun relative flex items-center gap-4 overflow-hidden p-4 text-left transition active:scale-[0.99] ${
            selected === family.id ? "ring-4 ring-[var(--sun)]" : ""
          }`}
          style={{ background: "linear-gradient(120deg, var(--sun-soft), var(--surface) 70%)" }}
        >
          <Kit mood="hug" size={68} />
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <span className="text-xl font-black">Family chat</span>
              <span className="text-sm text-ink-soft">{familyLast ? shortWhen(familyLast.createdAt) : ""}</span>
            </div>
            <div className="flex -space-x-2 py-1">
              {family.memberIds.slice(0, 8).map((id) => (
                <Avatar key={id} member={memberById(members, id)} size={28} />
              ))}
            </div>
            <p className="truncate text-ink-soft">
              {familyLast
                ? `${familyLast.senderId === me ? "You" : (memberById(members, familyLast.senderId)?.name ?? "")}: ${summary(familyLast)}`
                : "Say hi to everyone!"}
            </p>
          </div>
          <Badge n={unreadCount(family, messages[family.id], me)} />
        </button>
      )}

      <h3 className="mt-2 px-1 text-sm font-black tracking-wide text-ink-soft uppercase">Just the two of you</h3>
      {others.length === 0 && (
        <p className="px-1 text-ink-soft">
          Nobody else here yet. {account.me.role === "parent" ? "Add people in Family." : ""}
        </p>
      )}
      {others.map((member) => {
        const chat = directWith(member.id);
        const last = chat ? lastMessage(messages[chat.id]) : undefined;
        const unread = chat ? unreadCount(chat, messages[chat.id], me) : 0;
        return (
          <button
            key={member.id}
            onClick={() => void openPerson(member.id)}
            className={`card tint-${member.color} flex items-center gap-3 p-3 text-left transition active:scale-[0.99] ${
              chat && selected === chat.id ? "ring-4 ring-[var(--tint)]" : ""
            }`}
          >
            <Avatar member={member} size={52} />
            <div className="min-w-0 flex-1">
              <div className="flex items-baseline gap-2">
                <span className="text-lg font-extrabold">{member.name}</span>
                {member.role === "parent" && <span className="text-xs font-bold text-ink-soft">parent</span>}
                <span className="ml-auto text-sm text-ink-soft">{last ? shortWhen(last.createdAt) : ""}</span>
              </div>
              <p className={`truncate ${unread ? "font-extrabold" : "text-ink-soft"}`}>
                {last ? `${last.senderId === me ? "You: " : ""}${summary(last)}` : `Send ${member.name} a message`}
              </p>
            </div>
            <Badge n={unread} />
          </button>
        );
      })}
    </div>
  );
}
