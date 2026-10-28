import type { TableColumns } from "../bindings";
import { COLUMNS, DEFAULT_COLUMNS, type ColumnId, type TableScreen } from "./columns";

/**
 * Which columns a screen shows (1cA-5): the user's choice, worked out
 * against the columns this version has. Plain functions, so the rules can be
 * tested without a screen.
 *
 * The backend keeps two lists of ids for a choice and does not know what
 * they mean: the ones **shown**, in order, and the ones **known** when the
 * choice was saved. Together they say what to do with a column that has
 * changed since:
 * - an id that is no longer a column is dropped, never an error;
 * - a column added later is hidden, because it is not in `known` and so it
 *   was not hidden by the user, unless it is one of the screen's default
 *   columns: that appears;
 * - the title can't be hidden.
 *
 * A choice also keeps the widths the user dragged columns to (1cB-21): a
 * width for a column that is no longer one is dropped, and one outside what
 * a column allows is brought back inside. A choice saved before widths
 * existed has none.
 */

/** Every column, in the order the editor lists the hidden ones. */
export const ALL_COLUMNS: readonly ColumnId[] = [
  "title",
  "artist",
  "bpm",
  "key",
  "time",
  "genre",
  "rating",
  "dateAdded",
  "file",
  "folder",
];

/** The widest a column can be dragged to, in pixels. */
export const WIDEST = 1200;

/** The widths the user dragged columns to, in pixels; a column without one has its own. */
export type ColumnWidths = Partial<Record<ColumnId, number>>;

/** A stored choice; one saved before widths existed has none. */
export type StoredChoice = Pick<TableColumns, "shown" | "known"> & Partial<Pick<TableColumns, "widths">>;

/** The column that can never be hidden. */
export const ALWAYS_SHOWN: ColumnId = "title";

function isColumn(id: string): id is ColumnId {
  return Object.hasOwn(COLUMNS, id);
}

/**
 * The columns to draw: the screen's default when nothing was chosen,
 * otherwise the choice made good against this version's columns.
 */
export function resolveColumns(
  screen: TableScreen,
  stored: StoredChoice | null | undefined,
): readonly ColumnId[] {
  const defaults = DEFAULT_COLUMNS[screen];
  if (stored === null || stored === undefined) return defaults;

  const shown: ColumnId[] = [];
  for (const id of stored.shown) {
    if (isColumn(id) && !shown.includes(id)) shown.push(id);
  }
  // A default column this version has and the choice has never heard of.
  // It goes after the default column that comes before it, else before the
  // one that comes after it, else at the end.
  defaults.forEach((id, at) => {
    if (stored.known.includes(id) || shown.includes(id)) return;
    const before = defaults.slice(0, at).reverse().find((other) => shown.includes(other));
    const after = defaults.slice(at + 1).find((other) => shown.includes(other));
    const place =
      before !== undefined
        ? shown.indexOf(before) + 1
        : after !== undefined
          ? shown.indexOf(after)
          : shown.length;
    shown.splice(place, 0, id);
  });
  if (!shown.includes(ALWAYS_SHOWN)) shown.unshift(ALWAYS_SHOWN);
  return shown;
}

/** `px` as a width `id` can have: whole pixels, no less than its least and no more than `WIDEST`. */
export function clampWidth(id: ColumnId, px: number): number {
  return Math.round(Math.min(WIDEST, Math.max(COLUMNS[id].min, px)));
}

/** The widths of a stored choice, made good against this version's columns. */
export function resolveWidths(stored: StoredChoice | null | undefined): ColumnWidths {
  const widths: ColumnWidths = {};
  for (const [id, px] of Object.entries(stored?.widths ?? {})) {
    if (isColumn(id) && Number.isFinite(px)) widths[id] = clampWidth(id, px);
  }
  return widths;
}

/**
 * What to store for a choice: the columns shown, every column there is now,
 * and the widths dragged to.
 */
export function storedFrom(shown: readonly ColumnId[], widths: ColumnWidths = {}): TableColumns {
  return { shown: [...shown], known: [...ALL_COLUMNS], widths: { ...widths } };
}

/** The columns not shown, in the editor's order. */
export function hiddenColumns(shown: readonly ColumnId[]): ColumnId[] {
  return ALL_COLUMNS.filter((id) => !shown.includes(id));
}

/** `shown` with `id` moved one place up (-1) or down (1); unchanged at either end. */
export function moveColumn(shown: readonly ColumnId[], id: ColumnId, by: -1 | 1): ColumnId[] {
  const at = shown.indexOf(id);
  const to = at + by;
  const next = [...shown];
  if (at < 0 || to < 0 || to >= next.length) return next;
  [next[at], next[to]] = [next[to], next[at]];
  return next;
}

/**
 * `shown` with `id` moved to place `to`, counted in the list without it (0
 * is first, `length - 1` is last). Unchanged if `id` isn't shown.
 */
export function moveColumnTo(shown: readonly ColumnId[], id: ColumnId, to: number): ColumnId[] {
  const rest = shown.filter((other) => other !== id);
  if (rest.length === shown.length) return [...shown];
  rest.splice(Math.min(Math.max(0, to), rest.length), 0, id);
  return rest;
}

/** `shown` without `id`. The title stays. */
export function hideColumn(shown: readonly ColumnId[], id: ColumnId): ColumnId[] {
  return id === ALWAYS_SHOWN ? [...shown] : shown.filter((other) => other !== id);
}

/** `shown` with `id` added at the end. */
export function showColumn(shown: readonly ColumnId[], id: ColumnId): ColumnId[] {
  return shown.includes(id) ? [...shown] : [...shown, id];
}

/** Whether `shown` is exactly the screen's default. */
export function isDefault(screen: TableScreen, shown: readonly ColumnId[]): boolean {
  const defaults = DEFAULT_COLUMNS[screen];
  return defaults.length === shown.length && defaults.every((id, at) => shown[at] === id);
}

/**
 * The grid the table's rows and heading row share (1cB-21): a column the
 * user dragged to a width is exactly that wide; the others start at their
 * width and the growing ones share what is left. `minWidth` is the width the
 * table needs before it scrolls sideways.
 */
export function tableLayout(
  columns: readonly ColumnId[],
  widths: ColumnWidths,
  actionsWidth?: number,
): { template: string; minWidth: number } {
  const parts: string[] = [];
  let minWidth = actionsWidth ?? 0;
  for (const id of columns) {
    const def = COLUMNS[id];
    const own = widths[id];
    if (own !== undefined) {
      parts.push(`${own}px`);
      minWidth += own;
    } else {
      parts.push(def.grow ? `minmax(${def.width}px, ${def.grow}fr)` : `${def.width}px`);
      minWidth += def.width;
    }
  }
  if (actionsWidth !== undefined) parts.push(`${actionsWidth}px`);
  return { template: parts.join(" "), minWidth };
}
