// Editing a note or list. Changes save by themselves a moment after typing stops.

import {
  ArrowDown,
  ArrowLeft,
  ArrowUp,
  CheckSquare,
  ExternalLink,
  Heading,
  Link2,
  List,
  Send,
  Trash2,
  Type,
  Users,
} from "lucide-react";
import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { api, errorText, openLink } from "../lib/api";
import { useBack } from "../lib/back";
import { normalize } from "../lib/links";
import {
  insertAfter,
  isEmpty,
  markEdited,
  moveBlock,
  NOTE_EMOJI,
  previewLines,
  progress,
  removeBlock,
  setMeta,
  updateBlock,
  visibleBlocks,
} from "../lib/notes";
import { memberById, otherMember, useAppState } from "../lib/state";
import { COLORS, type Block, type BlockKind, type Member, type Note } from "../lib/types";
import { Kit } from "./Kit";
import { Avatar, Button, Confetti, IconButton, Sheet, toast } from "./ui";

const SAVE_AFTER_MS = 600;

const KINDS: { kind: BlockKind; label: string; icon: typeof Type }[] = [
  { kind: "check", label: "Checkbox", icon: CheckSquare },
  { kind: "text", label: "Text", icon: Type },
  { kind: "bullet", label: "Bullet", icon: List },
  { kind: "link", label: "Link", icon: Link2 },
  { kind: "heading", label: "Heading", icon: Heading },
];

export function NoteEditor({ noteId, onBack }: { noteId: string; onBack: (() => void) | null }) {
  const stored = useAppState((s) => s.notes.find((n) => n.id === noteId));
  const me = useAppState((s) => s.account?.me.id ?? "");
  const members = useAppState((s) => s.members);
  const [draft, setDraft] = useState<Note | null>(stored ?? null);
  const [focus, setFocus] = useState<string | null>(null);
  const [emojiOpen, setEmojiOpen] = useState(false);
  const [sharing, setSharing] = useState(false);
  const [celebrate, setCelebrate] = useState(0);
  const lastSent = useRef<Note | null>(stored ?? null);
  const timer = useRef<number | undefined>(undefined);
  const inputs = useRef(new Map<string, HTMLInputElement | HTMLTextAreaElement>());

  useBack(!!onBack, () => onBack?.());

  // Someone else's edit (or the hub's merge of ours) arrives: take it, unless there are edits here not yet saved.
  useEffect(() => {
    if (!stored) return;
    setDraft((d) => {
      if (!d || d.id !== stored.id) return stored;
      const unsaved = lastSent.current !== d;
      if (unsaved) return d;
      lastSent.current = stored;
      return stored;
    });
  }, [stored]);

  useEffect(() => {
    if (focus) inputs.current.get(focus)?.focus();
  }, [focus]);

  // Save what's pending when leaving the note.
  const draftRef = useRef(draft);
  draftRef.current = draft;
  useEffect(
    () => () => {
      window.clearTimeout(timer.current);
      const d = draftRef.current;
      if (d && d !== lastSent.current) void api.saveNote(d).catch(() => {});
    },
    [],
  );

  if (!draft) {
    return (
      <div className="grid h-full place-items-center p-8 text-center text-ink-soft">
        <div>
          <Kit mood="sleep" size={120} />
          <p>This list isn't here anymore.</p>
          {onBack && <Button onClick={onBack}>Back</Button>}
        </div>
      </div>
    );
  }

  const owner = memberById(members, draft.ownerId);
  const mineToOwn = draft.ownerId === me;
  const blocks = visibleBlocks(draft);
  const { done, total } = progress(draft);

  const change = (edited: Note) => {
    const next = markEdited(draft, edited, me);
    setDraft(next);
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => {
      lastSent.current = next;
      void api.saveNote(next).catch((e) => toast(errorText(e), "error"));
    }, SAVE_AFTER_MS);
  };

  const toggle = (b: Block) => {
    const next = updateBlock(draft, b.id, { checked: !b.checked });
    change(next);
    const p = progress(next);
    if (!b.checked && p.total >= 2 && p.done === p.total) setCelebrate((c) => c + 1);
  };

  const addLine = (afterId: string | null, kind: BlockKind) => {
    const [next, id] = insertAfter(draft, afterId, kind);
    change(next);
    setFocus(id);
  };

  const onKey = (e: KeyboardEvent, b: Block, index: number) => {
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      addLine(b.id, b.kind === "heading" ? "text" : b.kind);
    } else if (e.key === "Backspace" && !b.text && !b.url && blocks.length > 1) {
      e.preventDefault();
      change(removeBlock(draft, b.id));
      setFocus(blocks[Math.max(0, index - 1)].id === b.id ? null : blocks[Math.max(0, index - 1)].id);
    } else if (e.key === "ArrowUp" && e.altKey) {
      e.preventDefault();
      change(moveBlock(draft, b.id, -1));
    } else if (e.key === "ArrowDown" && e.altKey) {
      e.preventDefault();
      change(moveBlock(draft, b.id, 1));
    }
  };

  const shareTo = async (chatId: string) => {
    setSharing(false);
    try {
      // Saved first, so the hub has it when the card arrives.
      window.clearTimeout(timer.current);
      lastSent.current = draft;
      await api.saveNote(draft);
      await api.sendMessage(chatId, {
        kind: "note",
        noteId: draft.id,
        title: draft.title,
        emoji: draft.emoji,
        preview: previewLines(draft, 6),
        shared: draft.shared,
      });
      toast("Shared! 🎉");
    } catch (e) {
      toast(errorText(e), "error");
    }
  };

  const remove = async () => {
    if (!window.confirm(`Delete "${draft.title || "this note"}"? It can't be brought back.`)) return;
    window.clearTimeout(timer.current);
    lastSent.current = draft;
    await api.deleteNote(draft.id);
    onBack?.();
  };

  return (
    <div className={`tint-${draft.color} relative flex h-full min-h-0 flex-col`}>
      {celebrate > 0 && <Confetti key={celebrate} />}
      <header
        className="safe-top flex items-center gap-1 border-b-2 border-line px-2 py-2"
        style={{ background: "linear-gradient(90deg, var(--tint-soft), var(--surface))" }}
      >
        {onBack && (
          <IconButton label="Back" onClick={onBack}>
            <ArrowLeft />
          </IconButton>
        )}
        <button
          className="grid size-12 shrink-0 place-items-center rounded-2xl text-3xl hover:bg-surface"
          onClick={() => setEmojiOpen((o) => !o)}
          aria-label="Pick an emoji"
        >
          {draft.emoji}
        </button>
        <input
          value={draft.title}
          onChange={(e) => change(setMeta(draft, { title: e.target.value }))}
          placeholder="Give it a name"
          maxLength={200}
          className="min-w-0 flex-1 bg-transparent px-1 text-2xl font-black outline-none"
        />
        <IconButton label="Send to a chat" onClick={() => setSharing(true)} disabled={isEmpty(draft)}>
          <Send />
        </IconButton>
        {mineToOwn && (
          <IconButton label="Delete" onClick={() => void remove()}>
            <Trash2 />
          </IconButton>
        )}
      </header>

      <div className="scroll-area flex-1 px-4 py-4">
        <div className="mx-auto max-w-2xl">
          {emojiOpen && (
            <div className="anim-pop card mb-4 p-3">
              <div className="flex flex-wrap gap-1">
                {NOTE_EMOJI.map((e) => (
                  <button
                    key={e}
                    className="grid size-12 place-items-center rounded-xl text-2xl hover:bg-surface-2"
                    onClick={() => {
                      change(setMeta(draft, { emoji: e }));
                      setEmojiOpen(false);
                    }}
                  >
                    {e}
                  </button>
                ))}
              </div>
              <div className="mt-3 flex gap-2">
                {COLORS.map((c) => (
                  <button
                    key={c}
                    aria-label={c}
                    onClick={() => change(setMeta(draft, { color: c }))}
                    className={`tint-${c} size-10 rounded-full transition ${draft.color === c ? "ring-4 ring-ink/30" : ""}`}
                    style={{ background: "var(--tint)" }}
                  />
                ))}
              </div>
            </div>
          )}

          <div className="mb-4 flex flex-wrap items-center gap-2">
            {mineToOwn ? (
              <button
                onClick={() => change(setMeta(draft, { shared: !draft.shared }))}
                className={`inline-flex items-center gap-2 rounded-full px-4 py-2 font-bold transition ${
                  draft.shared ? "bg-[var(--tint)] text-white" : "bg-surface-2 text-ink-soft"
                }`}
              >
                <Users size={18} />{" "}
                {draft.shared ? "Family list: everyone can edit" : "Just me: tap to share with the family"}
              </button>
            ) : (
              <span className="inline-flex items-center gap-2 rounded-full bg-surface-2 px-3 py-1.5 text-sm font-bold text-ink-soft">
                <Avatar member={owner} size={22} /> {owner?.name}'s family list
              </span>
            )}
            {total > 0 && (
              <span className="ml-auto font-bold text-ink-soft">
                {done} of {total} done
              </span>
            )}
          </div>
          {total > 0 && (
            <div className="mb-4 h-3 overflow-hidden rounded-full bg-surface-2">
              <div
                className="h-full rounded-full bg-[var(--tint)] transition-all"
                style={{ width: `${(done / total) * 100}%` }}
              />
            </div>
          )}
          {celebrate > 0 && total > 0 && done === total && (
            <div className="anim-pop mb-4 flex items-center gap-3 rounded-3xl bg-[var(--tint-soft)] p-3">
              <Kit mood="party" size={72} />
              <span className="text-xl font-black">All done! Amazing! 🎉</span>
            </div>
          )}

          <div className="flex flex-col gap-1">
            {blocks.map((b, index) => (
              <BlockRow
                key={b.id}
                block={b}
                editor={draft.shared ? memberById(members, b.editedBy) : undefined}
                isMe={b.editedBy === me}
                focused={focus === b.id}
                onFocus={() => setFocus(b.id)}
                register={(el) => {
                  if (el) inputs.current.set(b.id, el);
                  else inputs.current.delete(b.id);
                }}
                onText={(text) => change(updateBlock(draft, b.id, { text }))}
                onUrl={(url) => change(updateBlock(draft, b.id, { url }))}
                onToggle={() => toggle(b)}
                onKind={(kind) => change(updateBlock(draft, b.id, { kind }))}
                onMove={(dir) => change(moveBlock(draft, b.id, dir))}
                onDelete={() => change(removeBlock(draft, b.id))}
                onKey={(e) => onKey(e, b, index)}
              />
            ))}
          </div>

          <div className="mt-4 flex flex-wrap gap-2">
            {KINDS.map(({ kind, label, icon: Icon }) => (
              <button
                key={kind}
                onClick={() => addLine(null, kind)}
                className="inline-flex items-center gap-1.5 rounded-full border-2 border-dashed border-line px-4 py-2 font-bold text-ink-soft hover:border-[var(--tint)] hover:text-ink"
              >
                <Icon size={18} /> {label}
              </button>
            ))}
          </div>
        </div>
      </div>

      {sharing && <ShareToChat onPick={(id) => void shareTo(id)} onClose={() => setSharing(false)} />}
    </div>
  );
}

function BlockRow({
  block,
  editor,
  isMe,
  focused,
  onFocus,
  register,
  onText,
  onUrl,
  onToggle,
  onKind,
  onMove,
  onDelete,
  onKey,
}: {
  block: Block;
  /** Who last changed this line, on a family list. */
  editor: Member | undefined;
  isMe: boolean;
  focused: boolean;
  onFocus: () => void;
  register: (el: HTMLInputElement | null) => void;
  onText: (t: string) => void;
  onUrl: (u: string) => void;
  onToggle: () => void;
  onKind: (k: BlockKind) => void;
  onMove: (dir: -1 | 1) => void;
  onDelete: () => void;
  onKey: (e: KeyboardEvent) => void;
}) {
  const link = block.kind === "link" ? normalize(block.url) : null;
  const showEditor = editor && block.kind !== "heading" && (block.text.trim() || block.url.trim());
  const editorName = isMe ? "You" : editor?.name;
  return (
    <div className={`rounded-2xl px-2 py-1 transition ${focused ? "bg-[var(--tint-soft)]" : ""}`}>
      <div className="flex items-center gap-2">
        {block.kind === "check" && (
          <button
            onClick={onToggle}
            aria-label={block.checked ? "Untick" : "Tick"}
            className={`grid size-9 shrink-0 place-items-center rounded-xl border-[3px] text-lg font-black transition active:scale-90 ${
              block.checked ? "border-[var(--tint)] bg-[var(--tint)] text-white" : "border-line bg-surface"
            }`}
          >
            {block.checked ? "✓" : ""}
          </button>
        )}
        {block.kind === "bullet" && (
          <span className="grid size-9 shrink-0 place-items-center text-2xl text-[var(--tint)]">•</span>
        )}
        {block.kind === "link" && <Link2 className="ml-1.5 shrink-0 text-[var(--tint)]" />}
        <input
          ref={register}
          value={block.text}
          onFocus={onFocus}
          onChange={(e) => onText(e.target.value)}
          onKeyDown={onKey}
          placeholder={
            block.kind === "heading"
              ? "Heading"
              : block.kind === "link"
                ? "What is it?"
                : block.kind === "check"
                  ? "Something to do"
                  : "Write here"
          }
          className={`min-w-0 flex-1 bg-transparent py-1.5 outline-none ${
            block.kind === "heading" ? "text-xl font-black" : "text-lg"
          } ${block.kind === "check" && block.checked ? "text-ink-soft line-through" : ""}`}
        />
        {showEditor && (
          <span
            className="flex max-w-24 shrink-0 items-center gap-1 text-xs font-bold text-ink-soft"
            title={`Last edited by ${editorName}`}
          >
            <Avatar member={editor} size={20} />
            <span className="truncate">{editorName}</span>
          </span>
        )}
        {link && (
          <IconButton label="Open link" onClick={() => void openLink(link)} className="size-10">
            <ExternalLink size={18} />
          </IconButton>
        )}
      </div>
      {block.kind === "link" && (
        <input
          value={block.url}
          onFocus={onFocus}
          onChange={(e) => onUrl(e.target.value)}
          onKeyDown={onKey}
          placeholder="https://…"
          inputMode="url"
          autoCapitalize="off"
          className="mb-1 ml-9 w-[calc(100%-2.25rem)] rounded-xl bg-surface-2 px-3 py-1.5 text-sm text-ink-soft outline-none"
        />
      )}
      {focused && (
        <div className="anim-pop mt-1 mb-1 flex flex-wrap items-center gap-1 pl-1">
          {KINDS.map(({ kind, label, icon: Icon }) => (
            <button
              key={kind}
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => onKind(kind)}
              aria-label={`Make it a ${label.toLowerCase()}`}
              title={label}
              className={`grid size-9 place-items-center rounded-xl ${block.kind === kind ? "bg-[var(--tint)] text-white" : "hover:bg-surface"}`}
            >
              <Icon size={18} />
            </button>
          ))}
          <span className="mx-1 h-6 w-px bg-line" />
          <button
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => onMove(-1)}
            aria-label="Move up"
            className="grid size-9 place-items-center rounded-xl hover:bg-surface"
          >
            <ArrowUp size={18} />
          </button>
          <button
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => onMove(1)}
            aria-label="Move down"
            className="grid size-9 place-items-center rounded-xl hover:bg-surface"
          >
            <ArrowDown size={18} />
          </button>
          <button
            onMouseDown={(e) => e.preventDefault()}
            onClick={onDelete}
            aria-label="Delete line"
            className="grid size-9 place-items-center rounded-xl text-danger hover:bg-surface"
          >
            <Trash2 size={18} />
          </button>
        </div>
      )}
    </div>
  );
}

/** Picks a chat to send something to. */
export function ShareToChat({
  onPick,
  onClose,
  title = "Send to…",
}: {
  onPick: (chatId: string) => void;
  onClose: () => void;
  title?: ReactNode;
}) {
  const chats = useAppState((s) => s.chats);
  const members = useAppState((s) => s.members);
  const me = useAppState((s) => s.account?.me.id ?? "");
  const family = chats.find((c) => c.kind === "family");
  const people = members.filter((m) => m.id !== me);

  const pickPerson = async (memberId: string) => {
    try {
      const existing = chats.find((c) => c.kind === "direct" && otherMember(c, me) === memberId);
      onPick(existing ? existing.id : (await api.openDirect(memberId)).id);
    } catch (e) {
      toast(errorText(e), "error");
    }
  };

  return (
    <Sheet title={title} onClose={onClose}>
      <div className="flex flex-col gap-2">
        {family && (
          <button
            onClick={() => onPick(family.id)}
            className="card tint-sun flex items-center gap-3 p-3 text-left"
            style={{ background: "var(--tint-soft)" }}
          >
            <Kit mood="hug" size={48} />
            <span className="text-lg font-black">Family chat</span>
          </button>
        )}
        {people.map((m) => (
          <button
            key={m.id}
            onClick={() => void pickPerson(m.id)}
            className="card flex items-center gap-3 p-3 text-left"
          >
            <Avatar member={m} size={48} />
            <span className="text-lg font-extrabold">{m.name}</span>
          </button>
        ))}
      </div>
    </Sheet>
  );
}
