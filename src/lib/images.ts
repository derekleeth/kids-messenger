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

export async function prepareImage(blob: Blob): Promise<Prepared> {
  // `from-image` applies the photo's rotation before it's dropped with the rest of the EXIF data.
  const bitmap = await createImageBitmap(blob, { imageOrientation: "from-image" });
  const { width, height } = fitWithin(bitmap.width, bitmap.height);
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("Couldn't get the picture ready.");
  // Pictures with see-through parts get a white background instead of black.
  ctx.fillStyle = "#ffffff";
  ctx.fillRect(0, 0, width, height);
  ctx.drawImage(bitmap, 0, 0, width, height);
  bitmap.close();
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
