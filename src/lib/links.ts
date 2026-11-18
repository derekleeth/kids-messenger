// Finding web links in messages (the same rules as crates/kit-protocol/src/links.rs): only http(s), and a bare
// "www." address gets https://.

export type Part = { text: string; url?: string };

const TRAILING = /[.,!?)\]'";:]+$/;

export function normalize(raw: string): string | null {
  const word = raw.trim().replace(TRAILING, "");
  const lower = word.toLowerCase();
  let url: string;
  if (lower.startsWith("https://") || lower.startsWith("http://")) url = word;
  else if (lower.startsWith("www.")) url = `https://${word}`;
  else return null;
  try {
    const parsed = new URL(url);
    if (!parsed.hostname.includes(".") && !parsed.hostname.startsWith("[")) return null;
  } catch {
    return null;
  }
  return url;
}

/** Splits text into plain parts and link parts, keeping every character. */
export function splitLinks(text: string): Part[] {
  const parts: Part[] = [];
  const re = /(?:https?:\/\/|www\.)[^\s<>]+/gi;
  let last = 0;
  for (const match of text.matchAll(re)) {
    const trimmed = match[0].replace(TRAILING, "");
    const url = normalize(trimmed);
    const start = match.index ?? 0;
    if (!url) continue;
    if (start > last) parts.push({ text: text.slice(last, start) });
    parts.push({ text: trimmed, url });
    last = start + trimmed.length;
  }
  if (last < text.length) parts.push({ text: text.slice(last) });
  return parts;
}

export function firstLink(text: string): string | null {
  return splitLinks(text).find((p) => p.url)?.url ?? null;
}

export function site(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, "");
  } catch {
    return url;
  }
}

/** A message that's only emoji (up to three) shows big. */
export function isBigEmoji(text: string): boolean {
  const t = text.trim();
  if (!t || t.length > 24) return false;
  return /^(?:\p{Extended_Pictographic}(?:️|‍\p{Extended_Pictographic}|\p{Emoji_Modifier})*\s*){1,3}$/u.test(t);
}

/** `kitchat://join?hub=…&also=…&code=…`, as a parent's QR code holds it. `also` is any further hub addresses. */
export function parseJoinLink(text: string): { hub: string; also: string[]; code: string } | null {
  const t = text.trim();
  if (!t.toLowerCase().startsWith("kitchat://join")) return null;
  const params = new URLSearchParams(t.slice(t.indexOf("?") + 1));
  const hub = params.get("hub");
  const code = params.get("code");
  return hub && code ? { hub, also: params.getAll("also").filter(Boolean), code } : null;
}
