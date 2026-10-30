import { useQueryClient } from "@tanstack/react-query";
import { useWhenJobsEnd } from "../activity/useWhenJobsEnd";
import { listKeys, reloadLists } from "../app/lists";

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
    void reloadLists(queryClient, "library", "missing", "crates", "folders", "tagSources", "review");
    // The Overview's numbers take long to work out on a big Library: a job
    // ending while they're on their way doesn't start them over (the numbers
    // are asked again every few seconds while jobs are counting).
    for (const queryKey of listKeys("overview")) {
      void queryClient.invalidateQueries({ queryKey }, { cancelRefetch: false });
    }
  });
}
