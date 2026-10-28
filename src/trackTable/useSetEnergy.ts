import { useMutation } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { commands, type EnergySet } from "../bindings";

/**
 * Gives tracks an energy from 1 to 10 (1cB-8; ROADMAP 1.11): one operation
 * for all of them, so one Undo, in the top bar, puts them all back. Nothing in
 * the lists changes (no screen shows energy yet), so no list is asked for
 * again; the top bar's Undo asks for itself after any action.
 */
export function useSetEnergy() {
  return useMutation<EnergySet, Error, { recordingIds: readonly number[]; energy: number }>({
    mutationFn: ({ recordingIds, energy }) => unwrap(commands.setEnergy([...recordingIds], energy)),
  });
}
