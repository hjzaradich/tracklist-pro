import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { commands } from "../bindings";

// The opt-in to reading OneDrive online-only files (1aB-8), kept in the
// backend's `setting` table. Off by default: reading one downloads it.

export const READ_ONLINE_ONLY_QUERY_KEY = ["setting", "read_online_only_files"] as const;

/** Whether the app may read online-only files. Off while it loads or if it can't be read. */
export function useReadOnlineOnlyFiles(): boolean {
  const { data } = useQuery({
    queryKey: READ_ONLINE_ONLY_QUERY_KEY,
    queryFn: () => unwrap(commands.readOnlineOnlyFiles()),
  });
  return data === true;
}

/** Turns the opt-in on or off. */
export function useSetReadOnlineOnlyFiles() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (on: boolean) => {
      await unwrap(commands.setReadOnlineOnlyFiles(on));
      return on;
    },
    onSuccess: (on) => queryClient.setQueryData(READ_ONLINE_ONLY_QUERY_KEY, on),
  });
}
