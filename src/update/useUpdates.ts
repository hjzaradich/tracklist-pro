import { useMutation, useQuery, useQueryClient, type QueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { unwrap } from "../api/errors";
import { commands, type UpdateStatus } from "../bindings";

// Update checks (ROADMAP 4.5): off until the user says yes; the app checks and
// tells, nothing more. The backend holds the answer and the once-a-day rule.

export const UPDATE_STATUS_QUERY_KEY = ["setting", "update_checks"] as const;

/** Where update checks stand. Cheap: it reads the stored answer and never contacts anything. */
export function useUpdateStatus() {
  return useQuery({
    queryKey: UPDATE_STATUS_QUERY_KEY,
    queryFn: () => unwrap(commands.updateStatus()),
  });
}

let startupCheckStarted = false;

/** Lets a test start over: the app checks once a run. */
export function resetStartupUpdateCheck() {
  startupCheckStarted = false;
}

/**
 * Asks the backend to check at the start of the app: it does so only when
 * update checks are on and the last answer is a day old. Once per run, from the
 * first screen that shows an update line. A failed check is not shown.
 */
export function useStartupUpdateCheck() {
  const queryClient = useQueryClient();
  useEffect(() => {
    if (startupCheckStarted) return;
    startupCheckStarted = true;
    void check(queryClient, false);
  }, [queryClient]);
}

async function check(queryClient: QueryClient, force: boolean) {
  const status = await unwrap(commands.checkForUpdates(force));
  queryClient.setQueryData(UPDATE_STATUS_QUERY_KEY, status);
  return status;
}

/** "Check now". */
export function useCheckForUpdatesNow() {
  const queryClient = useQueryClient();
  return useMutation({ mutationFn: () => check(queryClient, true) });
}

/** Answers the first-run question, or flips the Settings switch. A "yes" checks at once. */
export function useSetUpdateChecks() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (on: boolean) => {
      await unwrap(commands.setUpdateChecks(on));
      if (on) {
        try {
          await check(queryClient, true);
        } catch {
          // The answer is saved; a failed check is not shown.
        }
      }
    },
    onSettled: () => queryClient.invalidateQueries({ queryKey: UPDATE_STATUS_QUERY_KEY }),
  });
}
