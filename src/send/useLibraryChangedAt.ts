import { useQuery, useQueryClient, type QueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { CRATES_LIST_QUERY_KEY } from "../crates/useCrates";
import { LIBRARY_TRACKS_QUERY_KEY } from "../library/useLibraryTracks";

// The send review's last read (what the Overview's header repeats: "No
// tracks or crates to send") describes the Library as it was when the export
// was read. The backend keeps that read until the next one and can't tell
// that the Library moved on, so the screens do: the lists that show the
// Library (its tracks, its crates and their counts) say when they last
// changed, and a read made before that is old.

/** When the Library last changed, in milliseconds since the Unix epoch; 0 before any change. */
export const LIBRARY_CHANGED_QUERY_KEY = ["send", "libraryChangedAt"] as const;

/** The lists whose change is a change to what a send would carry. */
const WATCHED = [LIBRARY_TRACKS_QUERY_KEY, CRATES_LIST_QUERY_KEY] as const;

function isWatched(queryKey: readonly unknown[]): boolean {
  return WATCHED.some(
    (watched) =>
      watched.length === queryKey.length && watched.every((part, i) => part === queryKey[i]),
  );
}

/**
 * Notes the moment a watched list comes back different from the one before
 * it. The first answer is no change, and neither is the same answer again
 * (the lists are asked again whenever a job ends). Call it once, in the
 * shell, so a change made on any screen is noted.
 */
export function useNoteLibraryChanges() {
  const queryClient = useQueryClient();
  useEffect(() => noteLibraryChanges(queryClient), [queryClient]);
}

/** Starts noting; returns the function that stops it. Exported for the shell's hook only. */
export function noteLibraryChanges(queryClient: QueryClient): () => void {
  const seen = new Map<string, unknown>();
  return queryClient.getQueryCache().subscribe((event) => {
    if (event.type !== "updated" || event.action.type !== "success") return;
    const { query } = event;
    if (!isWatched(query.queryKey)) return;
    const before = seen.get(query.queryHash);
    seen.set(query.queryHash, query.state.data);
    if (before !== undefined && before !== query.state.data) {
      queryClient.setQueryData(LIBRARY_CHANGED_QUERY_KEY, Date.now());
    }
  });
}

/** When the Library last changed (see {@link useNoteLibraryChanges}). */
export function useLibraryChangedAt(): number {
  return useQuery({
    queryKey: LIBRARY_CHANGED_QUERY_KEY,
    // Never asked: only a change sets it.
    queryFn: () => 0,
    initialData: 0,
    staleTime: Infinity,
    gcTime: Infinity,
  }).data;
}
