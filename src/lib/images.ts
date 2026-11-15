// Getting a picture ready to send: shrunk to a sensible size and drawn again onto a canvas. Re-drawing means only the
// pixels are sent: the camera's EXIF block, with the GPS position of where the picture was taken, is left behind.

const MAX_SIDE = 1920;
const QUALITY = 0.86;

export interface Prepared {
  bytes: Uint8Array;
  width: number;
  height: number;
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
  return { bytes: new Uint8Array(await out.arrayBuffer()), width, height };
}

/** Pictures among pasted or dropped items. */
export function imageFiles(list: FileList | DataTransferItemList | null | undefined): File[] {
  if (!list) return [];
  const files: File[] = [];
  for (const item of Array.from(list as ArrayLike<File | DataTransferItem>)) {
    const file = item instanceof File ? item : item.kind === "file" ? item.getAsFile() : null;
    if (file && file.type.startsWith("image/")) files.push(file);
  }
  return files;
}
