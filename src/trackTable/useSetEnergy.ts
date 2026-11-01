import { useMutation, useQueryClient } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { reloadLists } from "../app/lists";
import { commands, type EnergySet } from "../bindings";
import { CRATE_SUMMARY_QUERY_KEY } from "../crates/useCrateSummary";

/**
 * Gives tracks an energy from 1 to 10 (1cB-8; ROADMAP 1.11): one operation
 * for all of them, so one Undo, in the top bar, puts them all back. The
 * track table's rows show it (the Energy column, 1cD) and the Details panel
 * too, and the Energy and No energy doors list by it and the Overview counts
 * it (ROADMAP 1.13), so those are asked for again, and a crate's summary
 * line shows the range (1cB-15). A long list is the index job's: its end
 * reloads them. The top bar's Undo asks for itself after any action.
 */
export function useSetEnergy() {
  const queryClient = useQueryClient();
  return useMutation<EnergySet, Error, { recordingIds: readonly number[]; energy: number }>({
    mutationFn: ({ recordingIds, energy }) => unwrap(commands.setEnergy([...recordingIds], energy)),
    onSuccess: () =>
      Promise.all([
        queryClient.invalidateQueries({ queryKey: CRATE_SUMMARY_QUERY_KEY }),
        reloadLists(queryClient, "library", "crates", "overview"),
      ]),
  });
}
