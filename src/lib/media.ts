// Photos and videos by ID. The Rust side keeps each one as a file (downloading it from the hub the first time), and the
// page shows it straight from disk through Tauri's asset protocol, so a big video never crosses the IPC bridge.

import { convertFileSrc } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { api, isTauri, onEvent } from "./api";

/** A file on this device as a URL the page can load. (The browser mock hands out URLs already.) */
export function fileUrl(path: string): string {
  return isTauri ? convertFileSrc(path) : path;
}

const urls = new Map<string, Promise<string>>();

export function mediaUrl(id: string): Promise<string> {
  let url = urls.get(id);
  if (!url) {
    url = api.mediaFile(id).then(fileUrl);
    url.catch(() => urls.delete(id));
    urls.set(id, url);
  }
  return url;
}

/** The picture's URL once it's here. `null` IDs (and `lazy` ones, until `lazy` is false) load nothing. */
export function useMedia(
  id: string | null | undefined,
  lazy = false,
): { url: string | null; failed: boolean; error: string | null } {
  const [state, setState] = useState<{ id: string | null; url: string | null; error: string | null }>({
    id: null,
    url: null,
    error: null,
  });
  useEffect(() => {
    if (!id || lazy) return;
    let live = true;
    mediaUrl(id).then(
      (url) => live && setState({ id, url, error: null }),
      (e) => live && setState({ id, url: null, error: String(e) }),
    );
    return () => {
      live = false;
    };
  }, [id, lazy]);
  if (state.id !== id) return { url: null, failed: false, error: null };
  return { url: state.url, failed: state.error !== null, error: state.error };
}

// ---- Progress ---------------------------------------------------------------------------------------------------

type Progress = { done: number; total: number };
const downloads = new Map<string, Progress>();
const uploads = new Map<string, Progress>();
const listeners = new Set<() => void>();
let listening = false;

function listen() {
  if (listening) return;
  listening = true;
  const changed = () => listeners.forEach((l) => l());
  void onEvent<{ mediaId: string; received: number; total: number }>("download", (p) => {
    downloads.set(p.mediaId, { done: p.received, total: p.total });
    changed();
  });
  void onEvent<{ messageId: string; sent: number; total: number }>("upload", (p) => {
    uploads.set(p.messageId, { done: p.sent, total: p.total });
    changed();
  });
}

function useProgress(map: Map<string, Progress>, key: string | null | undefined): Progress | null {
  const [, bump] = useState(0);
  useEffect(() => {
    listen();
    const l = () => bump((n) => n + 1);
    listeners.add(l);
    return () => void listeners.delete(l);
  }, []);
  return key ? (map.get(key) ?? null) : null;
}

/** How far a download of this media has got. */
export const useDownload = (mediaId: string | null | undefined) => useProgress(downloads, mediaId);
/** How far the upload of this message's photo or video has got. */
export const useUpload = (messageId: string | null | undefined) => useProgress(uploads, messageId);

export function percent(p: Progress | null): number | null {
  return p && p.total > 0 ? Math.min(100, Math.floor((p.done / p.total) * 100)) : null;
}

export function sizeText(bytes: number): string {
  if (bytes >= 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
  if (bytes >= 1024 * 1024) return `${Math.round(bytes / (1024 * 1024))} MB`;
  return `${Math.max(1, Math.round(bytes / 1024))} KB`;
}

export function durationText(ms: number): string {
  const s = Math.round(ms / 1000);
  const m = Math.floor(s / 60);
  return `${m}:${String(s % 60).padStart(2, "0")}`;
}
