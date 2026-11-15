// Small building blocks used across the app.

import { X } from "lucide-react";
import { useEffect, useState, type ButtonHTMLAttributes, type ReactNode } from "react";
import { useBack } from "../lib/back";
import type { Member, Status } from "../lib/types";

export function Avatar({ member, size = 44, ring = false }: { member?: Member; size?: number; ring?: boolean }) {
  return (
    <div
      className={`tint-${member?.color ?? "grape"} grid shrink-0 place-items-center rounded-full`}
      style={{
        width: size,
        height: size,
        background: "var(--tint-soft)",
        boxShadow: ring ? "0 0 0 3px var(--tint)" : "inset 0 0 0 2px var(--tint)",
        fontSize: size * 0.55,
        lineHeight: 1,
      }}
      aria-hidden
    >
      {member?.avatar ?? "🙂"}
    </div>
  );
}

type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  tone?: "brand" | "soft" | "plain" | "danger";
  big?: boolean;
};

export function Button({ tone = "brand", big = false, className = "", ...rest }: ButtonProps) {
  const tones = {
    brand: "bg-brand text-brand-ink shadow-[0_3px_0_rgba(0,0,0,0.15)] active:translate-y-px",
    soft: "bg-brand-soft text-ink",
    plain: "bg-transparent text-ink hover:bg-brand-soft",
    danger: "bg-danger text-white shadow-[0_3px_0_rgba(0,0,0,0.15)]",
  };
  return (
    <button
      type="button"
      className={`inline-flex items-center justify-center gap-2 rounded-full font-extrabold transition disabled:opacity-50 ${
        big ? "min-h-14 px-7 text-lg" : "min-h-11 px-5"
      } ${tones[tone]} ${className}`}
      {...rest}
    />
  );
}

export function IconButton({
  label,
  className = "",
  children,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { label: string }) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      className={`grid size-12 shrink-0 place-items-center rounded-full text-ink transition hover:bg-brand-soft active:scale-95 disabled:opacity-40 ${className}`}
      {...rest}
    >
      {children}
    </button>
  );
}

/** A panel over the page: a centred card on wide screens, a sheet from the bottom on phones. */
export function Sheet({
  title,
  onClose,
  children,
  wide = false,
}: {
  title: ReactNode;
  onClose: () => void;
  children: ReactNode;
  wide?: boolean;
}) {
  useBack(true, onClose);
  useEffect(() => {
    const key = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [onClose]);
  return (
    <div
      className="fixed inset-0 z-40 flex items-end justify-center bg-[rgba(30,20,60,0.45)] sm:items-center"
      onMouseDown={(e) => e.target === e.currentTarget && onClose()}
    >
      <div
        role="dialog"
        aria-modal
        className={`anim-rise card safe-bottom flex max-h-[92vh] w-full flex-col rounded-b-none sm:rounded-b-[22px] ${
          wide ? "sm:max-w-2xl" : "sm:max-w-md"
        }`}
      >
        <div className="flex items-center gap-2 px-5 pt-4 pb-2">
          <h2 className="flex-1 text-xl font-black">{title}</h2>
          <IconButton label="Close" onClick={onClose}>
            <X />
          </IconButton>
        </div>
        <div className="scroll-area px-5 pb-5">{children}</div>
      </div>
    </div>
  );
}

export function Field({ label, children, hint }: { label: string; children: ReactNode; hint?: ReactNode }) {
  return (
    <label className="block">
      <span className="mb-1 block text-sm font-extrabold text-ink-soft">{label}</span>
      {children}
      {hint && <span className="mt-1 block text-sm text-ink-soft">{hint}</span>}
    </label>
  );
}

export const inputClass =
  "w-full rounded-2xl border-2 border-line bg-surface-2 px-4 py-3 text-lg outline-none focus:border-brand";

export function StatusPill({ status }: { status: Status }) {
  if (status.state === "online" || status.state === "signedOut") return null;
  const connecting = status.state === "connecting";
  return (
    <div
      className={`rounded-full px-3 py-1 text-sm font-bold ${
        connecting ? "bg-[var(--sun-soft)] text-ink" : "bg-[var(--bubblegum-soft)] text-ink"
      }`}
      title={status.detail ?? undefined}
    >
      {connecting ? "Connecting…" : (status.detail ?? "Offline: messages will send later")}
    </div>
  );
}

// ---- Toasts ----

type Toast = { id: number; text: string; tone: "info" | "error" };
let toasts: Toast[] = [];
const toastListeners = new Set<(t: Toast[]) => void>();
let nextToast = 1;

export function toast(text: string, tone: Toast["tone"] = "info") {
  const t = { id: nextToast++, text, tone };
  toasts = [...toasts, t];
  toastListeners.forEach((l) => l(toasts));
  setTimeout(() => {
    toasts = toasts.filter((x) => x.id !== t.id);
    toastListeners.forEach((l) => l(toasts));
  }, 4200);
}

export function Toasts() {
  const [list, setList] = useState<Toast[]>(toasts);
  useEffect(() => {
    toastListeners.add(setList);
    return () => {
      toastListeners.delete(setList);
    };
  }, []);
  return (
    <div className="pointer-events-none fixed inset-x-0 bottom-24 z-50 flex flex-col items-center gap-2 px-4">
      {list.map((t) => (
        <div
          key={t.id}
          role="status"
          className={`anim-pop max-w-md rounded-2xl px-5 py-3 font-bold shadow-lg ${
            t.tone === "error" ? "bg-danger text-white" : "bg-ink text-bg"
          }`}
        >
          {t.text}
        </div>
      ))}
    </div>
  );
}

/** A little burst of confetti, for finishing a list. */
export function Confetti() {
  const colors = ["var(--sun)", "var(--sky)", "var(--bubblegum)", "var(--mint)", "var(--grape)", "var(--tangerine)"];
  return (
    <div className="pointer-events-none absolute inset-x-0 top-0 h-64 overflow-hidden" aria-hidden>
      {Array.from({ length: 28 }, (_, i) => (
        <span
          key={i}
          className="absolute top-0 block h-3 w-2 rounded-sm"
          style={{
            left: `${(i * 37) % 100}%`,
            background: colors[i % colors.length],
            animation: `confetti-fall ${1.2 + (i % 5) * 0.2}s ease-in ${(i % 7) * 0.05}s forwards`,
          }}
        />
      ))}
    </div>
  );
}

export const AVATARS = ["🦊", "🐰", "🦁", "🐼", "🐯", "🦄", "🐸", "🐙", "🦋", "🐶", "🐱", "🐨", "🦖", "🐝", "🐧", "🦉"];
