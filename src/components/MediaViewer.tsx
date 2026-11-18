// A photo or video, full screen. A photo shows its chat-sized picture straight away and swaps in the full-size one
// once that's downloaded; a video downloads (with progress), then plays. Either can be saved to the Downloads folder
// (computer) or the gallery (Android).

import { Check, Download, LoaderCircle, Play, X } from "lucide-react";
import { useState } from "react";
import { api, errorText } from "../lib/api";
import { durationText, percent, sizeText, useDownload, useMedia } from "../lib/media";
import { toast } from "./ui";

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
      {poster.url && (
        <img src={poster.url} alt="" className="max-h-full max-w-full rounded-xl object-contain opacity-60" />
      )}
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

/** The file that's saved: a photo at full size when there is one, or the video itself. */
function savedId(v: Viewing): string {
  return v.kind === "image" ? (v.fullMediaId ?? v.mediaId) : v.videoMediaId;
}

function SaveButton({ viewing }: { viewing: Viewing }) {
  const id = savedId(viewing);
  const [state, setState] = useState<{ id: string; saving: boolean; saved: boolean }>({
    id,
    saving: false,
    saved: false,
  });
  const current = state.id === id ? state : { id, saving: false, saved: false };
  const what = viewing.kind === "image" ? "photo" : "video";

  async function save(e: React.MouseEvent) {
    e.stopPropagation();
    if (current.saving) return;
    setState({ id, saving: true, saved: false });
    try {
      const saved = await api.saveMedia(id);
      setState({ id, saving: false, saved: true });
      toast(saved.place === "gallery" ? `Saved the ${what} to the gallery` : `Saved to Downloads as ${saved.name}`);
    } catch (err) {
      setState({ id, saving: false, saved: false });
      toast(`Couldn't save the ${what}: ${errorText(err)}`, "error");
    }
  }

  return (
    <button
      className="grid size-11 place-items-center rounded-full bg-white/15 text-white disabled:opacity-70"
      aria-label={current.saving ? `Saving ${what}` : `Save ${what}`}
      title={`Save ${what}`}
      disabled={current.saving}
      onClick={save}
    >
      {current.saving ? <LoaderCircle className="animate-spin" /> : current.saved ? <Check /> : <Download />}
    </button>
  );
}

export function MediaViewer({ viewing, onClose }: { viewing: Viewing; onClose: () => void }) {
  return (
    <div className="fixed inset-0 z-50 grid place-items-center bg-black/90 p-4" onClick={onClose}>
      <div className="absolute right-3 z-10 flex gap-2" style={{ top: "max(12px, env(safe-area-inset-top))" }}>
        <SaveButton viewing={viewing} />
        <button
          className="grid size-11 place-items-center rounded-full bg-white/15 text-white"
          aria-label="Close"
          onClick={onClose}
        >
          <X />
        </button>
      </div>
      {viewing.kind === "image" ? <Photo v={viewing} /> : <Video v={viewing} />}
    </div>
  );
}
