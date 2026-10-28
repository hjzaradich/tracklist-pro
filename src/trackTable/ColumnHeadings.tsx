import { useEffect, useId, useRef, useState, type KeyboardEvent, type PointerEvent } from "react";
import { useTranslation } from "react-i18next";
import type { SortColumn, TableSort } from "../bindings";
import { clampWidth, WIDEST, type ColumnWidths } from "./columnChoice";
import { COLUMNS, type ColumnId } from "./columns";
import styles from "./TrackTable.module.css";

/** How far the pointer moves, in pixels, before a press on a heading is a drag and not a click. */
const DRAG_FROM = 4;

/** What an arrow key does to a column's width, in pixels. */
const KEY_STEP = 10;

/** A press on a heading that may turn into a drag to another place. */
interface Moving {
  id: ColumnId;
  pointerId: number;
  startX: number;
  x: number;
  dragging: boolean;
  /** The place it would go to now, in the columns without it. */
  to: number;
  frame: number | null;
}

/** A press on the line after a heading. */
interface Sizing {
  id: ColumnId;
  pointerId: number;
  startX: number;
  startWidth: number;
  frame: number | null;
}

/**
 * The heading row of the track table (1cB-21; ROADMAP 1.11): a click on a
 * heading sorts by it; dragging a heading to another place moves the
 * column; dragging the line after a heading (or, with the keyboard, the
 * Left and Right arrow keys on it) changes the column's width.
 *
 * Pointer events by hand, no library. The pointer moves are applied once per
 * animation frame, and a width being dragged is shown through `onPreview`
 * (the table sets it without drawing its rows again); only the end of a
 * drag goes to `onReorder` and `onResize`, which save it.
 */
export function ColumnHeadings({
  columns,
  widths,
  sort,
  onSort,
  onReorder,
  onResize,
  onPreview,
  actionsHeading,
}: {
  columns: readonly ColumnId[];
  widths: ColumnWidths;
  sort: TableSort | null;
  onSort: (column: SortColumn) => void;
  /** The column `id` was dropped at place `to`, counted in the columns without it. */
  onReorder?: (id: ColumnId, to: number) => void;
  /** The column was given this width (the other widths stay as `widths` has them). */
  onResize?: (id: ColumnId, width: number) => void;
  /** A width is being dragged: show it, or (`null`) show what is saved. */
  onPreview?: (width: { id: ColumnId; width: number } | null) => void;
  /** The (hidden) heading of the actions column, when the table has one. */
  actionsHeading?: string;
}) {
  const { t } = useTranslation("trackTable");
  const base = useId();
  const cells = useRef(new Map<ColumnId, HTMLDivElement>());
  const moving = useRef<Moving | null>(null);
  const sizing = useRef<Sizing | null>(null);
  // The click that ends a drag is not a sort.
  const ignoreClick = useRef(false);
  const [drag, setDrag] = useState<{ id: ColumnId; to: number } | null>(null);

  /** The click that follows the end of a drag, however it ended, is not a sort. */
  const ignoreNextClick = () => {
    ignoreClick.current = true;
    setTimeout(() => {
      ignoreClick.current = false;
    }, 0);
  };

  /** The heading's width on screen, else the one it has by choice or default. */
  const widthOf = (id: ColumnId): number => {
    const shown = cells.current.get(id)?.getBoundingClientRect().width ?? 0;
    return shown > 0 ? shown : (widths[id] ?? COLUMNS[id].width);
  };

  // Moving a heading ------------------------------------------------------

  /** Where the column `id` would go with the pointer at `x`. */
  const placeFor = (id: ColumnId, x: number): number => {
    let to = 0;
    for (const other of columns) {
      if (other === id) continue;
      const box = cells.current.get(other)?.getBoundingClientRect();
      if (box === undefined || x <= box.left + box.width / 2) break;
      to += 1;
    }
    return to;
  };

  const follow = (press: Moving) => {
    press.frame = null;
    const cell = cells.current.get(press.id);
    if (cell) cell.style.transform = `translateX(${press.x - press.startX}px)`;
    const to = placeFor(press.id, press.x);
    if (to !== press.to) {
      press.to = to;
      setDrag({ id: press.id, to });
    }
  };

  const endMove = (press: Moving) => {
    if (press.frame !== null) cancelAnimationFrame(press.frame);
    const cell = cells.current.get(press.id);
    if (cell) cell.style.transform = "";
    setDrag(null);
    moving.current = null;
  };

  const startMove = (event: PointerEvent<HTMLButtonElement>, id: ColumnId) => {
    if (event.button !== 0 || onReorder === undefined) return;
    ignoreClick.current = false;
    moving.current = {
      id,
      pointerId: event.pointerId,
      startX: event.clientX,
      x: event.clientX,
      dragging: false,
      to: columns.indexOf(id),
      frame: null,
    };
  };

  const moveMove = (event: PointerEvent<HTMLButtonElement>) => {
    const press = moving.current;
    if (press === null || event.pointerId !== press.pointerId) return;
    press.x = event.clientX;
    if (!press.dragging) {
      if (Math.abs(press.x - press.startX) < DRAG_FROM) return;
      press.dragging = true;
      event.currentTarget.setPointerCapture?.(event.pointerId);
    }
    press.frame ??= requestAnimationFrame(() => follow(press));
  };

  const finishMove = (event: PointerEvent<HTMLButtonElement>) => {
    const press = moving.current;
    if (press === null) {
      // The drag was given up: this release, and the click after it, are not a sort.
      if (ignoreClick.current) ignoreNextClick();
      return;
    }
    if (event.pointerId !== press.pointerId) return;
    const was = press.dragging;
    const to = placeFor(press.id, event.clientX);
    endMove(press);
    if (!was) return;
    ignoreNextClick();
    if (to !== columns.indexOf(press.id)) onReorder?.(press.id, to);
  };

  const cancelMove = (event: PointerEvent<HTMLButtonElement>) => {
    const press = moving.current;
    if (press !== null && event.pointerId === press.pointerId) endMove(press);
  };

  // Escape gives a drag up, wherever the focus is (a browser may not focus a
  // button that is pressed). The release that follows does nothing, and the
  // click it sends is not a sort.
  const giveUp = useRef<() => void>(() => {});
  useEffect(() => {
    giveUp.current = () => {
      const press = moving.current;
      if (press?.dragging) {
        endMove(press);
        // Held until the release, which may come much later.
        ignoreClick.current = true;
      }
      const size = sizing.current;
      if (size) {
        if (size.frame !== null) cancelAnimationFrame(size.frame);
        sizing.current = null;
        onPreview?.(null);
      }
    };
  });
  useEffect(() => {
    const onKey = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") giveUp.current();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, []);

  // Sizing a column -------------------------------------------------------

  const startSize = (event: PointerEvent<HTMLDivElement>, id: ColumnId) => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.currentTarget.setPointerCapture?.(event.pointerId);
    sizing.current = {
      id,
      pointerId: event.pointerId,
      startX: event.clientX,
      startWidth: widthOf(id),
      frame: null,
    };
  };

  const moveSize = (event: PointerEvent<HTMLDivElement>) => {
    const press = sizing.current;
    if (press === null || event.pointerId !== press.pointerId) return;
    const width = clampWidth(press.id, press.startWidth + event.clientX - press.startX);
    // Once per animation frame, with the latest width.
    if (press.frame !== null) cancelAnimationFrame(press.frame);
    press.frame = requestAnimationFrame(() => {
      press.frame = null;
      onPreview?.({ id: press.id, width });
    });
  };

  const finishSize = (event: PointerEvent<HTMLDivElement>) => {
    const press = sizing.current;
    if (press === null || event.pointerId !== press.pointerId) return;
    if (press.frame !== null) cancelAnimationFrame(press.frame);
    sizing.current = null;
    const width = clampWidth(press.id, press.startWidth + event.clientX - press.startX);
    if (width === Math.round(press.startWidth)) {
      onPreview?.(null);
      return;
    }
    // The width stays as shown: the saved one is the same.
    onPreview?.({ id: press.id, width });
    onResize?.(press.id, width);
  };

  const cancelSize = (event: PointerEvent<HTMLDivElement>) => {
    const press = sizing.current;
    if (press === null || event.pointerId !== press.pointerId) return;
    if (press.frame !== null) cancelAnimationFrame(press.frame);
    sizing.current = null;
    onPreview?.(null);
  };

  const keySize = (event: KeyboardEvent<HTMLDivElement>, id: ColumnId) => {
    const by = event.key === "ArrowLeft" ? -KEY_STEP : event.key === "ArrowRight" ? KEY_STEP : 0;
    if (by === 0 || event.ctrlKey || event.metaKey || event.altKey) return;
    event.preventDefault();
    const now = widthOf(id);
    const width = clampWidth(id, now + by);
    if (width !== Math.round(now)) onResize?.(id, width);
  };

  // The line where a dropped heading would go: before this one, or after the last.
  const others = drag === null ? [] : columns.filter((id) => id !== drag.id);
  const before = drag === null ? undefined : others[drag.to];
  const after = drag !== null && before === undefined ? others.at(-1) : undefined;

  return (
    <div role="row" aria-rowindex={1} className={styles.headRow}>
      {columns.map((id) => {
        const def = COLUMNS[id];
        const sorted = sort?.column === def.sort;
        const name = t(`columns.${id}`);
        const classes = [
          styles.headCell,
          def.numeric ? styles.numeric : "",
          drag?.id === id ? styles.dragging : "",
          before === id ? styles.dropBefore : "",
          after === id ? styles.dropAfter : "",
        ];
        return (
          <div
            key={id}
            ref={(element) => {
              if (element) cells.current.set(id, element);
              else cells.current.delete(id);
            }}
            role="columnheader"
            // Named by its heading alone, not by the resize line beside it.
            aria-labelledby={`${base}-${id}`}
            aria-sort={sorted ? (sort.descending ? "descending" : "ascending") : undefined}
            className={classes.filter(Boolean).join(" ")}
          >
            <button
              id={`${base}-${id}`}
              type="button"
              className={styles.sortButton}
              title={t("sortBy", { column: name })}
              onClick={() => {
                if (!ignoreClick.current) onSort(def.sort);
              }}
              onPointerDown={(event) => startMove(event, id)}
              onPointerMove={moveMove}
              onPointerUp={finishMove}
              onPointerCancel={cancelMove}
            >
              <span className={styles.headText}>{name}</span>
              {sorted && <SortCaret descending={sort.descending} />}
            </button>
            {onResize && (
              <div
                role="separator"
                aria-orientation="vertical"
                aria-label={t("resize", { column: name })}
                title={t("resize", { column: name })}
                aria-valuenow={Math.round(widths[id] ?? def.width)}
                aria-valuemin={def.min}
                aria-valuemax={WIDEST}
                tabIndex={0}
                className={styles.resizer}
                onPointerDown={(event) => startSize(event, id)}
                onPointerMove={moveSize}
                onPointerUp={finishSize}
                onPointerCancel={cancelSize}
                onKeyDown={(event) => keySize(event, id)}
              />
            )}
          </div>
        );
      })}
      {actionsHeading !== undefined && (
        <div role="columnheader" className={styles.headCell}>
          <span className={styles.hidden}>{actionsHeading}</span>
        </div>
      )}
    </div>
  );
}

/** The arrow beside the sorted heading. Decorative: `aria-sort` says it. */
function SortCaret({ descending }: { descending: boolean }) {
  return (
    <svg
      className={styles.caret}
      width="10"
      height="10"
      viewBox="0 0 10 10"
      aria-hidden="true"
      focusable="false"
    >
      <path d={descending ? "M1 3l4 4 4-4z" : "M1 7l4-4 4 4z"} fill="currentColor" />
    </svg>
  );
}
