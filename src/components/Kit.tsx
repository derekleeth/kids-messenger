// Kit, the baby lynx: the app's mascot, in a mood for each moment (and each sticker).
//
// Kit is drawn here as SVG. Pictures made for a mood (for example with Gemini) can replace the drawing: put PNGs named
// after the mood in src/assets/kit/ (wave.png, sleep.png, party.png …) and they're used instead. See
// src/assets/kit/README.md.

import type { CSSProperties } from "react";

export type KitMood =
  | "happy"
  | "wave"
  | "sleep"
  | "party"
  | "love"
  | "lol"
  | "wow"
  | "thinking"
  | "thumbsup"
  | "hug"
  | "cool"
  | "list"
  | "camera";

/** Stickers are Kit moods (kit-protocol STICKERS) with friendlier names. */
export const STICKER_MOOD: Record<string, KitMood> = {
  wave: "wave",
  love: "love",
  lol: "lol",
  wow: "wow",
  sleepy: "sleep",
  party: "party",
  thinking: "thinking",
  thumbsup: "thumbsup",
  hug: "hug",
  cool: "cool",
};

const pictures = import.meta.glob<string>("../assets/kit/*.png", { eager: true, import: "default", query: "?url" });

function pictureFor(mood: KitMood): string | undefined {
  return pictures[`../assets/kit/${mood}.png`];
}

const FUR = "#f2b872";
const LINE = "#5b3a1e";
const SPOT = "#c97c3c";
const LIGHT = "#fff4e4";
const EYE = "#22163d";
const PINK = "#ff6f91";

function Eyes({ mood }: { mood: KitMood }) {
  const stroke = { stroke: EYE, strokeWidth: 6, strokeLinecap: "round" as const, fill: "none" };
  switch (mood) {
    case "sleep":
      return (
        <g {...stroke}>
          <path d="M56 106 q14 10 28 0" />
          <path d="M116 106 q14 10 28 0" />
        </g>
      );
    case "party":
    case "hug":
    case "thumbsup":
      return (
        <g {...stroke}>
          <path d="M56 112 q14 -16 28 0" />
          <path d="M116 112 q14 -16 28 0" />
        </g>
      );
    case "lol":
      return (
        <g {...stroke}>
          <path d="M58 100 l20 8 l-20 8" />
          <path d="M142 100 l-20 8 l20 8" />
        </g>
      );
    case "love":
      return (
        <g fill="#ff4f7b" stroke={LINE} strokeWidth="2">
          <path d="M70 124 C50 110 52 92 64 94 C68 94 70 98 70 100 C70 98 72 94 76 94 C88 92 90 110 70 124 Z" />
          <path d="M130 124 C110 110 112 92 124 94 C128 94 130 98 130 100 C130 98 132 94 136 94 C148 92 150 110 130 124 Z" />
        </g>
      );
    case "cool":
      return (
        <g>
          <path d="M44 100 h112" stroke="#111" strokeWidth="5" />
          <rect x="48" y="96" width="44" height="26" rx="10" fill="#111" />
          <rect x="108" y="96" width="44" height="26" rx="10" fill="#111" />
          <path d="M56 102 l10 0" stroke="#fff" strokeWidth="3" strokeLinecap="round" opacity="0.7" />
          <path d="M116 102 l10 0" stroke="#fff" strokeWidth="3" strokeLinecap="round" opacity="0.7" />
        </g>
      );
    case "wow":
      return (
        <g>
          <circle cx="70" cy="108" r="19" fill="#fff" stroke={EYE} strokeWidth="4" />
          <circle cx="130" cy="108" r="19" fill="#fff" stroke={EYE} strokeWidth="4" />
          <circle cx="70" cy="110" r="10" fill={EYE} />
          <circle cx="130" cy="110" r="10" fill={EYE} />
          <circle cx="74" cy="105" r="3.5" fill="#fff" />
          <circle cx="134" cy="105" r="3.5" fill="#fff" />
        </g>
      );
    case "thinking":
      return (
        <g>
          <ellipse cx="70" cy="110" rx="15" ry="18" fill={EYE} />
          <ellipse cx="130" cy="110" rx="15" ry="18" fill={EYE} />
          <circle cx="76" cy="100" r="6" fill="#fff" />
          <circle cx="136" cy="100" r="6" fill="#fff" />
        </g>
      );
    default:
      return (
        <g>
          <ellipse cx="70" cy="110" rx="16" ry="19" fill={EYE} />
          <ellipse cx="130" cy="110" rx="16" ry="19" fill={EYE} />
          <circle cx="75" cy="102" r="6" fill="#fff" />
          <circle cx="135" cy="102" r="6" fill="#fff" />
          <circle cx="66" cy="117" r="3" fill="#fff" />
          <circle cx="126" cy="117" r="3" fill="#fff" />
        </g>
      );
  }
}

function Mouth({ mood }: { mood: KitMood }) {
  if (mood === "lol" || mood === "party") {
    return <path d="M86 140 Q100 166 114 140 Z" fill="#8a2142" stroke={LINE} strokeWidth="3" strokeLinejoin="round" />;
  }
  if (mood === "wow") return <ellipse cx="100" cy="148" rx="8" ry="10" fill="#8a2142" stroke={LINE} strokeWidth="3" />;
  if (mood === "sleep")
    return <path d="M94 144 q6 4 12 0" stroke={LINE} strokeWidth="4" strokeLinecap="round" fill="none" />;
  if (mood === "thinking")
    return <path d="M92 146 q8 -4 16 2" stroke={LINE} strokeWidth="4" strokeLinecap="round" fill="none" />;
  return (
    <path
      d="M100 139 q-6 12 -16 6 M100 139 q6 12 16 6"
      stroke={LINE}
      strokeWidth="4"
      strokeLinecap="round"
      fill="none"
    />
  );
}

function Extras({ mood }: { mood: KitMood }) {
  switch (mood) {
    case "wave":
      return (
        <g className="anim-sway" style={{ transformOrigin: "168px 190px" }}>
          <ellipse
            cx="172"
            cy="150"
            rx="14"
            ry="22"
            fill={FUR}
            stroke={LINE}
            strokeWidth="4"
            transform="rotate(20 172 150)"
          />
          <path
            d="M184 120 q8 -6 14 -2 M188 132 q8 -2 12 4"
            stroke="#ffc233"
            strokeWidth="4"
            strokeLinecap="round"
            fill="none"
          />
        </g>
      );
    case "sleep":
      return (
        <g fill="#8b6cf7" fontWeight="900" fontFamily="Nunito Variable, sans-serif">
          <text x="150" y="58" fontSize="26">
            z
          </text>
          <text x="168" y="38" fontSize="20">
            z
          </text>
          <text x="182" y="22" fontSize="14">
            z
          </text>
        </g>
      );
    case "party":
      return (
        <g>
          <path d="M84 40 L100 -4 L116 40 Z" fill="#f76ba5" stroke={LINE} strokeWidth="3" strokeLinejoin="round" />
          <circle cx="100" cy="-6" r="6" fill="#ffc233" stroke={LINE} strokeWidth="2" />
          <circle cx="94" cy="26" r="3" fill="#fff" />
          <circle cx="106" cy="14" r="3" fill="#fff" />
          {[
            [24, 30, "#3fa9f5"],
            [176, 40, "#2fc99a"],
            [30, 170, "#ffc233"],
            [178, 168, "#f76ba5"],
            [16, 96, "#8b6cf7"],
            [188, 98, "#ff8a3d"],
          ].map(([x, y, c]) => (
            <rect
              key={`${x}-${y}`}
              x={Number(x)}
              y={Number(y)}
              width="9"
              height="9"
              rx="2"
              fill={String(c)}
              transform={`rotate(25 ${x} ${y})`}
            />
          ))}
        </g>
      );
    case "love":
      return (
        <path
          d="M170 50 C160 42 162 32 168 33 C170 33 171 35 171 36 C171 35 172 33 174 33 C180 32 182 42 170 50 Z"
          fill="#ff4f7b"
        />
      );
    case "lol":
      return <path d="M150 100 q6 10 0 16 q-6 -6 0 -16 Z" fill="#7fd3ff" stroke={LINE} strokeWidth="2" />;
    case "thinking":
      return (
        <g>
          <ellipse cx="128" cy="168" rx="13" ry="10" fill={FUR} stroke={LINE} strokeWidth="4" />
          <circle cx="176" cy="44" r="16" fill="#ffe066" stroke={LINE} strokeWidth="3" />
          <rect x="169" y="58" width="14" height="10" rx="3" fill="#c9c2d9" stroke={LINE} strokeWidth="3" />
          <path d="M150 30 l-8 -6 M176 18 v-10 M198 30 l8 -6" stroke="#ffc233" strokeWidth="4" strokeLinecap="round" />
        </g>
      );
    case "thumbsup":
      return (
        <g>
          <rect x="156" y="128" width="26" height="30" rx="9" fill={FUR} stroke={LINE} strokeWidth="4" />
          <rect x="162" y="104" width="12" height="30" rx="6" fill={FUR} stroke={LINE} strokeWidth="4" />
        </g>
      );
    case "hug":
      return (
        <g>
          <path d="M24 170 q30 30 70 4" stroke={LINE} strokeWidth="18" strokeLinecap="round" fill="none" />
          <path d="M24 170 q30 30 70 4" stroke={FUR} strokeWidth="11" strokeLinecap="round" fill="none" />
          <path d="M176 170 q-30 30 -70 4" stroke={LINE} strokeWidth="18" strokeLinecap="round" fill="none" />
          <path d="M176 170 q-30 30 -70 4" stroke={FUR} strokeWidth="11" strokeLinecap="round" fill="none" />
          <path
            d="M100 196 C88 188 90 180 95 181 C97 181 99 183 100 184 C101 183 103 181 105 181 C110 180 112 188 100 196 Z"
            fill="#ff4f7b"
          />
        </g>
      );
    case "list":
      return (
        <g transform="rotate(8 168 150)">
          <rect x="146" y="112" width="48" height="60" rx="6" fill="#fff" stroke={LINE} strokeWidth="4" />
          <path
            d="M155 128 l4 4 l7 -8 M155 144 l4 4 l7 -8"
            stroke="#2fc99a"
            strokeWidth="4"
            strokeLinecap="round"
            fill="none"
          />
          <path d="M172 129 h14 M172 145 h14 M156 160 h30" stroke="#c9c2d9" strokeWidth="4" strokeLinecap="round" />
        </g>
      );
    case "camera":
      return (
        <g>
          <rect x="140" y="132" width="56" height="40" rx="9" fill="#3fa9f5" stroke={LINE} strokeWidth="4" />
          <circle cx="168" cy="152" r="12" fill="#fff" stroke={LINE} strokeWidth="4" />
          <circle cx="168" cy="152" r="5" fill={EYE} />
          <rect x="150" y="124" width="16" height="10" rx="3" fill="#3fa9f5" stroke={LINE} strokeWidth="3" />
        </g>
      );
    default:
      return null;
  }
}

/** The SVG drawing of Kit's head (the whole sticker), in a mood. */
function KitDrawing({ mood }: { mood: KitMood }) {
  return (
    <svg viewBox="-10 -20 220 230" width="100%" height="100%" role="img" aria-label={`Kit the lynx (${mood})`}>
      <defs>
        <linearGradient id="kit-tuft" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#22d3ee" />
          <stop offset="1" stopColor="#6366f1" />
        </linearGradient>
      </defs>
      {/* tail */}
      <path d="M150 186 q40 6 36 -30" stroke={LINE} strokeWidth="18" strokeLinecap="round" fill="none" />
      <path d="M150 186 q40 6 36 -30" stroke={FUR} strokeWidth="11" strokeLinecap="round" fill="none" />
      <circle cx="186" cy="154" r="7" fill="#312e81" />
      {/* body */}
      <ellipse cx="100" cy="186" rx="52" ry="28" fill={FUR} stroke={LINE} strokeWidth="5" />
      <ellipse cx="100" cy="190" rx="28" ry="16" fill={LIGHT} />
      <ellipse cx="74" cy="206" rx="15" ry="9" fill={LIGHT} stroke={LINE} strokeWidth="4" />
      <ellipse cx="126" cy="206" rx="15" ry="9" fill={LIGHT} stroke={LINE} strokeWidth="4" />
      {/* ears with their tufts */}
      <path d="M30 86 L52 14 Q56 8 62 14 L100 60 Z" fill={FUR} stroke={LINE} strokeWidth="5" strokeLinejoin="round" />
      <path
        d="M170 86 L148 14 Q144 8 138 14 L100 60 Z"
        fill={FUR}
        stroke={LINE}
        strokeWidth="5"
        strokeLinejoin="round"
      />
      <path d="M44 74 L56 32 L82 60 Z" fill="#ffb3c7" />
      <path d="M156 74 L144 32 L118 60 Z" fill="#ffb3c7" />
      <path d="M55 16 L46 -14 L64 10 Z" fill="url(#kit-tuft)" stroke="#312e81" strokeWidth="3" strokeLinejoin="round" />
      <path
        d="M145 16 L154 -14 L136 10 Z"
        fill="url(#kit-tuft)"
        stroke="#312e81"
        strokeWidth="3"
        strokeLinejoin="round"
      />
      {/* cheek ruff and head */}
      <path
        d="M30 118 L10 138 L32 140 L18 160 L46 152 L100 160 L154 152 L182 160 L168 140 L190 138 L170 118 Z"
        fill="#fde7c8"
        stroke={LINE}
        strokeWidth="5"
        strokeLinejoin="round"
      />
      <ellipse cx="100" cy="110" rx="76" ry="62" fill={FUR} stroke={LINE} strokeWidth="5" />
      <path
        d="M92 52 q2 12 0 20 M100 50 q0 14 0 22 M108 52 q-2 12 0 20"
        stroke={SPOT}
        strokeWidth="5"
        strokeLinecap="round"
        fill="none"
      />
      <circle cx="40" cy="104" r="4" fill={SPOT} />
      <circle cx="160" cy="104" r="4" fill={SPOT} />
      <circle cx="48" cy="90" r="3" fill={SPOT} />
      <circle cx="152" cy="90" r="3" fill={SPOT} />
      <ellipse cx="100" cy="140" rx="40" ry="27" fill={LIGHT} />
      <Eyes mood={mood} />
      <ellipse
        cx="44"
        cy="134"
        rx="12"
        ry="7"
        fill="#ff8fab"
        opacity={mood === "hug" || mood === "love" ? 0.95 : 0.55}
      />
      <ellipse
        cx="156"
        cy="134"
        rx="12"
        ry="7"
        fill="#ff8fab"
        opacity={mood === "hug" || mood === "love" ? 0.95 : 0.55}
      />
      <path
        d="M90 128 Q100 122 110 128 Q104 138 100 139 Q96 138 90 128 Z"
        fill={PINK}
        stroke={LINE}
        strokeWidth="3"
        strokeLinejoin="round"
      />
      <Mouth mood={mood} />
      <Extras mood={mood} />
    </svg>
  );
}

export function Kit({
  mood = "happy",
  size = 120,
  className = "",
  style,
}: {
  mood?: KitMood;
  size?: number;
  className?: string;
  style?: CSSProperties;
}) {
  // A mood with no picture yet keeps the drawing, so stickers stay different from each other.
  const picture = pictureFor(mood);
  return (
    <div className={`inline-block shrink-0 select-none ${className}`} style={{ width: size, height: size, ...style }}>
      {picture ? (
        <img src={picture} alt={`Kit the lynx (${mood})`} className="h-full w-full object-contain" draggable={false} />
      ) : (
        <KitDrawing mood={mood} />
      )}
    </div>
  );
}
