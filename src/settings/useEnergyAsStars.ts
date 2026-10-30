import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { commands } from "../bindings";
import { SEND_STATE_QUERY_KEY } from "../send/useSend";

// The opt-in to sending energy as rekordbox's star rating (1cC-2, ROADMAP
// 1.9), kept in the backend's `setting` table. Off by default.

export const ENERGY_AS_STARS_QUERY_KEY = ["setting", "energy_as_stars"] as const;

/** What the screen knows of the setting: never "off" until it has been read. */
export type EnergyAsStars =
  | { status: "loading" }
  | { status: "error"; error: unknown }
  | { status: "ready"; on: boolean };

/**
 * Whether energy is sent as the star rating. While it is being read, or if it
 * can't be read, the answer is not "off": the screen must not offer a change
 * to a setting whose state it doesn't know.
 */
export function useEnergyAsStars(): EnergyAsStars {
  const { data, error, isError } = useQuery({
    queryKey: ENERGY_AS_STARS_QUERY_KEY,
    queryFn: () => unwrap(commands.readEnergyAsStars()),
  });
  if (data !== undefined) return { status: "ready", on: data };
  return isError ? { status: "error", error } : { status: "loading" };
}

/** Turns the setting on or off. It is never an operation, so there is nothing to Undo. */
export function useSetEnergyAsStars() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (on: boolean) => {
      await unwrap(commands.setEnergyAsStars(on));
      return on;
    },
    onSuccess: (on) => {
      queryClient.setQueryData(ENERGY_AS_STARS_QUERY_KEY, on);
      // A review held under the other setting no longer says what a send would
      // do. The Overview's send reads it from this state, so it is asked again;
      // the user reads the export again for a fresh review (the send's own
      // write re-checks, so a stale one can't be sent). Nothing here reopens
      // the send: leaving the review is the same as any other navigation.
      void queryClient.invalidateQueries({ queryKey: SEND_STATE_QUERY_KEY });
    },
  });
}

/**
 * How many tracks' stars turning the setting on would change, from the
 * newest read, asked each time (never cached: the answer follows the reads).
 */
export function useEnergyStarsWouldChange() {
  return useMutation({ mutationFn: () => unwrap(commands.energyAsStarsWouldChange()) });
}
