import { type QueryClient, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import {
  commands,
  type AcceptedAll,
  type AcceptItem,
  type Acted,
  type SwitchEffect,
  type Switched,
} from "../bindings";
import { ALL_MUSIC_QUERY_KEY } from "../allMusic/useAllMusic";
import { REKORDBOX_OFFER_QUERY_KEY } from "../firstRun/queryKeys";
import { LIBRARY_TRACKS_QUERY_KEY } from "../library/useLibraryTracks";
import { useAcceptedStore } from "./acceptedStore";
import { MISSING_TRACKS_QUERY_KEY } from "./useMissingTracks";
import { REVIEW_ACCEPT_ALL_QUERY_KEY, REVIEW_QUERY_KEYS } from "./useReview";

// What the user decides about a Review item (1bE-5, 1bE-6, 1bE-7). Each action is
// one undoable step on the Rust side; here it only calls the command and asks
// for the queue again, whether it worked or was refused (a refused action
// means the item changed under the screen, so the screen shows it again).

/**
 * The queue and the comparison, asked for again, with the lists an answer
 * can change: the Library (a switched file), All music and the rekordbox
 * offer (a match that is confirmed can be added), and Missing (an entry
 * whose match was refused has no file again).
 */
function reload(queryClient: QueryClient) {
  return Promise.all(
    [
      ...REVIEW_QUERY_KEYS,
      LIBRARY_TRACKS_QUERY_KEY,
      ALL_MUSIC_QUERY_KEY,
      REKORDBOX_OFFER_QUERY_KEY,
      MISSING_TRACKS_QUERY_KEY,
    ].map((queryKey) => queryClient.invalidateQueries({ queryKey })),
  );
}

/**
 * `onDone` is called when the action worked, before the queue is asked for
 * again (never after a refusal): the screen moves to the next item then
 * (1bE-2). It is a hook option, not a callback of one `mutate` call, because
 * the queue changes while the answer is awaited and the component that made
 * the call may be gone by then; a hook option still runs.
 */
function useAction<Args, Done>(run: (args: Args) => Promise<Done>, onDone?: (done: Done) => void) {
  const queryClient = useQueryClient();
  return useMutation<Done, Error, Args>({
    mutationFn: run,
    onSuccess: (done) => onDone?.(done),
    onSettled: () => reload(queryClient),
  });
}

/** "Dismiss" for a "Better file available" item: remembered for that one file. */
export function useDismissBetterFile(onDone?: () => void) {
  return useAction<{ recordingId: number; fileId: number }, Acted>(
    ({ recordingId, fileId }) => unwrap(commands.reviewDismissBetterFile(recordingId, fileId)),
    onDone,
  );
}

/** "Keep anyway" for a quality problem. */
export function useKeepAnyway(onDone?: () => void) {
  return useAction<number, Acted>(
    (recordingId) => unwrap(commands.reviewKeepAnyway(recordingId)),
    onDone,
  );
}

/** "Find a better file" for a quality problem: remembers the wish. */
export function useWantBetterFile(onDone?: () => void) {
  return useAction<number, Acted>(
    (recordingId) => unwrap(commands.reviewWantBetterFile(recordingId)),
    onDone,
  );
}

/**
 * Switches a Library track's linked file, which is gone, to a copy. `effect`
 * is what the item showed the switch means in rekordbox: the switch is made
 * only if it still means that.
 */
export function useSwitchLinkedFile(onDone?: () => void) {
  return useAction<{ recordingId: number; fileId: number; effect: SwitchEffect }, Switched>(
    ({ recordingId, fileId, effect }) =>
      unwrap(commands.reviewSwitchLinkedFile(recordingId, fileId, effect)),
    onDone,
  );
}

/**
 * The items "Accept all" would switch now. N on its button is how many, and
 * a click sends exactly these back: an item that joined the queue since the
 * button was drawn is never switched unseen.
 */
export function useAcceptAllItems(enabled: boolean) {
  return useQuery({
    queryKey: REVIEW_ACCEPT_ALL_QUERY_KEY,
    enabled,
    queryFn: (): Promise<AcceptItem[]> => unwrap(commands.reviewAcceptAllItems()),
  });
}

/**
 * "Accept all" for the items shown: every one that is still a clear switch, as
 * one step. What it switched is kept for the line at the top of Review
 * (`AcceptedLine`, 1bE-3), until it is dismissed or the next "Accept all".
 */
export function useAcceptAll() {
  const record = useAcceptedStore((state) => state.record);
  return useAction<AcceptItem[], AcceptedAll>(
    (items) => unwrap(commands.reviewAcceptClearBestFiles(items)),
    record,
  );
}

/** A probable relink match: the rekordbox entry and the file it is matched to. */
export interface RelinkMatch {
  entryId: number;
  fileId: number;
}

/** "Confirm" for a relink match: the entry is that file. */
export function useConfirmRelink(onDone?: () => void) {
  return useAction<RelinkMatch, Acted>(
    ({ entryId, fileId }) => unwrap(commands.reviewConfirmRelink(entryId, fileId)),
    onDone,
  );
}

/** "Not the same" for a relink match: the entry is not that file, and is never matched to it again. */
export function useRejectRelink(onDone?: () => void) {
  return useAction<RelinkMatch, Acted>(
    ({ entryId, fileId }) => unwrap(commands.reviewRejectRelink(entryId, fileId)),
    onDone,
  );
}
