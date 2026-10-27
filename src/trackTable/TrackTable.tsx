import {
  type CSSProperties,
  type KeyboardEvent,
  type ReactNode,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import type { SortColumn, TableRow, TableSort } from "../bindings";
import { useKeyNotation } from "../keys/useKeyNotation";
import { COLUMNS, type CellContext, type ColumnDef, type ColumnId } from "./columns";
import styles from "./TrackTable.module.css";

/** Rows drawn above and below the ones on screen, so a quick scroll never shows a gap. */
const OVERSCAN = 6;

/** The viewport's height in rows when the browser can't say (it has no layout). */
const FALLBACK_ROWS = 20;

export interface TrackTableProps {
  /** The table's accessible name. */
  label: string;
  /** The columns, in order. The list is data: the column editor (1cA-5) hands in another. */
  columns: readonly ColumnId[];
  /** How many rows there are in all. */
  total: number;
  /** The row at an index, once its window has arrived. */
  getRow: (index: number) => TableRow | undefined;
  /** Called with the first and last index on screen (and the rows around them) whenever they change. */
  onRangeChange: (first: number, last: number) => void;
  /** The order the rows are in, as the headings show it. */
  sort: TableSort | null;
  /** A heading was clicked. */
  onSort: (column: SortColumn) => void;
  /** The last cell of each row: the screen's buttons. */
  actions?: (row: TableRow) => ReactNode;
  /** The actions column's width in pixels. */
  actionsWidth?: number;
  /**
   * Changes when the rows are another order or another list (a sort, a
   * filter, a search or another crate): the table goes back to the top. A
   * selected track stays selected, and the table scrolls to it, when
   * `positionOf` finds it in the new rows; otherwise nothing is selected.
   */
  resetKey: string;
  /**
   * Where the track with this id is in the rows now (their index), or
   * `null` if it isn't one of them. Without it a new `resetKey` always
   * clears the selection.
   */
  positionOf?: (recordingId: number) => Promise<number | null>;
  /** The rows of the current `resetKey` have arrived, so an index can be scrolled to. Default: yes. */
  settled?: boolean;
  /** The selected row changed (or went); `null` for none. */
  onSelectionChange?: (row: TableRow | null) => void;
}

/** The row height the theme sets, in pixels. */
function readRowHeight(): number {
  const value = Number.parseFloat(
    getComputedStyle(document.documentElement).getPropertyValue("--row-height"),
  );
  return Number.isFinite(value) && value > 0 ? value : 28;
}

/**
 * The track table (1cA-3; ROADMAP 1.11 "The track table"): one table for
 * the Library, All music and a crate. It draws only the rows on screen plus
 * a few beside them, so a library of 100,000 tracks scrolls as smoothly as
 * one of 100, and it asks for rows by index (`getRow`, `onRangeChange`),
 * never for the whole list.
 *
 * - **Sorting** is the backend's: a click on a heading calls `onSort`, and
 *   the screen asks for the rows again in the new order.
 * - **One row is selected** (tinted, with the accent bar), by click or by
 *   keyboard. The table is one tab stop: Up and Down, Page Up and Page
 *   Down, Home and End move the selection; buttons inside a row are tab
 *   stops of their own.
 * - **Playing** is shown by the play button's bars alone; the row isn't
 *   tinted for it.
 */
export function TrackTable({
  label,
  columns,
  total,
  getRow,
  onRangeChange,
  sort,
  onSort,
  actions,
  actionsWidth = 200,
  resetKey,
  positionOf,
  settled = true,
  onSelectionChange,
}: TrackTableProps) {
  const { t, i18n } = useTranslation("trackTable");
  const keyNotation = useKeyNotation();
  const id = useId();
  const viewport = useRef<HTMLDivElement>(null);
  // The theme's row height, read once: it is the same for the life of the app.
  const [rowHeight] = useState(readRowHeight);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewportHeight, setViewportHeight] = useState(0);
  const [active, setActive] = useState<number | null>(null);
  // The selected row's track, so another order can find it again. Known
  // once its row has arrived.
  const [activeId, setActiveId] = useState<number | null>(null);
  // The track being looked for in the rows of `resetKey`, and where the
  // backend said it is (`index` is `null` for nowhere).
  const [looking, setLooking] = useState<{ id: number; key: string } | null>(null);
  const [found, setFound] = useState<{ key: string; index: number | null } | null>(null);
  // Where to scroll to once a track is found again.
  const [reveal, setReveal] = useState<{ top: number } | null>(null);

  // Another order or list: back to the top, and the selected track, if
  // there is one, is looked for in the new rows. (Adjusting state while
  // rendering, as React's guide for state that follows a prop.)
  const [seen, setSeen] = useState(resetKey);
  if (seen !== resetKey) {
    setSeen(resetKey);
    const carried = activeId ?? looking?.id ?? null;
    setActive(null);
    setActiveId(null);
    setFound(null);
    setLooking(positionOf && carried !== null ? { id: carried, key: resetKey } : null);
    setScrollTop(0);
  }
  useLayoutEffect(() => {
    if (viewport.current) viewport.current.scrollTop = 0;
  }, [resetKey]);

  // Ask where the track is now. The latest `positionOf` is the one for the
  // latest rows.
  const positionRef = useRef(positionOf);
  useEffect(() => {
    positionRef.current = positionOf;
  });
  useEffect(() => {
    if (looking === null) return;
    let current = true;
    const answer = (index: number | null) => {
      if (current) setFound({ key: looking.key, index });
    };
    positionRef.current?.(looking.id).then(answer, () => answer(null));
    return () => {
      current = false;
    };
  }, [looking]);

  // The viewport's height, and when it changes.
  useLayoutEffect(() => {
    const element = viewport.current;
    if (!element) return;
    const measure = () => setViewportHeight(element.clientHeight);
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  const defs = columns.map((column) => COLUMNS[column]);
  const context: CellContext = { t, keyNotation, language: i18n.language };

  // The header is a row in the scroll area, so the rows' room is what is left.
  const room = Math.max(
    rowHeight,
    (viewportHeight > 0 ? viewportHeight : FALLBACK_ROWS * rowHeight) - rowHeight,
  );
  const rowsOnScreen = Math.ceil(room / rowHeight);
  const last = total - 1;
  const first = Math.min(Math.max(0, Math.floor(scrollTop / rowHeight)), Math.max(0, last));
  const lastShown = Math.min(last, first + rowsOnScreen);
  const from = Math.max(0, first - OVERSCAN);
  const to = Math.min(last, lastShown + OVERSCAN);

  useEffect(() => {
    if (total > 0) onRangeChange(from, to);
  }, [from, to, total, onRangeChange]);

  // The selected row, to tell the screen when it changes.
  const activeRow = active === null ? undefined : getRow(active);

  // The selected row's track is known once its row has arrived. (Not in the
  // render that changes `resetKey`: its rows are the new list's.)
  if (seen === resetKey && active !== null && activeId === null && activeRow) {
    setActiveId(activeRow.recordingId);
  }

  // The track looked for has been found, and the rows it is in have arrived:
  // select it and put it in the middle of the screen. Not found: nothing is
  // selected, and the table stays at the top.
  if (
    looking !== null &&
    found !== null &&
    seen === resetKey &&
    found.key === looking.key &&
    found.key === resetKey &&
    settled
  ) {
    setLooking(null);
    setFound(null);
    if (found.index !== null && found.index < total) {
      const top = Math.max(0, found.index * rowHeight - Math.floor((room - rowHeight) / 2));
      setActive(found.index);
      setActiveId(looking.id);
      setScrollTop(top);
      setReveal({ top });
    }
  }
  useLayoutEffect(() => {
    if (reveal && viewport.current) viewport.current.scrollTop = reveal.top;
  }, [reveal]);
  useEffect(() => {
    onSelectionChange?.(activeRow ?? null);
  }, [activeRow, onSelectionChange]);

  const select = useCallback(
    (index: number) => {
      const clamped = Math.min(Math.max(0, index), Math.max(0, total - 1));
      setActive(clamped);
      setActiveId(getRow(clamped)?.recordingId ?? null);
      // Choosing a row is the answer to looking for one.
      setLooking(null);
      setFound(null);
      // Keep it in view.
      const element = viewport.current;
      if (!element) return;
      const top = clamped * rowHeight;
      let next = element.scrollTop;
      if (top < next) next = top;
      else if (top + rowHeight > next + room) next = top + rowHeight - room;
      if (next !== element.scrollTop) {
        element.scrollTop = next;
        setScrollTop(next);
      }
    },
    [total, rowHeight, room, getRow],
  );

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    // Keys pressed on a button inside a row are the button's.
    if (event.target !== event.currentTarget || total === 0) return;
    if (event.ctrlKey || event.metaKey || event.altKey) return;
    const page = Math.max(1, rowsOnScreen - 1);
    const start = active ?? (event.key === "ArrowUp" || event.key === "PageUp" ? first + 1 : first - 1);
    let target: number;
    switch (event.key) {
      case "ArrowDown":
        target = start + 1;
        break;
      case "ArrowUp":
        target = start - 1;
        break;
      case "PageDown":
        target = (active ?? first) + page;
        break;
      case "PageUp":
        target = (active ?? first) - page;
        break;
      case "Home":
        target = 0;
        break;
      case "End":
        target = last;
        break;
      default:
        return;
    }
    event.preventDefault();
    select(target);
  };

  const templateColumns = [
    ...defs.map((def) => (def.grow ? `minmax(${def.width}px, ${def.grow}fr)` : `${def.width}px`)),
    ...(actions ? [`${actionsWidth}px`] : []),
  ].join(" ");
  const minWidth = defs.reduce((sum, def) => sum + def.width, actions ? actionsWidth : 0);
  const cssVars = {
    "--tt-columns": templateColumns,
    "--tt-min-width": `${minWidth}px`,
    "--tt-row": `${rowHeight}px`,
  } as CSSProperties;

  const activeIndexRendered = active !== null && active >= from && active <= to;
  const rows: ReactNode[] = [];
  for (let index = from; index <= to; index++) {
    const row = getRow(index);
    rows.push(
      <div
        key={index}
        id={`${id}-row-${index}`}
        role="row"
        aria-rowindex={index + 2}
        aria-selected={active === index}
        aria-busy={row === undefined || undefined}
        className={[
          styles.row,
          index % 2 === 1 ? styles.stripe : "",
          active === index ? styles.selected : "",
        ]
          .filter(Boolean)
          .join(" ")}
        style={{ transform: `translateY(${index * rowHeight}px)` }}
        onClick={() => select(index)}
      >
        {defs.map((def) => (
          <Cell key={def.id} def={def} row={row} context={context} />
        ))}
        {actions && (
          <div role="gridcell" className={`${styles.cell} ${styles.actionsCell}`}>
            {row !== undefined && actions(row)}
          </div>
        )}
      </div>,
    );
  }

  return (
    <div className={styles.table} style={cssVars}>
      <div
        ref={viewport}
        className={styles.viewport}
        role="grid"
        aria-label={label}
        aria-rowcount={total + 1}
        aria-colcount={defs.length + (actions ? 1 : 0)}
        aria-activedescendant={activeIndexRendered ? `${id}-row-${active}` : undefined}
        tabIndex={0}
        onKeyDown={onKeyDown}
        onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
      >
        <div role="rowgroup" className={styles.header}>
          <div role="row" aria-rowindex={1} className={styles.headRow}>
            {defs.map((def) => {
              const sorted = sort?.column === def.sort;
              const name = t(`columns.${def.id}`);
              return (
                <div
                  key={def.id}
                  role="columnheader"
                  aria-sort={sorted ? (sort.descending ? "descending" : "ascending") : undefined}
                  className={`${styles.headCell} ${def.numeric ? styles.numeric : ""}`}
                >
                  <button
                    type="button"
                    className={styles.sortButton}
                    title={t("sortBy", { column: name })}
                    onClick={() => onSort(def.sort)}
                  >
                    <span className={styles.headText}>{name}</span>
                    {sorted && <SortCaret descending={sort.descending} />}
                  </button>
                </div>
              );
            })}
            {actions && (
              <div role="columnheader" className={styles.headCell}>
                <span className={styles.hidden}>{t("columns.actions")}</span>
              </div>
            )}
          </div>
        </div>
        <div role="rowgroup" className={styles.rows} style={{ height: total * rowHeight }}>
          {rows}
        </div>
      </div>
    </div>
  );
}

/** One cell of a row; a row still on its way has empty cells. */
function Cell({
  def,
  row,
  context,
}: {
  def: ColumnDef;
  row: TableRow | undefined;
  context: CellContext;
}) {
  const classes = [styles.cell, def.numeric ? styles.numeric : "", def.cell ? styles.rich : ""];
  if (row === undefined) {
    return <div role="gridcell" className={classes.join(" ")} />;
  }
  const text = def.text(row, context);
  return (
    <div
      role="gridcell"
      className={classes.join(" ")}
      // A cell cut off with "…" carries the text it shows.
      title={def.cell ? undefined : text || undefined}
    >
      {def.cell ? def.cell(row, context) : text}
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
