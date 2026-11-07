import type { TargetPoint } from "../bindings";

// The numbers behind the timeline's drawing (3C-P2b; ROADMAP 3.5). Pure, so
// the picture can be tested without drawing it.

export const WIDTH = 600;
export const HEIGHT = 200;
/** Room kept round the plot: left for the energy scale, top for the transition markers. */
export const PAD = { left: 30, right: 14, top: 26, bottom: 18 };
export const MIN_ENERGY = 1;
export const MAX_ENERGY = 10;
/** Most points the backend keeps. */
export const MOST_POINTS = 32;

export interface Point {
  at: number;
  energy: number;
}

const PLOT_WIDTH = WIDTH - PAD.left - PAD.right;
const PLOT_HEIGHT = HEIGHT - PAD.top - PAD.bottom;

const clamp = (n: number, low: number, high: number) => Math.min(high, Math.max(low, n));

/** How far along the set (0 to 1) an entry sits: the backend's `i / (n - 1)`. */
export function entryAt(index: number, count: number): number {
  return count <= 1 ? 0.5 : index / (count - 1);
}

export const xOf = (at: number) => PAD.left + clamp(at, 0, 1) * PLOT_WIDTH;
export const yOf = (energy: number) =>
  PAD.top + (1 - (clamp(energy, MIN_ENERGY, MAX_ENERGY) - MIN_ENERGY) / (MAX_ENERGY - MIN_ENERGY)) * PLOT_HEIGHT;

/** Where along the set (0 to 1, to 2 places) a drawing x is. */
export const atOf = (x: number) => Math.round(clamp((x - PAD.left) / PLOT_WIDTH, 0, 1) * 100) / 100;
/** The whole energy (1 to 10) a drawing y is. */
export const energyOf = (y: number) =>
  Math.round(clamp(MAX_ENERGY - ((y - PAD.top) / PLOT_HEIGHT) * (MAX_ENERGY - MIN_ENERGY), MIN_ENERGY, MAX_ENERGY));

/** The shape's points that have both numbers, in order along the set. */
export function shapeOf(target: readonly TargetPoint[]): Point[] {
  return target
    .flatMap((p) => (p.at === null || p.energy === null ? [] : [{ at: p.at, energy: p.energy }]))
    .sort((a, b) => a.at - b.at);
}

/** Runs of consecutive known values: an unknown one ends a run, so it is a gap in the line. */
export function runs(values: readonly (number | null)[], xs: readonly number[], y: (v: number) => number): [number, number][][] {
  const out: [number, number][][] = [];
  let run: [number, number][] = [];
  values.forEach((value, i) => {
    if (value === null) {
      if (run.length > 0) out.push(run);
      run = [];
    } else {
      run.push([xs[i] ?? 0, y(value)]);
    }
  });
  if (run.length > 0) out.push(run);
  return out;
}

export const pathOf = (points: readonly (readonly [number, number])[]) =>
  points.map(([x, y], i) => `${i === 0 ? "M" : "L"}${x.toFixed(1)} ${y.toFixed(1)}`).join(" ");

/** The target as drawn: flat before the first point and after the last. */
export function targetPath(shape: readonly Point[]): string {
  const first = shape[0];
  const last = shape.at(-1);
  if (first === undefined || last === undefined) return "";
  const line: [number, number][] = [[xOf(0), yOf(first.energy)]];
  for (const p of shape) line.push([xOf(p.at), yOf(p.energy)]);
  line.push([xOf(1), yOf(last.energy)]);
  return pathOf(line);
}

/** The shape with point `index` moved (kept in range and in order). */
export function movePoint(shape: readonly Point[], index: number, to: Point): Point[] {
  const moved = { at: clamp(Math.round(to.at * 100) / 100, 0, 1), energy: clamp(Math.round(to.energy), MIN_ENERGY, MAX_ENERGY) };
  return shape.map((p, i) => (i === index ? moved : p)).sort((a, b) => a.at - b.at);
}

export const removePoint = (shape: readonly Point[], index: number): Point[] => shape.filter((_, i) => i !== index);

/** The shape with a point added; one already at that place is replaced. */
export function addPoint(shape: readonly Point[], point: Point): Point[] {
  const added = { at: point.at, energy: clamp(Math.round(point.energy), MIN_ENERGY, MAX_ENERGY) };
  return [...shape.filter((p) => p.at !== added.at), added].sort((a, b) => a.at - b.at);
}

/** Where "Add point" puts one: the middle of the widest stretch without a point, at the line's height there. */
export function nextPoint(shape: readonly Point[]): Point {
  const edges = [0, ...shape.map((p) => p.at), 1];
  let best = 0;
  for (let i = 1; i < edges.length; i++) {
    if ((edges[i] ?? 0) - (edges[i - 1] ?? 0) > (edges[best + 1] ?? 0) - (edges[best] ?? 0)) best = i - 1;
  }
  const at = Math.round((((edges[best] ?? 0) + (edges[best + 1] ?? 1)) / 2) * 100) / 100;
  const before = shape.filter((p) => p.at < at).at(-1);
  const after = shape.find((p) => p.at > at);
  const energy = before?.energy ?? after?.energy ?? 5;
  return { at, energy: before !== undefined && after !== undefined ? (before.energy + after.energy) / 2 : energy };
}
