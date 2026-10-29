/**
 * The time in a file that a pointer at `clientX` is over, when the waveform
 * spans `box` (its left edge and width in pixels): the left edge is the
 * start, the right edge the end, and a pointer beyond either is held there.
 * An element with no width has nowhere to point at: 0.
 */
export function timeAtPointer(clientX: number, box: { left: number; width: number }, duration: number): number {
  if (!(box.width > 0) || !(duration > 0)) return 0;
  const fraction = (clientX - box.left) / box.width;
  return Math.min(1, Math.max(0, fraction)) * duration;
}

/** How far the arrow keys move the playhead, in seconds, and how far Page Up and Page Down do. */
export const ARROW_STEP = 5;
export const PAGE_STEP = 30;

/**
 * Where the keys on the seek bar go from `position`: arrows a few seconds,
 * Page Up and Down more, Home and End to the ends. `null` for a key that
 * isn't the bar's.
 */
export function timeAfterKey(key: string, position: number, duration: number): number | null {
  const to = (seconds: number) => Math.min(duration, Math.max(0, seconds));
  switch (key) {
    case "ArrowLeft":
    case "ArrowDown":
      return to(position - ARROW_STEP);
    case "ArrowRight":
    case "ArrowUp":
      return to(position + ARROW_STEP);
    case "PageDown":
      return to(position - PAGE_STEP);
    case "PageUp":
      return to(position + PAGE_STEP);
    case "Home":
      return 0;
    case "End":
      return duration;
    default:
      return null;
  }
}
