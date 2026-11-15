// Android's Back button steps back one level inside the app (closing a picture, a sheet, a chat) before it leaves.
// MainActivity.kt calls `window.__kitBack()`, which runs the newest handler and says whether one was there.

import { useEffect, useRef } from "react";

const handlers: (() => void)[] = [];

declare global {
  interface Window {
    __kitBack?: () => boolean;
    __kitSharedIn?: () => void;
  }
}

window.__kitBack = () => {
  const handler = handlers[handlers.length - 1];
  if (!handler) return false;
  handler();
  return true;
};

/** While `active`, Back calls `onBack`. The most recently activated handler goes first. */
export function useBack(active: boolean, onBack: () => void) {
  const latest = useRef(onBack);
  useEffect(() => {
    latest.current = onBack;
  });
  useEffect(() => {
    if (!active) return;
    const handler = () => latest.current();
    handlers.push(handler);
    return () => {
      const at = handlers.lastIndexOf(handler);
      if (at >= 0) handlers.splice(at, 1);
    };
  }, [active]);
}
