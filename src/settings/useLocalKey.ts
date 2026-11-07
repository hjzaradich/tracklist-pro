import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { commands } from "../bindings";

// The owner's test switch for the app's own key estimates (3C-E2, ROADMAP 3.7).
// Off by default; while off, no track shows a local key and the comparison is
// never asked for.

export const LOCAL_KEY_FEATURE_QUERY_KEY = ["setting", "feature", "localKey"] as const;
export const LOCAL_KEY_COMPARISON_QUERY_KEY = ["estimate", "localKeyComparison"] as const;

/** Whether the switch is on. */
export function useLocalKeyFeature() {
  return useQuery({
    queryKey: LOCAL_KEY_FEATURE_QUERY_KEY,
    queryFn: () => unwrap(commands.featureEnabled("localKey")),
  });
}

/**
 * Turns the switch on or off. The backend adds or takes away the local keys at
 * once, so every list that shows keys is asked again.
 */
export function useSetLocalKeyFeature() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (on: boolean) => {
      await unwrap(commands.setFeatureEnabled("localKey", on));
      return on;
    },
    onSuccess: (on) => queryClient.setQueryData(LOCAL_KEY_FEATURE_QUERY_KEY, on),
    onSettled: () =>
      queryClient.invalidateQueries({
        predicate: (query) => query.queryKey[0] !== "setting",
      }),
  });
}

/** How the local key agrees with rekordbox's; only asked for while the switch is on. */
export function useLocalKeyComparison(on: boolean) {
  return useQuery({
    queryKey: LOCAL_KEY_COMPARISON_QUERY_KEY,
    queryFn: () => unwrap(commands.compareLocalKeys()),
    enabled: on,
  });
}
