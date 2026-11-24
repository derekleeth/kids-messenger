// Turns Kit artwork (big JPEGs on a white background, as Gemini makes them) into the small transparent PNGs the app
// uses, named after Kit's mood, plus the app icon source.
//
//   node scripts/kit-art.mjs            (reads art/kit/, writes src/assets/kit/*.png and art/kit/icon.png)
//   npx tauri icon art/kit/icon.png   (then run this script again for the Android adaptive icon)
//
// Add a picture: drop it in art/kit/ and add a line to MOODS.

import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import sharp from "sharp";

const SOURCE = "art/kit";
const OUT = "src/assets/kit";
const SIZE = 512;

/** Source file → mood name (see src/components/Kit.tsx). */
const MOODS = {
  "kit-mascot.jpeg": "happy",
  "kit-waving.jpeg": "wave",
  "kit-sleep.jpeg": "sleep",
  "kit-jumpingconfetti.jpeg": "party",
  "kit-thinking.jpeg": "list", // Kit with a checklist
  "kit-photo.jpeg": "camera",
  "kit-thinking-bulb.jpeg": "thinking",
  "kit-love.jpeg": "love",
  "kit-lol.jpeg": "lol",
  "kit-wow.jpeg": "wow",
  "kit-thumbsup.jpeg": "thumbsup",
  "kit-hug.jpeg": "hug",
  "kit-cool.jpeg": "cool",
};
const ICON_FROM = "kit-mascot.jpeg";

/** Gemini's white sticker outline can't be told apart from the white background (some pictures have a faint grey
 * edge around it, some don't), so it's removed with the background, faint edge and all, and every picture gets the
 * same clean white outline drawn again, this wide (a fraction of the picture's width). */
const OUTLINE_WIDTH = 0.018;

/** White to light grey with no colour: the background, the sticker outline and its faint edge. Kit's cream fur is
 * warmer than this, so it stays. */
const isBackground = (r, g, b) => Math.min(r, g, b) > 196 && Math.max(r, g, b) - Math.min(r, g, b) < 14;

/** Makes the white background (and the old outline) transparent: a flood fill in from the edges, so white inside
 * Kit (eye highlights, chest fur) stays. The pixels just inside the fill fade out a little to keep edges soft. Then
 * draws the new outline. */
async function cutOut(file) {
  const { data, info } = await sharp(file).ensureAlpha().raw().toBuffer({ resolveWithObject: true });
  const { width: w, height: h } = info;
  const seen = new Uint8Array(w * h);
  const stack = [];
  const push = (x, y) => {
    if (x < 0 || y < 0 || x >= w || y >= h) return;
    const i = y * w + x;
    if (seen[i]) return;
    const p = i * 4;
    if (!isBackground(data[p], data[p + 1], data[p + 2])) return;
    seen[i] = 1;
    stack.push(i);
  };
  for (let x = 0; x < w; x++) {
    push(x, 0);
    push(x, h - 1);
  }
  for (let y = 0; y < h; y++) {
    push(0, y);
    push(w - 1, y);
  }
  while (stack.length) {
    const i = stack.pop();
    const x = i % w;
    const y = (i - x) / w;
    push(x + 1, y);
    push(x - 1, y);
    push(x, y + 1);
    push(x, y - 1);
  }
  for (let i = 0; i < w * h; i++) {
    const p = i * 4;
    if (seen[i]) {
      data[p + 3] = 0;
      continue;
    }
    // Soften the rim: a light pixel touching the background is partly see-through.
    const x = i % w;
    const touches = (x > 0 && seen[i - 1]) || (x < w - 1 && seen[i + 1]) || seen[i - w] || seen[i + w];
    if (touches) {
      const light = Math.min(data[p], data[p + 1], data[p + 2]);
      data[p + 3] = Math.round(255 * Math.min(1, Math.max(0.35, (262 - light) / 40)));
    }
  }
  addOutline(data, w, h, Math.round(w * OUTLINE_WIDTH));
  return sharp(data, { raw: { width: w, height: h, channels: 4 } })
    .trim({ background: { r: 0, g: 0, b: 0, alpha: 0 } })
    .png()
    .toBuffer();
}

mkdirSync(OUT, { recursive: true });
/** Paints white, see-through pixels within `width` of Kit opaque (a chamfer distance transform, two passes). */
function addOutline(data, w, h, width) {
  const far = 1e9;
  const dist = new Float32Array(w * h);
  for (let i = 0; i < w * h; i++) dist[i] = data[i * 4 + 3] > 128 ? 0 : far;
  const pass = (i, j, cost) => {
    if (dist[j] + cost < dist[i]) dist[i] = dist[j] + cost;
  };
  for (let y = 0; y < h; y++)
    for (let x = 0; x < w; x++) {
      const i = y * w + x;
      if (x > 0) pass(i, i - 1, 1);
      if (y > 0) {
        pass(i, i - w, 1);
        if (x > 0) pass(i, i - w - 1, 1.414);
        if (x < w - 1) pass(i, i - w + 1, 1.414);
      }
    }
  for (let y = h - 1; y >= 0; y--)
    for (let x = w - 1; x >= 0; x--) {
      const i = y * w + x;
      if (x < w - 1) pass(i, i + 1, 1);
      if (y < h - 1) {
        pass(i, i + w, 1);
        if (x < w - 1) pass(i, i + w + 1, 1.414);
        if (x > 0) pass(i, i + w - 1, 1.414);
      }
    }
  for (let i = 0; i < w * h; i++) {
    const p = i * 4;
    const d = dist[i];
    if (d === 0 || d > width + 1) continue;
    // A soft last pixel, so the new edge isn't jagged.
    const a = Math.round(255 * Math.min(1, width + 1 - d));
    if (a <= data[p + 3]) continue;
    data[p] = data[p + 1] = data[p + 2] = 255;
    data[p + 3] = a;
  }
}

for (const [file, mood] of Object.entries(MOODS)) {
  const cut = await cutOut(join(SOURCE, file));
  const out = join(OUT, `${mood}.png`);
  const { size } = await sharp(cut)
    .resize(SIZE, SIZE, { fit: "contain", background: { r: 0, g: 0, b: 0, alpha: 0 } })
    .png({ compressionLevel: 9, palette: true, quality: 95, effort: 10 })
    .toFile(out);
  console.log(`${file} → ${out} (${Math.round(size / 1024)} KB)`);

  if (file === ICON_FROM) {
    const background = Buffer.from(
      `<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024">
        <defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stop-color="#7dd3fc"/><stop offset="1" stop-color="#a78bfa"/>
        </linearGradient></defs>
        <rect x="32" y="32" width="960" height="960" rx="230" fill="url(#g)"/>
      </svg>`,
    );
    const kit = await sharp(cut)
      .resize(840, 840, { fit: "contain", background: { r: 0, g: 0, b: 0, alpha: 0 } })
      .toBuffer();
    await sharp(background)
      .composite([{ input: kit, left: 92, top: 100 }])
      .png()
      .toFile(join(SOURCE, "icon.png"));
    console.log(`icon → ${join(SOURCE, "icon.png")}`);
  }
}

// Android's adaptive icon: Kit alone, inside the middle 66% that every launcher mask keeps, over a plain colour
// (values/ic_launcher_background.xml). Run after `tauri icon`, which writes a plain version of these.
const ANDROID_RES = "src-tauri/gen/android/app/src/main/res";
const happy = await cutOut(join(SOURCE, ICON_FROM));
for (const [density, px] of Object.entries({ mdpi: 108, hdpi: 162, xhdpi: 216, xxhdpi: 324, xxxhdpi: 432 })) {
  const inner = Math.round(px * 0.64);
  const kit = await sharp(happy)
    .resize(inner, inner, { fit: "contain", background: { r: 0, g: 0, b: 0, alpha: 0 } })
    .toBuffer();
  const at = Math.round((px - inner) / 2);
  await sharp({ create: { width: px, height: px, channels: 4, background: { r: 0, g: 0, b: 0, alpha: 0 } } })
    .composite([{ input: kit, left: at, top: at }])
    .png()
    .toFile(join(ANDROID_RES, `mipmap-${density}`, "ic_launcher_foreground.png"));
}
// Both the square and the round icon point at the adaptive layers. The manifest names a roundIcon, and launchers that
// draw circles (Pixel) take it first: without an adaptive round version they shrink the old flat ic_launcher_round.png
// into a white circle, and Kit ends up tiny.
const adaptive = `<?xml version="1.0" encoding="utf-8"?>
<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">
  <foreground android:drawable="@mipmap/ic_launcher_foreground"/>
  <background android:drawable="@color/ic_launcher_background"/>
</adaptive-icon>`;
for (const name of ["ic_launcher", "ic_launcher_round"])
  writeFileSync(join(ANDROID_RES, "mipmap-anydpi-v26", `${name}.xml`), adaptive);
console.log(`android adaptive icon → ${ANDROID_RES}/mipmap-*/ic_launcher_foreground.png, mipmap-anydpi-v26/*.xml`);
