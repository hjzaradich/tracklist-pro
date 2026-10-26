import { useQuery } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { commands, type LibraryTrack } from "../bindings";

/** Invalidate this after anything adds or removes a Library track. */
export const LIBRARY_TRACKS_QUERY_KEY = ["libraryTracks"] as const;

/**
 * The Library's track table (1cA-3) keeps its windows of rows under this key,
 * which starts with {@link LIBRARY_TRACKS_QUERY_KEY}, so whatever invalidates
 * the Library's tracks fetches the windows on screen again.
 */
export const LIBRARY_TABLE_QUERY_KEY = [...LIBRARY_TRACKS_QUERY_KEY, "table"] as const;

function fetchLibraryTracks(): Promise<LibraryTrack[]> {
  return unwrap(commands.libraryTracks());
}

/** Every Library track, already sorted for the Library list (1aD-6). */
export function useLibraryTracks() {
  return useQuery({ queryKey: LIBRARY_TRACKS_QUERY_KEY, queryFn: fetchLibraryTracks });
}
