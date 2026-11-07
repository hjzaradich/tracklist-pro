import { type KeyboardEvent, type PointerEvent, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { Timeline } from "../bindings";
import styles from "./Timeline.module.css";
import {
  addPoint,
  atOf,
  entryAt,
  energyOf,
  HEIGHT,
  MAX_ENERGY,
  MIN_ENERGY,
  MOST_POINTS,
  movePoint,
  PAD,
  type Point,
  pathOf,
  removePoint,
  runs,
  shapeOf,
  targetPath,
  WIDTH,
  xOf,
  yOf,
} from "./timelineShape";

const SCALE = [MIN_ENERGY, 5, MAX_ENERGY];
/** Closest two target points may sit (a hundredth of the set). */
const APART = 0.01;
/** How far a key press moves a point along the set. */
const STEP = 0.05;

/**
 * The energy curve of a playlist against its target shape (3C-P2b; ROADMAP
 * 3.5), drawn by hand as the waveform is. An entry with no energy leaves a
 * gap in the curve (a dashed tick), never a zero. A mark above each
 * transition says what its flags found: a diamond for a clash, a hollow
 * circle when something is unknown. The target points can be placed (press
 * the picture), dragged, or moved with the arrow keys and taken out with
 * Delete; each change is committed with `onCommit` and is one Undo step.
 */
export function TimelineChart({
  timeline,
  onCommit,
}: {
  timeline: Timeline;
  /** Resolves when the new shape is saved; the picture shows the draft until then. */
  onCommit: (shape: Point[]) => Promise<unknown>;
}) {
  const { t } = useTranslation("playlists");
  const svg = useRef<SVGSVGElement>(null);
  const [draft, setDraft] = useState<Point[] | null>(null);
  const dragging = useRef<number | null>(null);
  const saved = shapeOf(timeline.target);
  const shape = draft ?? saved;
  const count = timeline.entries.length;
  const xs = timeline.entries.map((_, i) => xOf(entryAt(i, count)));
  const lines = runs(
    timeline.entries.map((e) => e.energy),
    xs,
    yOf,
  );

  const commit = (next: Point[]) => {
    setDraft(next);
    void onCommit(next).finally(() => setDraft(null));
  };
  const drawing = (event: { clientX: number; clientY: number }) => {
    const box = svg.current?.getBoundingClientRect();
    if (!box || box.width === 0 || box.height === 0) return null;
    return { x: ((event.clientX - box.left) * WIDTH) / box.width, y: ((event.clientY - box.top) * HEIGHT) / box.height };
  };
  /** `p` kept between its neighbours, so dragging never reorders the points. */
  const between = (index: number, p: Point): Point => {
    const low = (shape[index - 1]?.at ?? -APART) + APART;
    const high = (shape[index + 1]?.at ?? 1 + APART) - APART;
    return { at: Math.min(Math.max(p.at, low), high), energy: p.energy };
  };

  const onBackground = (event: PointerEvent<SVGRectElement>) => {
    const at = drawing(event);
    if (at === null || shape.length >= MOST_POINTS) return;
    commit(addPoint(shape, { at: atOf(at.x), energy: energyOf(at.y) }));
  };
  const onMove = (event: PointerEvent<SVGSVGElement>) => {
    const index = dragging.current;
    const at = drawing(event);
    if (index === null || at === null) return;
    setDraft(movePoint(shape, index, between(index, { at: atOf(at.x), energy: energyOf(at.y) })));
  };
  const onUp = () => {
    const index = dragging.current;
    dragging.current = null;
    if (index === null || draft === null) return;
    const same = saved.length === draft.length && saved.every((p, i) => p.at === draft[i]?.at && p.energy === draft[i]?.energy);
    if (same) setDraft(null);
    else commit(draft);
  };
  const onKey = (event: KeyboardEvent<SVGGElement>, index: number) => {
    const p = shape[index];
    if (p === undefined) return;
    const move = (to: Point) => {
      event.preventDefault();
      commit(movePoint(shape, index, between(index, to)));
    };
    switch (event.key) {
      case "ArrowUp":
        return move({ ...p, energy: Math.min(MAX_ENERGY, p.energy + 1) });
      case "ArrowDown":
        return move({ ...p, energy: Math.max(MIN_ENERGY, p.energy - 1) });
      case "ArrowLeft":
        return move({ ...p, at: p.at - STEP });
      case "ArrowRight":
        return move({ ...p, at: p.at + STEP });
      case "Delete":
      case "Backspace":
        event.preventDefault();
        return commit(removePoint(shape, index));
    }
  };

  return (
    <svg
      ref={svg}
      className={styles.chart}
      viewBox={`0 0 ${WIDTH} ${HEIGHT}`}
      role="group"
      aria-label={t("timeline.chart.label")}
      onPointerMove={onMove}
      onPointerUp={onUp}
      onPointerCancel={onUp}
    >
      {SCALE.map((energy) => (
        <g key={energy}>
          <line className={styles.grid} x1={PAD.left} x2={WIDTH - PAD.right} y1={yOf(energy)} y2={yOf(energy)} />
          <text className={styles.scale} x={PAD.left - 6} y={yOf(energy)} textAnchor="end" dominantBaseline="middle">
            {energy}
          </text>
        </g>
      ))}
      <rect
        className={styles.plot}
        data-plot
        x={PAD.left}
        y={PAD.top}
        width={WIDTH - PAD.left - PAD.right}
        height={HEIGHT - PAD.top - PAD.bottom}
        onPointerDown={onBackground}
      />
      {shape.length > 0 && <path className={styles.target} data-target d={targetPath(shape)} />}
      {timeline.entries.map(
        (entry, i) =>
          entry.energy === null && (
            <line
              key={entry.entryId}
              className={styles.unknownTick}
              data-gap={entry.entryId}
              x1={xs[i]}
              x2={xs[i]}
              y1={PAD.top}
              y2={HEIGHT - PAD.bottom}
            />
          ),
      )}
      {lines.map((line, i) => (
        <path key={i} className={styles.curve} data-curve d={pathOf(line)} />
      ))}
      {timeline.entries.map(
        (entry, i) =>
          entry.energy !== null && (
            <circle key={entry.entryId} className={styles.dot} data-entry={entry.entryId} cx={xs[i]} cy={yOf(entry.energy)} r={3.5} />
          ),
      )}
      {timeline.transitions.map((step, i) => {
        const flows = [step.key, step.bpm, step.energy];
        const flag = flows.includes("clash") ? "clash" : flows.includes("unknown") ? "unknown" : null;
        if (flag === null) return null;
        const x = ((xs[i] ?? 0) + (xs[i + 1] ?? 0)) / 2;
        const y = PAD.top - 12;
        return (
          <g key={step.fromEntry} data-flag={flag} data-from={step.fromEntry}>
            <title>{t(flag === "clash" ? "timeline.chart.clash" : "timeline.chart.unknown")}</title>
            {flag === "clash" ? (
              <path className={styles.markClash} d={`M${x} ${y - 6} L${x + 6} ${y} L${x} ${y + 6} L${x - 6} ${y} Z`} />
            ) : (
              <circle className={styles.markUnknown} cx={x} cy={y} r={5} />
            )}
          </g>
        );
      })}
      {shape.map((p, i) => (
        <g
          key={i}
          className={styles.handle}
          role="slider"
          tabIndex={0}
          aria-label={t("timeline.target.point", { at: Math.round(p.at * 100) })}
          aria-valuemin={MIN_ENERGY}
          aria-valuemax={MAX_ENERGY}
          aria-valuenow={p.energy}
          aria-orientation="vertical"
          data-point={i}
          onKeyDown={(event) => onKey(event, i)}
          onPointerDown={(event) => {
            event.stopPropagation();
            dragging.current = i;
            event.currentTarget.setPointerCapture?.(event.pointerId);
          }}
        >
          <circle className={styles.hit} cx={xOf(p.at)} cy={yOf(p.energy)} r={9} />
          <circle className={styles.pointMark} cx={xOf(p.at)} cy={yOf(p.energy)} r={5} />
        </g>
      ))}
    </svg>
  );
}
