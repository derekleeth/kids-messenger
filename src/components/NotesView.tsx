// Lists and notes: my own, and family lists everyone shares.

import { Plus, Users } from "lucide-react";
import { useState } from "react";
import { api, errorText } from "../lib/api";
import { ago } from "../lib/format";
import { newNote, previewLines, progress, TEMPLATES } from "../lib/notes";
import { memberById, useAppState } from "../lib/state";
import type { Note } from "../lib/types";
import { Kit } from "./Kit";
import { Avatar, Button, Sheet, toast } from "./ui";

function updatedAt(n: Note) {
  return n.blocks.reduce((t, b) => Math.max(t, b.updatedAt), n.metaUpdatedAt);
}

function NoteCard({ note, selected, onOpen }: { note: Note; selected: boolean; onOpen: () => void }) {
  const members = useAppState((s) => s.members);
  const me = useAppState((s) => s.account?.me.id);
  const { done, total } = progress(note);
  const owner = memberById(members, note.ownerId);
  return (
    <button
      onClick={onOpen}
      className={`card tint-${note.color} flex flex-col p-4 text-left transition active:scale-[0.98] ${
        selected ? "ring-4 ring-[var(--tint)]" : ""
      }`}
      style={{ background: "linear-gradient(160deg, var(--tint-soft), var(--surface) 75%)" }}
    >
      <div className="flex items-start gap-2">
        <span className="text-3xl leading-none">{note.emoji}</span>
        <span className="line-clamp-2 flex-1 text-lg leading-tight font-black">{note.title || "Untitled"}</span>
        {note.shared && <Users size={18} className="mt-1 shrink-0 text-ink-soft" aria-label="Family list" />}
      </div>
      <div className="mt-2 min-h-12 space-y-0.5 text-[0.95rem] text-ink-soft">
        {previewLines(note, 3).map((line, i) => (
          <div key={i} className="truncate">
            {line}
          </div>
        ))}
      </div>
      {total > 0 && (
        <div className="mt-3 h-2.5 overflow-hidden rounded-full bg-surface-2">
          <div
            className="h-full rounded-full bg-[var(--tint)] transition-all"
            style={{ width: `${(done / total) * 100}%` }}
          />
        </div>
      )}
      <div className="mt-2 flex items-center gap-1.5 text-xs text-ink-soft">
        {owner && note.ownerId !== me && <Avatar member={owner} size={18} />}
        {note.ownerId !== me && owner ? `${owner.name} · ` : ""}
        {ago(updatedAt(note))}
      </div>
    </button>
  );
}

export function NotesView({ selected, onOpen }: { selected: string | null; onOpen: (noteId: string) => void }) {
  const account = useAppState((s) => s.account)!;
  const notes = useAppState((s) => s.notes);
  const [choosing, setChoosing] = useState(false);
  const me = account.me.id;
  const sorted = [...notes].sort((a, b) => updatedAt(b) - updatedAt(a));
  const mine = sorted.filter((n) => n.ownerId === me);
  const family = sorted.filter((n) => n.ownerId !== me);

  const create = async (templateId: string) => {
    const template = TEMPLATES.find((t) => t.id === templateId)!;
    const note = newNote(me, template);
    setChoosing(false);
    try {
      await api.saveNote(note);
      onOpen(note.id);
    } catch (e) {
      toast(errorText(e), "error");
    }
  };

  return (
    <div className="p-4">
      <div className="mb-4 flex items-center gap-3">
        <Kit mood="list" size={64} />
        <div className="flex-1">
          <h2 className="text-2xl font-black">Lists & notes</h2>
          <p className="text-ink-soft">Ideas, wish lists, cool sites…</p>
        </div>
        <Button onClick={() => setChoosing(true)}>
          <Plus /> New
        </Button>
      </div>

      {notes.length === 0 && (
        <div className="card flex flex-col items-center p-8 text-center">
          <Kit mood="thinking" size={140} />
          <p className="mt-2 text-lg font-bold">No lists yet!</p>
          <p className="mb-4 text-ink-soft">Make a list of cool sites, a wish list, or write down a big idea.</p>
          <Button big onClick={() => setChoosing(true)}>
            <Plus /> Make one
          </Button>
        </div>
      )}

      {mine.length > 0 && (
        <>
          <h3 className="mb-2 px-1 text-sm font-black tracking-wide text-ink-soft uppercase">Mine</h3>
          <div className="mb-6 grid grid-cols-[repeat(auto-fill,minmax(210px,1fr))] gap-3">
            {mine.map((n) => (
              <NoteCard key={n.id} note={n} selected={selected === n.id} onOpen={() => onOpen(n.id)} />
            ))}
          </div>
        </>
      )}
      {family.length > 0 && (
        <>
          <h3 className="mb-2 px-1 text-sm font-black tracking-wide text-ink-soft uppercase">Family lists</h3>
          <div className="grid grid-cols-[repeat(auto-fill,minmax(210px,1fr))] gap-3">
            {family.map((n) => (
              <NoteCard key={n.id} note={n} selected={selected === n.id} onOpen={() => onOpen(n.id)} />
            ))}
          </div>
        </>
      )}

      {choosing && (
        <Sheet title="Make a new…" onClose={() => setChoosing(false)}>
          <div className="grid grid-cols-2 gap-3">
            {TEMPLATES.map((t) => (
              <button
                key={t.id}
                onClick={() => void create(t.id)}
                className={`card tint-${t.color} flex flex-col items-center gap-1 p-5 transition active:scale-95`}
                style={{ background: "var(--tint-soft)" }}
              >
                <span className="text-4xl">{t.emoji}</span>
                <span className="text-lg font-black">{t.label}</span>
              </button>
            ))}
          </div>
        </Sheet>
      )}
    </div>
  );
}
