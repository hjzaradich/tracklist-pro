/** How far from the top or bottom edge of the list a drag starts scrolling it, in pixels. */
export const EDGE_ZONE = 48;
/** The most the list scrolls in one animation frame, at the very edge, in pixels. */
export const MAX_SCROLL_STEP = 22;

/**
 * Which way and how fast a list scrolls while rows are dragged over it (the
 * pointer at `y`, the list between `top` and `bottom`): negative scrolls up,
 * positive down, 0 not at all. Within `zone` pixels of an edge the list
 * scrolls, faster the nearer the edge (at least one pixel, `maxStep` at the
 * edge and beyond it). Outside the list, or in the middle, it stays still.
 * `inset` leaves room at the top for a header that stays in view.
 */
export function edgeScrollStep(
  y: number,
  top: number,
  bottom: number,
  options: { zone?: number; maxStep?: number; inset?: number } = {},
): number {
  const { zone = EDGE_ZONE, maxStep = MAX_SCROLL_STEP, inset = 0 } = options;
  const start = top + inset;
  if (y < top || y > bottom || bottom <= start) return 0;
  // Never more than half the list, so a short list has a still middle.
  const reach = Math.min(zone, (bottom - start) / 2);
  const fromTop = y - start;
  const fromBottom = bottom - y;
  if (fromTop < reach) return -stepFor(reach - Math.max(fromTop, 0), reach, maxStep);
  if (fromBottom < reach) return stepFor(reach - fromBottom, reach, maxStep);
  return 0;
}

function stepFor(depth: number, reach: number, maxStep: number): number {
  return Math.max(1, Math.round((Math.min(depth, reach) / reach) * maxStep));
}
