import { useQueryClient } from "@tanstack/react-query";
import { useWhenJobsEnd } from "../activity/useWhenJobsEnd";
import { ALL_MUSIC_QUERY_KEY } from "../allMusic/useAllMusic";
import { CRATES_QUERY_KEY } from "../crates/useCrates";
import { LIBRARY_TRACKS_QUERY_KEY } from "../library/useLibraryTracks";
import { MISSING_TRACKS_QUERY_KEY } from "../review/useMissingTracks";
import { REVIEW_QUERY_KEYS } from "../review/useReview";
import { OVERVIEW_QUERY_KEY } from "../overview/useOverview";
import { MUSIC_FOLDERS_QUERY_KEY } from "../scan/useMusicFolders";
import { TRACK_TAG_SOURCES_QUERY_KEY } from "./useTrackTagSources";

/**
 * Asks again for the lists background work feeds, whenever a task ends: a
 * scan makes All music's tracks and a folder's notes, a relink decides
 * what's Missing and what Review holds, and a scan marks a Library track's file missing. The track
 * tables (1cA-3) sort and filter by rows the index job writes, so its end reloads them too (a
 * crate's included). Only lists on screen are fetched again. Call it once, in the shell.
 */
export function useReloadListsWhenJobsEnd() {
  const queryClient = useQueryClient();
  useWhenJobsEnd(() => {
    for (const queryKey of [
      ALL_MUSIC_QUERY_KEY,
      MISSING_TRACKS_QUERY_KEY,
      LIBRARY_TRACKS_QUERY_KEY,
      CRATES_QUERY_KEY,
      MUSIC_FOLDERS_QUERY_KEY,
      TRACK_TAG_SOURCES_QUERY_KEY,
      ...REVIEW_QUERY_KEYS,
    ]) {
      void queryClient.invalidateQueries({ queryKey });
    }
    // The Overview's numbers take long to work out on a big Library: a job
    // ending while they're on their way doesn't start them over (the numbers
    // are asked again every few seconds while jobs are counting).
    void queryClient.invalidateQueries({ queryKey: OVERVIEW_QUERY_KEY }, { cancelRefetch: false });
  });
}
