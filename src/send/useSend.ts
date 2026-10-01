import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { commands, type SendState } from "../bindings";

// The send flow (1aF-1): where the send is, and its two steps. Each step is
// a job (Activity); the state says how it ended.

export const SEND_STATE_QUERY_KEY = ["send", "state"] as const;

/** How often the state is asked again while a step is under way. */
export const WAITING_INTERVAL_MS = 1_000;

function fetchSendState(): Promise<SendState> {
  return unwrap(commands.sendState());
}

/**
 * Where the send is. While a step begun at revision `waitingSince` hasn't
 * ended (the revision hasn't moved), it's asked every
 * {@link WAITING_INTERVAL_MS}.
 */
export function useSendState(waitingSince: number | null) {
  return useQuery({
    queryKey: SEND_STATE_QUERY_KEY,
    queryFn: fetchSendState,
    refetchInterval: (query) =>
      waitingSince !== null && query.state.data?.revision === waitingSince
        ? WAITING_INTERVAL_MS
        : false,
  });
}

/**
 * Reads rekordbox's export again and builds the preflight, as a job: `path`
 * (a file just picked) becomes the chosen export; `null` reads the chosen
 * one. Resolves to the job's id.
 */
export function usePrepareSend() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (path: string | null) => unwrap(commands.prepareSend(path)),
    onSettled: () => queryClient.invalidateQueries({ queryKey: SEND_STATE_QUERY_KEY }),
  });
}

/** The go for the preflight `token` names, as a job. Resolves to the job's id. */
export function useWriteSend() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ token, confirmed }: { token: string; confirmed: boolean }) =>
      unwrap(commands.writeSend(token, confirmed)),
    onSettled: () => queryClient.invalidateQueries({ queryKey: SEND_STATE_QUERY_KEY }),
  });
}
