import { useQueryClient } from "@tanstack/react-query";
import { useWhenJobsEnd } from "../activity/useWhenJobsEnd";
import { ALL_MUSIC_QUERY_KEY } from "../allMusic/useAllMusic";
import { LIBRARY_TRACKS_QUERY_KEY } from "../library/useLibraryTracks";
import { MISSING_TRACKS_QUERY_KEY } from "../review/useMissingTracks";
import { MUSIC_FOLDERS_QUERY_KEY } from "../scan/useMusicFolders";

/**
 * Asks again for the lists background work feeds, whenever a task ends: a
 * scan makes All music's tracks and a folder's notes, a relink decides
 * what's Missing, and a scan marks a Library track's file missing. Only
 * lists on screen are fetched again. Call it once, in the shell.
 */
export function useReloadListsWhenJobsEnd() {
  const queryClient = useQueryClient();
  useWhenJobsEnd(() => {
    for (const queryKey of [
      ALL_MUSIC_QUERY_KEY,
      MISSING_TRACKS_QUERY_KEY,
      LIBRARY_TRACKS_QUERY_KEY,
      MUSIC_FOLDERS_QUERY_KEY,
    ]) {
      void queryClient.invalidateQueries({ queryKey });
    }
  });
}
