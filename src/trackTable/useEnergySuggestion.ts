import { useQuery } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { commands, type EnergySuggestion } from "../bindings";

/** The suggestions for tracks: asked again when a background job ends (`estimates` in `app/lists`). */
export const ENERGY_SUGGESTION_QUERY_KEY = ["estimate", "energySuggestion"] as const;

/**
 * The app's suggested energy for a track, with its basis, or `null` for none
 * (ROADMAP 3.7). Only a suggestion: the track has no energy from it.
 */
export function useEnergySuggestion(recordingId: number) {
  return useQuery<EnergySuggestion | null, Error>({
    queryKey: [...ENERGY_SUGGESTION_QUERY_KEY, recordingId],
    queryFn: () => unwrap(commands.energySuggestion(recordingId)),
  });
}
