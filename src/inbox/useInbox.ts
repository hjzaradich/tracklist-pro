import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { reloadLists } from "../app/lists";
import { unwrap } from "../api/errors";
import { commands, type InboxCount, type InboxItem, type InboxOutcome, type Label } from "../bindings";
import { MUSIC_FOLDERS_QUERY_KEY } from "../scan/useMusicFolders";

/** Everything the Inbox shows starts with this, so one invalidation reaches it all. */
export const INBOX_QUERY_KEY = ["inbox"] as const;
const COUNT_KEY = [...INBOX_QUERY_KEY, "count"] as const;
const ITEMS_KEY = [...INBOX_QUERY_KEY, "items"] as const;

/** How many items are new (the sidebar badge), by label, and the Inbox folders. */
export function useInboxCount() {
  return useQuery({ queryKey: COUNT_KEY, queryFn: (): Promise<InboxCount> => unwrap(commands.inboxCount()) });
}

/** The new items, all of them or only those with `label`. */
export function useInboxItems(label: Label | null) {
  return useQuery({
    queryKey: [...ITEMS_KEY, label],
    queryFn: (): Promise<InboxItem[]> => unwrap(commands.inboxItems({ status: "new", label })),
  });
}

/** What the screen can ask of the Inbox. Each is one operation, taken back by the top bar's Undo. */
export type InboxAction =
  | { kind: "add"; itemIds: number[]; confirmLowBitrate: boolean }
  | { kind: "addBatch"; batchId: number; confirmLowBitrate: boolean }
  | { kind: "notNow" | "hide" | "clear"; itemIds: number[] };

function run(action: InboxAction): Promise<InboxOutcome> {
  switch (action.kind) {
    case "add":
      return unwrap(commands.inboxAdd(action.itemIds, action.confirmLowBitrate));
    case "addBatch":
      return unwrap(commands.inboxAddBatch(action.batchId, action.confirmLowBitrate));
    case "notNow":
      return unwrap(commands.inboxNotNow(action.itemIds));
    case "hide":
      return unwrap(commands.inboxHide(action.itemIds));
    case "clear":
      return unwrap(commands.inboxClear(action.itemIds));
  }
}

/**
 * Runs an Inbox action, then asks again for the lists it can change: the
 * Inbox itself (and so the badge), the Library, and the Overview's numbers.
 */
export function useInboxAction() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: run,
    onSuccess: () => reloadLists(queryClient, "inbox", "library", "overview"),
  });
}

/** Adds a music folder with the role Inbox; the command scans it too. */
export function useAddInboxFolder() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (path: string) => unwrap(commands.addMusicFolder(path, "inbox")),
    onSettled: () =>
      Promise.all([
        queryClient.invalidateQueries({ queryKey: MUSIC_FOLDERS_QUERY_KEY }),
        queryClient.invalidateQueries({ queryKey: INBOX_QUERY_KEY }),
      ]),
  });
}

/** Removes a music folder (its files go from the lists; none is deleted from disk). */
export function useRemoveMusicFolder() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: number) => unwrap(commands.removeMusicFolder(id)),
    onSettled: () => reloadLists(queryClient, "folders", "inbox", "library"),
  });
}
