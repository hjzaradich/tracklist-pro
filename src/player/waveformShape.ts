/** The height of the drawing; the middle is the line the shape mirrors around. */
export const HEIGHT = 100;
const MIDDLE = HEIGHT / 2;

/**
 * The outline of `values` (0 to 255 for silence to full scale, one per
 * bucket) mirrored around the middle, one unit wide per bucket. Silence is
 * a thin line, so the track's length is always visible.
 */
export function outline(values: readonly number[]): string {
  if (values.length === 0) return "";
  const half = (v: number) => Math.max((v / 255) * MIDDLE, 0.5);
  const top = values.map((v, i) => `${i === 0 ? "M" : "L"} ${i + 0.5} ${MIDDLE - half(v)}`);
  const bottom = values.map((v, i) => `L ${i + 0.5} ${MIDDLE + half(v)}`).reverse();
  return [...top, ...bottom, "Z"].join(" ");
}

/** How far through the track the playhead is, 0 to 1. */
export function playedFraction(position: number, length: number): number {
  if (!(length > 0) || !Number.isFinite(position)) return 0;
  return Math.min(Math.max(position / length, 0), 1);
}

