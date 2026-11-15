// A photo or video, full screen. A photo shows its chat-sized picture straight away and swaps in the full-size one
// once that's downloaded; a video downloads (with progress), then plays.

import { Play, X } from "lucide-react";
import { useState } from "react";
import { durationText, percent, sizeText, useDownload, useMedia } from "../lib/media";

export type Viewing =
  | { kind: "image"; mediaId: string; fullMediaId?: string | null; fullBytes?: number }
  | { kind: "video"; videoMediaId: string; posterMediaId?: string | null; bytes: number; durationMs: number };

function Loading({ mediaId, bytes, what }: { mediaId: string; bytes?: number; what: string }) {
  const p = percent(useDownload(mediaId));
  return (
    <div className="pointer-events-none absolute bottom-6 left-1/2 -translate-x-1/2 rounded-full bg-black/70 px-4 py-2 text-sm font-bold text-white">
      Loading {what}
      {p !== null ? ` ${p}%` : "…"}
      {bytes ? ` · ${sizeText(bytes)}` : ""}
    </div>
  );
}

function Photo({ v }: { v: Extract<Viewing, { kind: "image" }> }) {
  const shown = useMedia(v.mediaId);
  const full = useMedia(v.fullMediaId);
  const [fullReady, setFullReady] = useState(false);
  return (
    <>
      {shown.url && !fullReady && (
        <img src={shown.url} alt="" className="max-h-full max-w-full rounded-xl object-contain" />
      )}
      {full.url && (
        <img
          src={full.url}
          alt=""
          onLoad={() => setFullReady(true)}
          className={`max-h-full max-w-full rounded-xl object-contain ${fullReady ? "" : "hidden"}`}
        />
      )}
      {v.fullMediaId && !fullReady && !full.failed && (
        <Loading mediaId={v.fullMediaId} bytes={v.fullBytes} what="full size" />
      )}
    </>
  );
}

function Video({ v }: { v: Extract<Viewing, { kind: "video" }> }) {
  const poster = useMedia(v.posterMediaId);
  const video = useMedia(v.videoMediaId);
  if (video.url) {
    return (
      <video
        src={video.url}
        poster={poster.url ?? undefined}
        controls
        autoPlay
        playsInline
        className="max-h-full max-w-full rounded-xl"
        onClick={(e) => e.stopPropagation()}
      />
    );
  }
  return (
    <>
      {poster.url && <img src={poster.url} alt="" className="max-h-full max-w-full rounded-xl object-contain opacity-60" />}
      {video.failed ? (
        <div className="absolute rounded-2xl bg-black/70 px-5 py-3 text-center font-bold text-white">
          Couldn't load the video.
          <div className="text-sm font-normal opacity-80">{video.error}</div>
        </div>
      ) : (
        <>
          <div className="absolute grid size-20 place-items-center rounded-full bg-black/50 text-white">
            <Play size={40} />
          </div>
          <Loading mediaId={v.videoMediaId} bytes={v.bytes} what={`video (${durationText(v.durationMs)})`} />
        </>
      )}
    </>
  );
}

export function MediaViewer({ viewing, onClose }: { viewing: Viewing; onClose: () => void }) {
  return (
    <div className="fixed inset-0 z-50 grid place-items-center bg-black/90 p-4" onClick={onClose}>
      <button
        className="absolute right-3 z-10 grid size-11 place-items-center rounded-full bg-white/15 text-white"
        style={{ top: "max(12px, env(safe-area-inset-top))" }}
        aria-label="Close"
        onClick={onClose}
      >
        <X />
      </button>
      {viewing.kind === "image" ? <Photo v={viewing} /> : <Video v={viewing} />}
    </div>
  );
}
