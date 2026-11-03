import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { reloadLists } from "../app/lists";
import { commands, type EnergySet } from "../bindings";
import { CRATE_SUMMARY_QUERY_KEY } from "../crates/useCrateSummary";
import { useUndoneCount } from "../shell/undoneStore";

/** The key of "how many of these tracks have an energy of the user's own"; a set or a clear asks again. */
export const USER_ENERGY_COUNT_KEY = ["userEnergyCount"] as const;

/**
 * Takes the user's own energy away from tracks (1cD-XE): one operation, so one
 * Undo puts it back. Another source's energy then shows. The lists are asked
 * for again as after a set (`useSetEnergy`).
 */
export function useClearEnergy() {
  const queryClient = useQueryClient();
  return useMutation<EnergySet, Error, { recordingIds: readonly number[] }>({
    mutationFn: ({ recordingIds }) => unwrap(commands.clearEnergy([...recordingIds])),
    onSuccess: () =>
      Promise.all([
        queryClient.invalidateQueries({ queryKey: USER_ENERGY_COUNT_KEY }),
        queryClient.invalidateQueries({ queryKey: CRATE_SUMMARY_QUERY_KEY }),
        reloadLists(queryClient, "library", "crates", "overview"),
      ]),
  });
}

/**
 * How many of `recordingIds` have an energy of the user's own, so "Clear" is
 * offered only when there is something to clear. Nothing is asked for an
 * empty list. An Undo asks again.
 */
export function useUserEnergyCount(recordingIds: readonly number[]): number {
  const undone = useUndoneCount((state) => state.count);
  const { data } = useQuery({
    queryKey: [...USER_ENERGY_COUNT_KEY, undone, recordingIds],
    queryFn: () => unwrap(commands.userEnergyCount([...recordingIds])),
    enabled: recordingIds.length > 0,
  });
  return recordingIds.length > 0 ? (data ?? 0) : 0;
}
