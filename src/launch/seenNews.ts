import type { Overview, ReviewCounts } from "../bindings";

/**
 * What counts as new at launch (ROADMAP 1.13). Narrowly: the things a scan
 * or a read brings in, and nothing the user's own work moves. That is the
 * tracks outside the Library that the scan found, and the four groups of
 * items waiting in Review. The other lines of the to-do list (missing tags,
 * art, quality, analysis, tracks in no crate) describe the Library the user
 * is working on, and adding a track to it moves them, so they are never news.
 *
 * Something is new when one of these holds more than the last time the user
 * saw it: the last time the Overview showed all of them with nothing still
 * being counted, or the least they held since (so that a number the user
 * worked down, and that then grows again, is news). Only while the Library
 * holds the tracks it held then: a Library the user has changed (tracks added
 * or removed, "Start fresh") has moved these numbers by their own hand, so
 * there is no record, and the first complete numbers start a new one.
 */
export const SEEN_STORAGE_KEY = "tracklist-pro.launch.seen";

export const NEWS_KEYS = [
  "outsideLibrary",
  "needsCloserLook",
  "clearBestFile",
  "qualityProblems",
  "conflicts",
] as const;

export type NewsKey = (typeof NEWS_KEYS)[number];

/**
 * How many each of the {@link NEWS_KEYS} held, and how many tracks the
 * Library held then. The track count says whether the numbers can be
 * compared at all: when it differs from the Library's now, the user changed
 * the Library (added or removed tracks, started fresh) and moved the numbers
 * themselves, so the record is dropped and started again.
 */
export type Seen = Record<NewsKey, number> & { tracks: number };

/** The numbers the to-do list shows for the {@link NEWS_KEYS}. */
export function newsNumbers(overview: Overview, review: ReviewCounts): Seen {
  return {
    tracks: overview.tracks,
    outsideLibrary: overview.outsideLibrary,
    needsCloserLook: review.needsCloserLook,
    clearBestFile: review.clearBestFile,
    qualityProblems: review.qualityProblems,
    conflicts: review.conflicts,
  };
}

/** True if any number holds more than it did when last seen. */
export function hasNews(now: Seen, seen: Seen): boolean {
  return NEWS_KEYS.some((key) => now[key] > seen[key]);
}

/** Each number as the lesser of the two. */
export function lowest(a: Seen, b: Seen): Seen {
  const out = { ...a };
  for (const key of NEWS_KEYS) out[key] = Math.min(a[key], b[key]);
  return out;
}

function isSeen(value: unknown): value is Seen {
  if (typeof value !== "object" || value === null) return false;
  const record = value as Record<string, unknown>;
  return [...NEWS_KEYS, "tracks"].every((key) => {
    const count = record[key];
    return typeof count === "number" && Number.isSafeInteger(count) && count >= 0;
  });
}

/** The numbers as last seen; `null` before any were seen, or if the stored value is damaged. */
export function readSeen(): Seen | null {
  try {
    const raw = localStorage.getItem(SEEN_STORAGE_KEY);
    if (raw === null) return null;
    const stored: unknown = JSON.parse(raw);
    if (!isSeen(stored)) return null;
    const seen = { tracks: stored.tracks } as Seen;
    for (const key of NEWS_KEYS) seen[key] = stored[key];
    return seen;
  } catch {
    return null;
  }
}

/** Forgets what was seen. */
export function forgetSeen(): void {
  try {
    localStorage.removeItem(SEEN_STORAGE_KEY);
  } catch {
    // Storage is blocked: the record can't be read either.
  }
}

export function writeSeen(seen: Seen): void {
  try {
    localStorage.setItem(SEEN_STORAGE_KEY, JSON.stringify(seen));
  } catch {
    // Storage is blocked or full: the next launch can't tell what is new, so nothing is.
  }
}
