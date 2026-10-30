import { useQuery, useQueryClient, type QueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { CRATES_LIST_QUERY_KEY } from "../crates/useCrates";
import type { LibrarySummary } from "../bindings";
import { LIBRARY_SUMMARY_QUERY_KEY } from "../library/useLibrarySummary";

// The send review's last read (what the Overview's header repeats: "No
// tracks or crates to send") describes the Library as it was when the export
// was read. The backend keeps that read until the next one and can't tell
// that the Library moved on, so the screens do: what shows the Library (its
// change stamp, which moves when any track in it changes, and its crates and
// their counts) says when it last changed, and a read made before that is old.

/** When the Library last changed, in milliseconds since the Unix epoch; 0 before any change. */
export const LIBRARY_CHANGED_QUERY_KEY = ["send", "libraryChangedAt"] as const;

/**
 * What is watched for a change to what a send would carry, and what tells a
 * change from the same answer again: the Library's change stamp (a retitle
 * leaves its count alone but moves the stamp), and the crate list, which
 * TanStack hands back as the very same object when nothing in it changed.
 */
const WATCHED: { key: readonly unknown[]; signature: (data: unknown) => unknown }[] = [
  { key: LIBRARY_SUMMARY_QUERY_KEY, signature: (data) => (data as LibrarySummary).changeStamp },
  { key: CRATES_LIST_QUERY_KEY, signature: (data) => data },
];

function watching(queryKey: readonly unknown[]) {
  return WATCHED.find(
    ({ key }) => key.length === queryKey.length && key.every((part, i) => part === queryKey[i]),
  );
}

/**
 * Notes the moment a watched answer comes back different from the one before
 * it. The first answer is no change, and neither is the same answer again
 * (they are asked again whenever a job ends). Call it once, in the
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
    const watched = watching(query.queryKey);
    if (watched === undefined) return;
    const before = seen.get(query.queryHash);
    const now = watched.signature(query.state.data);
    seen.set(query.queryHash, now);
    if (before !== undefined && before !== now) {
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
