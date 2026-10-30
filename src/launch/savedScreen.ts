import { STAGES } from "../shell/stages";

/**
 * Where the app reopens (ROADMAP 1.13, "Launch"): the screen the user was
 * last on, and which crate if one was open. It is the user's own habit, not
 * Library data: no operation, no Undo, and "Start fresh" (which only empties
 * the Library) leaves it alone. Like the volume and the theme it lives in
 * the webview's `localStorage`, and the app works when storage is missing,
 * full or refuses to answer: it then opens as it does on a first run. A
 * stored value this version can't read is ignored.
 */
export const SCREEN_STORAGE_KEY = "tracklist-pro.launch.screen";

/** The screens that are remembered: the stages and Settings (the after-send lists are not one). */
const REMEMBERED_PATHS: readonly string[] = [...STAGES.map((stage) => stage.path), "/settings"];

export interface SavedScreen {
  /** A stage's path, e.g. `/library`. */
  path: string;
  /** The crate or folder open on the Crates screen. */
  crate?: number;
}

/** A crate id is a whole number above zero; anything else chooses nothing. */
export function isCrateId(id: unknown): id is number {
  return typeof id === "number" && Number.isSafeInteger(id) && id > 0;
}

/** The screen at `pathname` with `search`, if it is one that is remembered. */
export function screenAt(pathname: string, search: Record<string, unknown>): SavedScreen | null {
  if (!REMEMBERED_PATHS.includes(pathname)) return null;
  if (pathname === "/crates" && isCrateId(search.crate)) return { path: pathname, crate: search.crate };
  return { path: pathname };
}

/** The screen a stored text stands for; `null` for none, or for one that is damaged. */
export function parseSavedScreen(raw: string | null): SavedScreen | null {
  if (raw === null) return null;
  try {
    const stored = JSON.parse(raw) as { path?: unknown; crate?: unknown } | null;
    if (typeof stored?.path !== "string") return null;
    return screenAt(stored.path, { crate: stored.crate });
  } catch {
    return null;
  }
}

/** The screen the user was last on, if any. */
export function readSavedScreen(): SavedScreen | null {
  try {
    return parseSavedScreen(localStorage.getItem(SCREEN_STORAGE_KEY));
  } catch {
    return null;
  }
}

/** Remembers `screen`. Not worth stopping for if it can't: the app opens on Overview next time. */
export function writeSavedScreen(screen: SavedScreen): void {
  try {
    localStorage.setItem(SCREEN_STORAGE_KEY, JSON.stringify(screen));
  } catch {
    // Storage is blocked or full.
  }
}
