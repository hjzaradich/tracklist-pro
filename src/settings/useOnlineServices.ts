import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { commands, type OnlineService, type OnlineServicesStatus } from "../bindings";

// The lookup services (ROADMAP 2.2): each is off until the user switches it
// on in Settings. The backend holds the switches and the Discogs token; the
// token never comes back to the screen, only whether one is saved.

/** Not in `app/lists.ts`: nothing but Settings shows it, so a change invalidates it directly. */
export const ONLINE_SERVICES_QUERY_KEY = ["setting", "online_services"] as const;

/** Which lookup services are on, whether AcoustID's key is missing, and whether a Discogs token is saved. */
export function useOnlineServicesStatus() {
  return useQuery({
    queryKey: ONLINE_SERVICES_QUERY_KEY,
    queryFn: (): Promise<OnlineServicesStatus> => unwrap(commands.onlineServicesStatus()),
  });
}

/** Switches one service on or off. */
export function useSetOnlineService(service: OnlineService) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (on: boolean) => unwrap(commands.setOnlineService(service, on)),
    onSettled: () => queryClient.invalidateQueries({ queryKey: ONLINE_SERVICES_QUERY_KEY }),
  });
}

/** Checks a Discogs token with Discogs and keeps it only if Discogs accepts it. The answer says what came of it. */
export function useSaveDiscogsToken() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (token: string) => unwrap(commands.saveDiscogsToken(token)),
    // The typed token is the mutation's `variables`: drop the mutation as soon as nobody watches it, so no copy of the token outlives the save.
    gcTime: 0,
    onSettled: () => queryClient.invalidateQueries({ queryKey: ONLINE_SERVICES_QUERY_KEY }),
  });
}

/** Forgets the saved Discogs token. */
export function useRemoveDiscogsToken() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () => unwrap(commands.removeDiscogsToken()),
    onSettled: () => queryClient.invalidateQueries({ queryKey: ONLINE_SERVICES_QUERY_KEY }),
  });
}
