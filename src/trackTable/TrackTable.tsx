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
import { ColumnHeadings } from "./ColumnHeadings";
import { moveColumnTo, tableLayout, type ColumnWidths } from "./columnChoice";
import { COLUMNS, type CellContext, type ColumnDef, type ColumnId } from "./columns";
import styles from "./TrackTable.module.css";
import { useTrackKeys } from "./useTrackKeys";

/** Rows drawn above and below the ones on screen, so a quick scroll never shows a gap. */
const OVERSCAN = 6;

/** The viewport's height in rows when the browser can't say (it has no layout). */
const FALLBACK_ROWS = 20;

export interface TrackTableProps {
  /** The table's accessible name. */
  label: string;
  /** The columns, in order. The list is data: the column editor (1cA-5) hands in another. */
  columns: readonly ColumnId[];
  /** The widths the user dragged columns to (1cB-21); a column without one has its own. */
  widths?: ColumnWidths;
  /**
   * A heading was dragged to another place: the columns in their new order.
   * Without it the headings can't be dragged.
   */
  onReorder?: (columns: readonly ColumnId[]) => void;
  /**
   * The line after a heading was dragged (or its arrow keys pressed): all the
   * widths, with the new one. Without it the columns can't be resized.
   */
  onResize?: (widths: ColumnWidths) => void;
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
  /** The focused row changed (or went); `null` for none. */
  onSelectionChange?: (row: TableRow | null) => void;
  /**
   * How many tracks are selected, or which row is focused, changed (1cB-8);
   * `focus` is the focused row's index.
   */
  onSelectedChange?: (selection: { count: number; focus: number | null }) => void;
  /**
   * The ids (`TableRow.recordingId`) of the rows from `first` to `last` of the
   * current rows, for a range or all of them that the windows held don't
   * cover. Without it only the rows held can be selected.
   */
  idsInRange?: (first: number, last: number) => Promise<readonly number[]>;
  /**
   * The energy keys (1 to 0, 1cB-8) gave these tracks this energy. Without it
   * the keys are unavailable.
   */
  onSetEnergy?: (recordingIds: readonly number[], energy: number) => void;
}

const NO_WIDTHS: ColumnWidths = {};

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
 * - **Selecting** (1cB-8): a click selects one row (tinted, with the accent
 *   bar), Shift+click the rows from the last click to this one, Ctrl+click
 *   adds or takes away one, Ctrl+A all of them and Escape none. Only
 *   selected rows are tinted; the focused row of a bigger selection has a
 *   ring. The table is one tab stop: Up and Down, Page Up and Page Down,
 *   Home and End move the focus and select that row (with Shift, the rows
 *   between too); buttons inside a row are tab stops of their own. The
 *   hotkeys J, K and 1 to 0 work from anywhere on the screen
 *   ({@link useTrackKeys}).
 * - **Playing** is shown by the play button's bars alone; the row isn't
 *   tinted for it.
 */
export function TrackTable({
  label,
  columns,
  widths = NO_WIDTHS,
  onReorder,
  onResize,
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
  onSelectedChange,
  idsInRange,
  onSetEnergy,
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
  // Several tracks, or none, selected: their ids. `null` is the usual case,
  // the focused row alone. Where a Shift range starts: an index.
  const [picked, setPicked] = useState<ReadonlySet<number> | null>(null);
  const [anchor, setAnchor] = useState<number | null>(null);
  // Goes up with every change of the selection, and `listNow` is the list
  // the table is showing, so the ids of a range that arrive late are dropped
  // if the selection moved on or the list changed.
  const changes = useRef(0);
  const listNow = useRef(resetKey);
  useEffect(() => {
    listNow.current = resetKey;
  });
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
    setPicked(null);
    setAnchor(null);
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

  // Moves the focus to a row and keeps it in view; the selection is the caller's.
  const focus = useCallback(
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

  // Selects only the row at `index` (a click, or a move without Shift).
  const select = (index: number) => {
    changes.current += 1;
    setPicked(null);
    setAnchor(Math.min(Math.max(0, index), Math.max(0, total - 1)));
    focus(index);
  };

  // The ids of rows `first` to `last`: at once when the windows hold them,
  // else from the backend.
  const rangeIds = (first: number, last: number): number[] | Promise<readonly number[]> => {
    const held: number[] = [];
    let missing = false;
    for (let i = first; i <= last; i++) {
      const row = getRow(i);
      if (row === undefined) missing = true;
      else held.push(row.recordingId);
    }
    return missing && idsInRange ? idsInRange(first, last) : held;
  };
  const chooseRange = (first: number, last: number) => {
    const token = ++changes.current;
    const list = resetKey;
    const ids = rangeIds(first, last);
    if (Array.isArray(ids)) setPicked(new Set(ids));
    else
      void (ids as Promise<readonly number[]>).then(
        (found) => {
          if (token === changes.current && list === listNow.current) setPicked(new Set(found));
        },
        () => {},
      );
  };
  // Shift: the rows from the anchor to `index`, which gets the focus.
  const extendTo = (index: number) => {
    const to = Math.min(Math.max(0, index), Math.max(0, total - 1));
    const from = anchor ?? active ?? to;
    if (anchor === null) setAnchor(from);
    focus(to);
    chooseRange(Math.min(from, to), Math.max(from, to));
  };
  // Ctrl: one row in or out of the selection.
  const toggle = (index: number) => {
    const row = getRow(index);
    if (row === undefined) return;
    const next = new Set(picked ?? (activeId === null ? [] : [activeId]));
    if (!next.delete(row.recordingId)) next.add(row.recordingId);
    changes.current += 1;
    setPicked(next);
    setAnchor(index);
    focus(index);
  };
  const selectAll = () => {
    if (total > 0) chooseRange(0, total - 1);
  };
  const clear = () => {
    changes.current += 1;
    setPicked(new Set());
  };
  // J and K: a row from the one focused (from the top of the screen when none is).
  const move = (step: 1 | -1, extend: boolean) => {
    if (total === 0) return;
    const from = active ?? (step === 1 ? first - 1 : first + 1);
    if (extend) extendTo(from + step);
    else select(from + step);
  };

  const selectedCount = picked !== null ? picked.size : active !== null ? 1 : 0;
  useEffect(() => {
    onSelectedChange?.({ count: selectedCount, focus: active });
  }, [selectedCount, active, onSelectedChange]);
  const setEnergy = onSetEnergy
    ? (energy: number) => {
        const ids = picked !== null ? [...picked] : activeId === null ? [] : [activeId];
        if (ids.length > 0) onSetEnergy(ids, energy);
      }
    : undefined;
  useTrackKeys({ move, selectAll, clear, selected: selectedCount, setEnergy });

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
    if (event.shiftKey) extendTo(target);
    else select(target);
  };

  const grid = tableLayout(columns, widths, actions ? actionsWidth : undefined);
  const cssVars = {
    "--tt-columns": grid.template,
    "--tt-min-width": `${grid.minWidth}px`,
    "--tt-row": `${rowHeight}px`,
  } as CSSProperties;

  // A width being dragged is set on the table itself, without drawing the
  // rows again; the saved width takes over when the drag ends.
  const table = useRef<HTMLDivElement>(null);
  const preview = (over: { id: ColumnId; width: number } | null) => {
    const shown = over
      ? tableLayout(columns, { ...widths, [over.id]: over.width }, actions ? actionsWidth : undefined)
      : grid;
    table.current?.style.setProperty("--tt-columns", shown.template);
    table.current?.style.setProperty("--tt-min-width", `${shown.minWidth}px`);
  };

  const activeIndexRendered = active !== null && active >= from && active <= to;
  const rows: ReactNode[] = [];
  for (let index = from; index <= to; index++) {
    const row = getRow(index);
    const selected = picked !== null ? row !== undefined && picked.has(row.recordingId) : active === index;
    rows.push(
      <div
        key={index}
        id={`${id}-row-${index}`}
        role="row"
        aria-rowindex={index + 2}
        aria-selected={selected}
        aria-busy={row === undefined || undefined}
        className={[
          styles.row,
          index % 2 === 1 ? styles.stripe : "",
          selected ? styles.selected : "",
          picked !== null && active === index ? styles.focused : "",
        ]
          .filter(Boolean)
          .join(" ")}
        style={{ transform: `translateY(${index * rowHeight}px)` }}
        onClick={(event) => {
          if (event.shiftKey) extendTo(index);
          else if (event.ctrlKey || event.metaKey) toggle(index);
          else select(index);
        }}
        // A Shift+click selects rows, not the text between them.
        onMouseDown={(event) => {
          if (event.shiftKey) event.preventDefault();
        }}
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
    <div ref={table} className={styles.table} style={cssVars}>
      <div
        ref={viewport}
        className={styles.viewport}
        role="grid"
        aria-label={label}
        aria-rowcount={total + 1}
        aria-colcount={defs.length + (actions ? 1 : 0)}
        aria-multiselectable
        aria-activedescendant={activeIndexRendered ? `${id}-row-${active}` : undefined}
        tabIndex={0}
        onKeyDown={onKeyDown}
        onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
      >
        <div role="rowgroup" className={styles.header}>
          <ColumnHeadings
            columns={columns}
            widths={widths}
            sort={sort}
            onSort={onSort}
            onReorder={onReorder && ((id, to) => onReorder(moveColumnTo(columns, id, to)))}
            onResize={onResize && ((id, width) => onResize({ ...widths, [id]: width }))}
            onPreview={preview}
            actionsHeading={actions ? t("columns.actions") : undefined}
          />
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
