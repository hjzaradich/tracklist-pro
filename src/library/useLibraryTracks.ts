import { useQuery } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { commands, type LibraryTrack } from "../bindings";

/** Invalidate this after anything adds or removes a Library track. */
export const LIBRARY_TRACKS_QUERY_KEY = ["libraryTracks"] as const;

function fetchLibraryTracks(): Promise<LibraryTrack[]> {
  return unwrap(commands.libraryTracks());
}

/** Every Library track, already sorted for the Library list (1aD-6). */
export function useLibraryTracks() {
  return useQuery({ queryKey: LIBRARY_TRACKS_QUERY_KEY, queryFn: fetchLibraryTracks });
}
