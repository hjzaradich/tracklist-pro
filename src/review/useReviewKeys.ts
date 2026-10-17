import { useEffect, useRef } from "react";

/** A key the Review screen answers: Enter, S, or a digit from 1 to 9. */
export type ReviewKey = "enter" | "s" | number;

/**
 * Anything that takes the keys for itself: a button (the play buttons sit in
 * every file row, and Enter presses the focused one), a field, a link, a
 * slider, an editable text, or a dialog that is waiting for an answer.
 */
const OWN_KEYS =
  'button, input, select, textarea, summary, a[href], [contenteditable]:not([contenteditable="false"]), ' +
  '[role="button"], [role="slider"], [role="textbox"], [role="combobox"], [role="menuitem"], ' +
  '[role="dialog"], [role="alertdialog"]';

/** A dialog that is waiting for an answer has the keyboard, wherever the focus is. */
const OPEN_DIALOG = '[role="dialog"], [role="alertdialog"], dialog[open]';

/** The key as Review reads it, or `null` for any other key or one with a modifier. */
function reviewKey(event: KeyboardEvent): ReviewKey | null {
  if (event.ctrlKey || event.altKey || event.metaKey || event.shiftKey) return null;
  if (event.key === "Enter") return "enter";
  // With Caps Lock on, S comes as "S" with no Shift.
  if (event.key === "s" || event.key === "S") return "s";
  return /^[1-9]$/.test(event.key) ? Number(event.key) : null;
}

/**
 * Calls `onKey` for Enter, S and the digits 1 to 9 (1bE-2, ROADMAP 1.12
 * "Keyboard"), from anywhere on the screen except a key that belongs to
 * something else: pressed on a button or in a field, with a modifier, while
 * a dialog is open, or already answered by another handler. A held key counts
 * once. `onKey` says whether it did anything: only then is the key taken
 * (`preventDefault`), so a key that does nothing here still does what it does
 * elsewhere.
 *
 * One listener on the window for the life of the component, reading the latest
 * `onKey`, so it is never stale and never added twice.
 */
export function useReviewKeys(onKey: (key: ReviewKey) => boolean) {
  const latest = useRef(onKey);
  useEffect(() => {
    latest.current = onKey;
  });
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const key = reviewKey(event);
      if (key === null || event.defaultPrevented || event.repeat) return;
      const target = event.target;
      if (target instanceof Element && target.closest(OWN_KEYS) !== null) return;
      if (document.querySelector(OPEN_DIALOG) !== null) return;
      if (latest.current(key)) event.preventDefault();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);
}
