import { describe, expect, it } from "vitest";
import {
  insertAfter,
  isEmpty,
  markEdited,
  moveBlock,
  newNote,
  previewLines,
  progress,
  removeBlock,
  setMeta,
  TEMPLATES,
  updateBlock,
  visibleBlocks,
} from "./notes";

const list = TEMPLATES.find((t) => t.id === "list")!;
const idea = TEMPLATES.find((t) => t.id === "idea")!;

function threeItems() {
  let note = newNote("me", list, 1000);
  const first = note.blocks[0].id;
  note = updateBlock(note, first, { text: "Milk" }, 1001);
  let id: string;
  [note, id] = insertAfter(note, first, "check", 1002);
  note = updateBlock(note, id, { text: "Apples" }, 1003);
  [note, id] = insertAfter(note, id, "check", 1004);
  note = updateBlock(note, id, { text: "Pasta" }, 1005);
  return note;
}

const texts = (n: ReturnType<typeof newNote>) => visibleBlocks(n).map((b) => b.text);

describe("notes", () => {
  it("starts from a template", () => {
    const note = newNote("me", idea, 5);
    expect(note.ownerId).toBe("me");
    expect(note.shared).toBe(false);
    expect(visibleBlocks(note).map((b) => b.kind)).toEqual(["heading", "text", "heading", "bullet"]);
    expect(
      isEmpty(
        newNote(
          "me",
          TEMPLATES.find((t) => t.id === "blank")!,
        ),
      ),
    ).toBe(true);
    expect(isEmpty(note)).toBe(false);
  });

  it("inserts lines in order, including in the middle", () => {
    const start = threeItems();
    expect(texts(start)).toEqual(["Milk", "Apples", "Pasta"]);
    const milk = visibleBlocks(start)[0].id;
    const [added, id] = insertAfter(start, milk, "check", 2000);
    const note = updateBlock(added, id, { text: "Eggs" }, 2001);
    expect(texts(note)).toEqual(["Milk", "Eggs", "Apples", "Pasta"]);
  });

  it("moves lines up and down, and not past the ends", () => {
    let note = threeItems();
    const [milk, , pasta] = visibleBlocks(note).map((b) => b.id);
    note = moveBlock(note, milk, 1, 3000);
    expect(texts(note)).toEqual(["Apples", "Milk", "Pasta"]);
    note = moveBlock(note, pasta, -1, 3001);
    note = moveBlock(note, pasta, -1, 3002);
    expect(texts(note)).toEqual(["Pasta", "Apples", "Milk"]);
    expect(moveBlock(note, pasta, -1, 3003)).toBe(note);
  });

  it("deletes by marking, so the line can't come back", () => {
    let note = threeItems();
    const apples = visibleBlocks(note)[1];
    note = removeBlock(note, apples.id, 4000);
    expect(texts(note)).toEqual(["Milk", "Pasta"]);
    const gone = note.blocks.find((b) => b.id === apples.id)!;
    expect(gone.deleted).toBe(true);
    expect(gone.updatedAt).toBeGreaterThan(apples.updatedAt);
  });

  it("always moves the edit time forward, even with a slow clock", () => {
    const note = threeItems();
    const b = visibleBlocks(note)[0];
    const edited = updateBlock(note, b.id, { checked: true }, 0);
    expect(edited.blocks.find((x) => x.id === b.id)!.updatedAt).toBe(b.updatedAt + 1);
    expect(setMeta(note, { title: "x" }, 0).metaUpdatedAt).toBe(note.metaUpdatedAt + 1);
  });

  it("counts ticked items, ignoring empty unticked lines", () => {
    let note = threeItems();
    const [milk] = visibleBlocks(note).map((b) => b.id);
    [note] = insertAfter(note, null, "check", 5000);
    note = updateBlock(note, milk, { checked: true }, 5001);
    expect(progress(note)).toEqual({ done: 1, total: 3 });
  });

  it("previews the first lines", () => {
    let note = threeItems();
    note = updateBlock(note, visibleBlocks(note)[0].id, { checked: true }, 6000);
    expect(previewLines(note, 2)).toEqual(["☑ Milk", "☐ Apples"]);
  });

  it("marks who changed a line, but not for moving it", () => {
    const note = threeItems();
    const [milk, apples] = visibleBlocks(note).map((b) => b.id);
    const editor = (n: typeof note, id: string) => n.blocks.find((b) => b.id === id)!.editedBy;

    const ticked = markEdited(note, updateBlock(note, milk, { checked: true }, 7000), "mom");
    expect(editor(ticked, milk)).toBe("mom");
    expect(editor(ticked, apples)).toBe("");

    const moved = markEdited(ticked, moveBlock(ticked, milk, 1, 7001), "dad");
    expect(editor(moved, milk)).toBe("mom");

    const [added, id] = insertAfter(moved, null, "check", 7002);
    expect(editor(markEdited(moved, added, "dad"), id)).toBe("dad");
  });
});
