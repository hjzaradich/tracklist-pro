import { useQuery } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { commands } from "../bindings";

/** Invalidate this after anything adds or removes a Library track. */
export const LIBRARY_TRACKS_QUERY_KEY = ["libraryTracks"] as const;

/**
 * The Library's track table (1cA-3) keeps its windows of rows under this key,
 * which starts with {@link LIBRARY_TRACKS_QUERY_KEY}, so whatever invalidates
 * the Library's tracks fetches the windows on screen again.
 */
export const LIBRARY_TABLE_QUERY_KEY = [...LIBRARY_TRACKS_QUERY_KEY, "table"] as const;

/**
 * The Library's size and change stamp, under a key that starts with
 * {@link LIBRARY_TRACKS_QUERY_KEY} too, so it is asked again whenever the
 * Library's lists are.
 */
export const LIBRARY_SUMMARY_QUERY_KEY = [...LIBRARY_TRACKS_QUERY_KEY, "summary"] as const;

/**
 * How many tracks the Library holds (`tracks`) and a number that moves
 * whenever its list would come back different (`changeStamp`: a track added,
 * removed, retitled or relinked), compared only for equality. A cheap count
 * on the backend: nothing here reads the list itself, which at 100,000
 * tracks is tens of megabytes.
 */
export function useLibrarySummary() {
  return useQuery({
    queryKey: LIBRARY_SUMMARY_QUERY_KEY,
    queryFn: () => unwrap(commands.librarySummary()),
  });
}
