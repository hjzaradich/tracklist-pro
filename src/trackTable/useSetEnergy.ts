import { useMutation, useQueryClient } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { commands, type EnergySet } from "../bindings";
import { CRATE_SUMMARY_QUERY_KEY } from "../crates/useCrateSummary";

/**
 * Gives tracks an energy from 1 to 10 (1cB-8; ROADMAP 1.11): one operation
 * for all of them, so one Undo, in the top bar, puts them all back. No table
 * shows energy yet, so no list is asked for again; a crate's summary line
 * shows the range (1cB-15), so those are. The top bar's Undo asks for itself
 * after any action.
 */
export function useSetEnergy() {
  const queryClient = useQueryClient();
  return useMutation<EnergySet, Error, { recordingIds: readonly number[]; energy: number }>({
    mutationFn: ({ recordingIds, energy }) => unwrap(commands.setEnergy([...recordingIds], energy)),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: CRATE_SUMMARY_QUERY_KEY }),
  });
}
