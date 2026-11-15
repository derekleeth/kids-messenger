// Pictures by ID, as object URLs, fetched once through the Rust side (which caches them on disk).

import { useEffect, useState } from "react";
import { api } from "./api";

const urls = new Map<string, Promise<string>>();

function typeOf(bytes: Uint8Array): string {
  if (bytes[0] === 0xff && bytes[1] === 0xd8) return "image/jpeg";
  if (bytes[0] === 0x89 && bytes[1] === 0x50) return "image/png";
  if (bytes[0] === 0x47 && bytes[1] === 0x49) return "image/gif";
  if (bytes[8] === 0x57 && bytes[9] === 0x45) return "image/webp";
  if (bytes[0] === 0x3c) return "image/svg+xml";
  return "application/octet-stream";
}

export function mediaUrl(id: string): Promise<string> {
  let url = urls.get(id);
  if (!url) {
    url = api.media(id).then((buf) => {
      const bytes = new Uint8Array(buf);
      return URL.createObjectURL(new Blob([bytes], { type: typeOf(bytes) }));
    });
    url.catch(() => urls.delete(id));
    urls.set(id, url);
  }
  return url;
}

export function useMedia(id: string | null | undefined): { url: string | null; failed: boolean } {
  const [state, setState] = useState<{ id: string | null; url: string | null; failed: boolean }>({
    id: null,
    url: null,
    failed: false,
  });
  useEffect(() => {
    if (!id) return;
    let live = true;
    mediaUrl(id).then(
      (url) => live && setState({ id, url, failed: false }),
      () => live && setState({ id, url: null, failed: true }),
    );
    return () => {
      live = false;
    };
  }, [id]);
  return state.id === id ? { url: state.url, failed: state.failed } : { url: null, failed: false };
}
