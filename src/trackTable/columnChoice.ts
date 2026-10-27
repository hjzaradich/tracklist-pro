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
  stored: TableColumns | null | undefined,
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

/** What to store for a choice: the columns shown, and every column there is now. */
export function storedFrom(shown: readonly ColumnId[]): TableColumns {
  return { shown: [...shown], known: [...ALL_COLUMNS] };
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
