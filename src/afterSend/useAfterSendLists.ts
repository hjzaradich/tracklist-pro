import { useQuery } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { commands, type AfterSendLists } from "../bindings";

export const AFTER_SEND_LISTS_QUERY_KEY = ["afterSendLists"] as const;

function fetchAfterSendLists(): Promise<AfterSendLists> {
  return unwrap(commands.afterSendLists());
}

/**
 * The playlists and tracks rekordbox still holds that an import can't remove
 * (1aF-2). `refetchOnMount: "always"` so a new send's list is never the
 * previous one's.
 */
export function useAfterSendLists() {
  return useQuery({
    queryKey: AFTER_SEND_LISTS_QUERY_KEY,
    queryFn: fetchAfterSendLists,
    refetchOnMount: "always",
  });
}
