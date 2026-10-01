import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { unwrap } from "../api/errors";
import { commands, type AfterSendLists, type SendState, type XmlSource } from "../bindings";
import { XML_SOURCE_QUERY_KEY } from "../rekordbox/useXmlSource";
import { SEND_STATE_QUERY_KEY } from "../send/useSend";

export const AFTER_SEND_LISTS_QUERY_KEY = ["afterSendLists"] as const;

function fetchAfterSendLists(): Promise<AfterSendLists> {
  return unwrap(commands.afterSendLists());
}

/**
 * What the lists depend on, as the screen knows it: the send's revision (it
 * moves when a step ends: a read done, the file written) and when the
 * rekordbox export was last read.
 */
function marker(queryClient: ReturnType<typeof useQueryClient>): string {
  const send = queryClient.getQueryData<SendState>(SEND_STATE_QUERY_KEY);
  const source = queryClient.getQueryData<XmlSource>(XML_SOURCE_QUERY_KEY);
  return `${send?.revision ?? ""}|${source?.lastRead?.readAt ?? ""}`;
}

/**
 * The playlists and tracks rekordbox still holds that an import can't remove
 * (1aF-2). They're asked again on every mount, and whenever the send changes
 * under the screen (a read finished, the file written), so they never show
 * the state from before.
 */
export function useAfterSendLists() {
  const queryClient = useQueryClient();
  useEffect(() => {
    const cache = queryClient.getQueryCache();
    let last = marker(queryClient);
    return cache.subscribe(() => {
      const now = marker(queryClient);
      if (now !== last) {
        last = now;
        void queryClient.invalidateQueries({ queryKey: AFTER_SEND_LISTS_QUERY_KEY });
      }
    });
  }, [queryClient]);
  return useQuery({
    queryKey: AFTER_SEND_LISTS_QUERY_KEY,
    queryFn: fetchAfterSendLists,
    refetchOnMount: "always",
  });
}
