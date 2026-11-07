import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { commands } from "../bindings";

// The Diagnostics screen's reads and writes (ROADMAP 4.3). Nothing here leaves
// the machine: the log is read, the report is built and saved on this computer.

export const EVENTS_QUERY_KEY = ["diagnostics", "events"] as const;

/** The event log, newest first, and how many events the log file lost. */
export function useEventLog() {
  return useQuery({
    queryKey: EVENTS_QUERY_KEY,
    queryFn: () => commands.diagnosticEvents(),
  });
}

/** Builds the report. The text it returns is exactly what saving writes. */
export function usePreviewReport() {
  return useMutation({ mutationFn: () => unwrap(commands.previewDiagnosticsReport()) });
}

/** Saves the previewed report into the app's data folder. */
export function useSaveReport() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () => unwrap(commands.saveDiagnosticsReport()),
    onSettled: () => queryClient.invalidateQueries({ queryKey: EVENTS_QUERY_KEY }),
  });
}
