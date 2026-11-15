// Photos and videos waiting in the composer, and getting each one ready for `send_media`.
//
// Two kinds: files the page has as Blobs (picked on desktop, pasted, dropped), which are staged into the app's cache
// here; and files Android already copied there with a preview (photo picker, share sheet), which go as they are.

import { api } from "./api";
import { prepareImage, videoPoster } from "./images";
import { fileUrl } from "./media";
import type { OutgoingMedia, SharedItem } from "./types";

export interface Attachment {
  id: number;
  kind: "image" | "video";
  /** A picture for the composer: an object URL for a Blob, the preview file for a ready item ("" if there's none). */
  url: string;
  blob?: Blob;
  ready?: SharedItem;
}

let nextId = 1;

export function fromBlobs(files: Blob[]): Attachment[] {
  return files.map((blob) => ({
    id: nextId++,
    kind: blob.type.startsWith("video/") ? "video" : "image",
    url: URL.createObjectURL(blob),
    blob,
  }));
}

export function fromItems(items: SharedItem[]): Attachment[] {
  return items
    .filter((i) => i.kind === "image" || i.kind === "video")
    .map((ready) => ({
      id: nextId++,
      kind: ready.kind as "image" | "video",
      url: ready.previewPath ? fileUrl(ready.previewPath) : "",
      ready,
    }));
}

export function release(a: Attachment) {
  if (a.blob) URL.revokeObjectURL(a.url);
}

async function stage(blob: Blob | Uint8Array): Promise<string> {
  const bytes = blob instanceof Uint8Array ? blob : new Uint8Array(await blob.arrayBuffer());
  return api.stageFile(bytes);
}

export async function toOutgoing(a: Attachment, caption: string): Promise<OutgoingMedia> {
  if (a.ready) {
    const r = a.ready;
    return {
      kind: a.kind,
      path: r.path,
      previewPath: r.previewPath ?? null,
      width: r.width,
      height: r.height,
      durationMs: r.durationMs,
      caption,
    };
  }
  const blob = a.blob!;
  if (a.kind === "video") {
    const poster = await videoPoster(blob);
    return {
      kind: "video",
      path: await stage(blob),
      previewPath: poster.bytes ? await stage(poster.bytes) : null,
      width: poster.width,
      height: poster.height,
      durationMs: poster.durationMs,
      caption,
    };
  }
  const prepared = await prepareImage(blob);
  // A GIF goes as it is, so it keeps moving.
  const gif = blob.type === "image/gif";
  return {
    kind: "image",
    path: await stage(blob),
    previewPath: gif ? null : await stage(prepared.bytes),
    width: prepared.fullWidth,
    height: prepared.fullHeight,
    durationMs: 0,
    caption,
  };
}
