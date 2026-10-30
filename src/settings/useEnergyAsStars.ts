import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { commands } from "../bindings";
import { SEND_STATE_QUERY_KEY } from "../send/useSend";

// The opt-in to sending energy as rekordbox's star rating (1cC-2, ROADMAP
// 1.9), kept in the backend's `setting` table. Off by default.

export const ENERGY_AS_STARS_QUERY_KEY = ["setting", "energy_as_stars"] as const;

/** Whether energy is sent as the star rating. Off while it loads or if it can't be read. */
export function useEnergyAsStars(): boolean {
  const { data } = useQuery({
    queryKey: ENERGY_AS_STARS_QUERY_KEY,
    queryFn: () => unwrap(commands.readEnergyAsStars()),
  });
  return data === true;
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
      // A review made under the other setting no longer says what a send would do.
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
