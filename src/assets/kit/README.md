# Kit artwork

The PNGs here are made by `scripts/kit-art.mjs` from the original pictures in `art/kit/` (Gemini's JPEGs on white).
The script cuts out the white background, draws one even white sticker outline, trims, shrinks to 512×512 and names
each file after Kit's mood. Any mood
without a picture uses the SVG drawing in `src/components/Kit.tsx`.

| File           | Used for                                         | Have it? |
| -------------- | ------------------------------------------------ | -------- |
| `happy.png`    | Kit's usual face, loading screen                 | ✅       |
| `wave.png`     | Welcome screen, "wave" sticker                   | ✅       |
| `sleep.png`    | Empty chats and lists, "sleepy" sticker          | ✅       |
| `party.png`    | Finishing a list, "party" sticker                | ✅       |
| `list.png`     | Lists screen                                     | ✅       |
| `camera.png`   | Sending pictures, shares from other apps         | ✅       |
| `thinking.png` | "thinking" sticker (lightbulb)                   | ✅       |
| `love.png`     | "love" sticker                                   | ✅       |
| `lol.png`      | "lol" sticker                                    | ✅       |
| `wow.png`      | "wow" sticker                                    | ✅       |
| `thumbsup.png` | "thumbs up" sticker                              | ✅       |
| `hug.png`      | "hug" sticker, family chat                       | ✅       |
| `cool.png`     | "cool" sticker                                   | ✅       |

## Adding more

1. Save the picture in `art/kit/` and add a line for it to `MOODS` in `scripts/kit-art.mjs`.
2. Run `node scripts/kit-art.mjs`.
3. If the icon picture changed: `npx tauri icon art/kit/icon.png`, then `node scripts/kit-art.mjs` again (it
replaces the Android adaptive icon that `tauri icon` writes with one that fits launcher masks).

## Original prompt for Gemini

> A cute baby lynx cub mascot named Kit for a kids' messaging app. Chibi proportions: big round head, huge sparkly
> eyes, tiny pink nose, soft fluffy cheek fur, and the lynx's signature black ear tufts on oversized ears. Fur is warm
> golden-beige with faint spots, and the ear tufts and tail tip have a subtle indigo-to-cyan gradient (#a5b4fc to
> #22d3ee) as a nod to the TerminalLynx logo. Friendly, playful, holding a small glowing chat bubble. Flat vector
> illustration with clean bold outlines, soft cel shading, a bright cheerful palette, and no text. Centered on a plain
> white background with transparent-friendly edges, at sticker-style 1:1 square framing.
