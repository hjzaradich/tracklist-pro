import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { commands, type Overview } from "../bindings";

/** Every Overview number is kept under this key (ROADMAP 1.13). */
export const OVERVIEW_QUERY_KEY = ["overview"] as const;

/** How often the numbers are asked again while background work can still change them. */
export const COUNTING_INTERVAL_MS = 5_000;

/** True while any of the numbers can still change because background work runs. */
export function isCounting(overview: Overview): boolean {
  const { files, rekordbox, quality, index } = overview.counting;
  return files || rekordbox || quality || index;
}

/**
 * Every number the Overview shows (1cA-12). `libraryTracks` is the Library's
 * size now: when it changes, the numbers are asked again, and the old ones
 * stay on screen until the new ones arrive (the command is slow on a big
 * Library). While background work is counting, they're asked again every
 * {@link COUNTING_INTERVAL_MS}.
 */
export function useOverview(libraryTracks: number | undefined) {
  return useQuery({
    queryKey: [...OVERVIEW_QUERY_KEY, libraryTracks],
    queryFn: (): Promise<Overview> => unwrap(commands.overview()),
    enabled: libraryTracks !== undefined && libraryTracks > 0,
    placeholderData: keepPreviousData,
    refetchInterval: (query) =>
      query.state.data !== undefined && isCounting(query.state.data) ? COUNTING_INTERVAL_MS : false,
  });
}
