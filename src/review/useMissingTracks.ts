import { useQuery } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { commands, type MissingList } from "../bindings";

export const MISSING_TRACKS_QUERY_KEY = ["missingTracks"] as const;

function fetchMissingTracks(): Promise<MissingList> {
  return unwrap(commands.missingTracks());
}

/** The rekordbox tracks with no file, by last known folder (1aD-5). */
export function useMissingTracks() {
  return useQuery({ queryKey: MISSING_TRACKS_QUERY_KEY, queryFn: fetchMissingTracks });
}
