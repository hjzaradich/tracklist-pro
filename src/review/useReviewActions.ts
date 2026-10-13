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
import { LIBRARY_TRACKS_QUERY_KEY } from "../library/useLibraryTracks";
import { REVIEW_ACCEPT_ALL_QUERY_KEY, REVIEW_QUERY_KEYS } from "./useReview";

// What the user decides about a Review item (1bE-5, 1bE-7). Each action is
// one undoable step on the Rust side; here it only calls the command and asks
// for the queue again, whether it worked or was refused (a refused action
// means the item changed under the screen, so the screen shows it again).

/** The queue, the comparison and the Library list, asked for again. */
function reload(queryClient: QueryClient) {
  return Promise.all(
    [...REVIEW_QUERY_KEYS, LIBRARY_TRACKS_QUERY_KEY].map((queryKey) =>
      queryClient.invalidateQueries({ queryKey }),
    ),
  );
}

function useAction<Args, Done>(run: (args: Args) => Promise<Done>) {
  const queryClient = useQueryClient();
  return useMutation<Done, Error, Args>({
    mutationFn: run,
    onSettled: () => reload(queryClient),
  });
}

/** "Dismiss" for a "Better file available" item: remembered for that one file. */
export function useDismissBetterFile() {
  return useAction<{ recordingId: number; fileId: number }, Acted>(({ recordingId, fileId }) =>
    unwrap(commands.reviewDismissBetterFile(recordingId, fileId)),
  );
}

/** "Keep anyway" for a quality problem. */
export function useKeepAnyway() {
  return useAction<number, Acted>((recordingId) => unwrap(commands.reviewKeepAnyway(recordingId)));
}

/** "Find a better file" for a quality problem: remembers the wish. */
export function useWantBetterFile() {
  return useAction<number, Acted>((recordingId) =>
    unwrap(commands.reviewWantBetterFile(recordingId)),
  );
}

/**
 * Switches a Library track's linked file, which is gone, to a copy. `effect`
 * is what the item showed the switch means in rekordbox: the switch is made
 * only if it still means that.
 */
export function useSwitchLinkedFile() {
  return useAction<{ recordingId: number; fileId: number; effect: SwitchEffect }, Switched>(
    ({ recordingId, fileId, effect }) =>
      unwrap(commands.reviewSwitchLinkedFile(recordingId, fileId, effect)),
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

/** "Accept all" for the items shown: every one that is still a clear switch, as one step. */
export function useAcceptAll() {
  return useAction<AcceptItem[], AcceptedAll>((items) =>
    unwrap(commands.reviewAcceptClearBestFiles(items)),
  );
}
