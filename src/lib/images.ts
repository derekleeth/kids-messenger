// The small picture the chat shows for a photo or video, made on the page (desktop, and pictures pasted or dropped):
// shrunk to a sensible size and drawn again onto a canvas, which leaves the camera's EXIF block behind. The full
// photo or video goes too, cleaned of its location by the Rust side (`send_media`). On Android the photo picker and
// the share sheet make these previews natively instead (Outgoing.kt).

const MAX_SIDE = 1920;
const QUALITY = 0.86;

export interface Prepared {
  bytes: Uint8Array;
  width: number;
  height: number;
  /** The picture's own size, the right way up. */
  fullWidth: number;
  fullHeight: number;
}

/** The size a picture is shrunk to, keeping its shape. */
export function fitWithin(width: number, height: number, max = MAX_SIDE): { width: number; height: number } {
  const scale = Math.min(1, max / Math.max(width, height));
  return { width: Math.max(1, Math.round(width * scale)), height: Math.max(1, Math.round(height * scale)) };
}

type Decoded = { source: CanvasImageSource; width: number; height: number; close: () => void };

/** The picture as something a canvas can draw, the right way up. */
async function decode(blob: Blob): Promise<Decoded> {
  // `from-image` applies the photo's rotation before it's dropped with the rest of the EXIF data.
  try {
    const bitmap = await createImageBitmap(blob, { imageOrientation: "from-image" });
    return { source: bitmap, width: bitmap.width, height: bitmap.height, close: () => bitmap.close() };
  } catch {
    // Android's WebView can't decode some camera photos this way (Pixel "Ultra HDR" JPEGs, with a gain map), though
    // an <img> shows them fine. An <img> also applies the photo's rotation.
  }
  const url = URL.createObjectURL(blob);
  try {
    const img = new Image();
    img.src = url;
    await img.decode();
    return { source: img, width: img.naturalWidth, height: img.naturalHeight, close: () => URL.revokeObjectURL(url) };
  } catch {
    URL.revokeObjectURL(url);
    throw new Error("That picture can't be opened. Try a screenshot of it, or a different picture.");
  }
}

export async function prepareImage(blob: Blob): Promise<Prepared> {
  const picture = await decode(blob);
  const { width, height } = fitWithin(picture.width, picture.height);
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("Couldn't get the picture ready.");
  // Pictures with see-through parts get a white background instead of black.
  ctx.fillStyle = "#ffffff";
  ctx.fillRect(0, 0, width, height);
  ctx.drawImage(picture.source, 0, 0, width, height);
  picture.close();
  const out = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, "image/jpeg", QUALITY));
  if (!out) throw new Error("Couldn't get the picture ready.");
  return {
    bytes: new Uint8Array(await out.arrayBuffer()),
    width,
    height,
    fullWidth: picture.width,
    fullHeight: picture.height,
  };
}

export interface Poster {
  bytes: Uint8Array | null;
  width: number;
  height: number;
  durationMs: number;
}

/** A still from near the start of a video, and its size and length. `bytes` is null when it can't be drawn (a codec
 * this computer's WebView doesn't have); the video still sends. */
export async function videoPoster(blob: Blob): Promise<Poster> {
  const url = URL.createObjectURL(blob);
  const video = document.createElement("video");
  video.muted = true;
  video.playsInline = true;
  video.preload = "auto";
  try {
    video.src = url;
    await new Promise<void>((resolve, reject) => {
      video.onloadedmetadata = () => resolve();
      video.onerror = () => reject(new Error("That video can't be opened."));
      setTimeout(() => reject(new Error("That video can't be opened.")), 15000);
    });
    const durationMs = Number.isFinite(video.duration) ? Math.round(video.duration * 1000) : 0;
    const width = video.videoWidth;
    const height = video.videoHeight;
    let bytes: Uint8Array | null = null;
    try {
      video.currentTime = Math.min(0.5, video.duration / 2 || 0);
      await new Promise<void>((resolve, reject) => {
        video.onseeked = () => resolve();
        setTimeout(() => reject(new Error("no frame")), 8000);
      });
      const size = fitWithin(width, height);
      const canvas = document.createElement("canvas");
      canvas.width = size.width;
      canvas.height = size.height;
      canvas.getContext("2d")?.drawImage(video, 0, 0, size.width, size.height);
      const out = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, "image/jpeg", QUALITY));
      if (out) bytes = new Uint8Array(await out.arrayBuffer());
    } catch {
      // No still: the message shows a plain video card.
    }
    return { bytes, width: width || 1280, height: height || 720, durationMs };
  } finally {
    video.removeAttribute("src");
    video.load();
    URL.revokeObjectURL(url);
  }
}

/** Photos and videos among picked, pasted or dropped items. */
export function mediaFiles(list: FileList | DataTransferItemList | null | undefined): File[] {
  if (!list) return [];
  const files: File[] = [];
  for (const item of Array.from(list as ArrayLike<File | DataTransferItem>)) {
    const file = item instanceof File ? item : item.kind === "file" ? item.getAsFile() : null;
    if (file && (file.type.startsWith("image/") || file.type.startsWith("video/"))) files.push(file);
  }
  return files;
}
