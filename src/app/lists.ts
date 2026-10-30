import type { QueryClient } from "@tanstack/react-query";
import { AFTER_SEND_LISTS_QUERY_KEY } from "../afterSend/useAfterSendLists";
import { ALL_MUSIC_QUERY_KEY } from "../allMusic/useAllMusic";
import { CRATES_QUERY_KEY } from "../crates/useCrates";
import { REKORDBOX_OFFER_QUERY_KEY } from "../firstRun/queryKeys";
import { LIBRARY_TRACKS_QUERY_KEY } from "../library/useLibrarySummary";
import { OVERVIEW_QUERY_KEY } from "../overview/useOverview";
import { MISSING_TRACKS_QUERY_KEY } from "../review/useMissingTracks";
import { REVIEW_QUERY_KEYS } from "../review/useReview";
import { MUSIC_FOLDERS_QUERY_KEY } from "../scan/useMusicFolders";
import { TRACK_TAG_SOURCES_QUERY_KEY } from "../shell/useTrackTagSources";

/**
 * The one place that names the lists a change can leave out of date, in
 * groups. Everything that asks for lists again after a change (a job
 * ending, Undo, a Review decision, an add to the Library or a crate) picks
 * the groups it can affect and calls {@link reloadLists}, so a new list is
 * added here once, to the group it belongs to, and every one of those places
 * has it. Each group is a function, so that the key constants are read when
 * it is asked for, not when this module loads (the hooks that own the keys
 * import this module too).
 */
type Keys = readonly (readonly unknown[])[];

const GROUPS = {
  /** Library tracks (and their table), and All music, which marks which tracks are in the Library. */
  library: () => [LIBRARY_TRACKS_QUERY_KEY, ALL_MUSIC_QUERY_KEY],
  /** The offer to add rekordbox tracks to the Library. */
  offer: () => [REKORDBOX_OFFER_QUERY_KEY],
  /** The queue and the comparison of Review. */
  review: () => REVIEW_QUERY_KEYS,
  /** Tracks whose file is missing. */
  missing: () => [MISSING_TRACKS_QUERY_KEY],
  /** Crates, their tracks and counts. */
  crates: () => [CRATES_QUERY_KEY],
  /** Where each value of a track comes from. */
  tagSources: () => [TRACK_TAG_SOURCES_QUERY_KEY],
  /** Music folders and their notes. */
  folders: () => [MUSIC_FOLDERS_QUERY_KEY],
  /** What is left to do in rekordbox after a send. */
  afterSend: () => [AFTER_SEND_LISTS_QUERY_KEY],
  /** The Overview's numbers. */
  overview: () => [OVERVIEW_QUERY_KEY],
} satisfies Record<string, () => Keys>;

/** A group of lists that are asked for again together. */
export type ListGroup = keyof typeof GROUPS;

/** The query keys of `groups`, each once. */
export function listKeys(...groups: ListGroup[]): (readonly unknown[])[] {
  return [...new Set(groups.flatMap((group): Keys => GROUPS[group]()))];
}

/**
 * Marks the lists of `groups` out of date and fetches again the ones on
 * screen. Resolves when those are back.
 */
export function reloadLists(queryClient: QueryClient, ...groups: ListGroup[]): Promise<unknown> {
  return Promise.all(
    listKeys(...groups).map((queryKey) => queryClient.invalidateQueries({ queryKey })),
  );
}
