import { type QueryClient, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import {
  commands,
  type AcceptedAll,
  type AcceptItem,
  type Acted,
  type MergeChoice,
  type MergeField,
  type Merged,
  type NotSame,
  type ReviewItemId,
  type SwitchEffect,
  type Switched,
} from "../bindings";
import { AFTER_SEND_LISTS_QUERY_KEY } from "../afterSend/useAfterSendLists";
import { ALL_MUSIC_QUERY_KEY } from "../allMusic/useAllMusic";
import { CRATES_QUERY_KEY } from "../crates/useCrates";
import { REKORDBOX_OFFER_QUERY_KEY } from "../firstRun/queryKeys";
import { LIBRARY_TRACKS_QUERY_KEY } from "../library/useLibraryTracks";
import { useAcceptedStore } from "./acceptedStore";
import { TRACK_TAG_SOURCES_QUERY_KEY } from "../shell/useTrackTagSources";
import { MISSING_TRACKS_QUERY_KEY } from "./useMissingTracks";
import {
  REVIEW_ACCEPT_ALL_QUERY_KEY,
  REVIEW_MERGE_QUESTIONS_QUERY_KEY,
  REVIEW_QUERY_KEYS,
} from "./useReview";

// What the user decides about a Review item (1bE-5, 1bE-6, 1bE-7). Each action is
// one undoable step on the Rust side; here it only calls the command and asks
// for the queue again, whether it worked or was refused (a refused action
// means the item changed under the screen, so the screen shows it again).

/**
 * The queue and the comparison, asked for again, with the lists an answer
 * can change: the Library (a switched file), All music and the rekordbox
 * offer (a match that is confirmed can be added), Missing (an entry
 * whose match was refused has no file again), and the crates and the list of
 * tracks to remove in rekordbox (a merge of two Library tracks).
 */
function reload(queryClient: QueryClient) {
  return Promise.all(
    [
      ...REVIEW_QUERY_KEYS,
      LIBRARY_TRACKS_QUERY_KEY,
      ALL_MUSIC_QUERY_KEY,
      REKORDBOX_OFFER_QUERY_KEY,
      MISSING_TRACKS_QUERY_KEY,
      CRATES_QUERY_KEY,
      AFTER_SEND_LISTS_QUERY_KEY,
      TRACK_TAG_SOURCES_QUERY_KEY,
    ].map((queryKey) => queryClient.invalidateQueries({ queryKey })),
  );
}

/**
 * `onDone` is called when the action worked, before the queue is asked for
 * again (never after a refusal): the screen moves to the next item then
 * (1bE-2). It is a hook option, not a callback of one `mutate` call, because
 * the queue changes while the answer is awaited and the component that made
 * the call may be gone by then; a hook option still runs.
 *
 * A refusal shows at once (DOGFOOD B-5): the lists are asked for again, but
 * not awaited first. The mutation's error is set only once `onSettled` is
 * done, and reading every list again (the Missing list looks at the disk)
 * can take long enough that a refused action looked like nothing happened.
 * After an action that worked, the reload is awaited: the action stays under
 * way until the screen shows what it did.
 */
function useAction<Args, Done>(run: (args: Args) => Promise<Done>, onDone?: (done: Done) => void) {
  const queryClient = useQueryClient();
  return useMutation<Done, Error, Args>({
    mutationFn: run,
    onSuccess: (done) => onDone?.(done),
    onSettled: (_done, error) => {
      const reloaded = reload(queryClient);
      return error === null ? reloaded : undefined;
    },
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

/**
 * Withdraws the user's confirmation of a rekordbox entry (the Details panel,
 * 1cB-17): the entry is decided again from scratch. One undoable step, from
 * the top bar.
 */
export function useWithdrawRelink(onDone?: () => void) {
  return useAction<number, Acted>(
    (entryId) => unwrap(commands.reviewWithdrawRelink(entryId)),
    onDone,
  );
}

/**
 * "Keep both" for a "Same track twice in your Library" item (1bE-9).
 * `recordingIds` are the Library tracks the screen showed: the action is
 * refused if the item has other tracks by now.
 */
export function useKeepBoth(onDone?: () => void) {
  return useAction<{ anchor: number; recordingIds: number[] }, Acted>(
    ({ anchor, recordingIds }) => unwrap(commands.reviewKeepBoth(anchor, recordingIds)),
    onDone,
  );
}

/**
 * Merges a "Same track twice in your Library" item into the track `keep`
 * (1bE-9): the other Library tracks leave the Library. `choices` are the
 * values picked where the tracks differ (1cB-20). Undoable, choices and all.
 */
export function useMergeTracks(onDone?: () => void) {
  return useAction<
    { anchor: number; recordingIds: number[]; keep: number; choices: MergeChoice[] },
    Merged
  >(
    ({ anchor, recordingIds, keep, choices }) =>
      unwrap(commands.reviewMerge(anchor, recordingIds, keep, choices)),
    onDone,
  );
}

/**
 * The fields a merge into `keep` asks about (1cB-20): where the tracks hold
 * different values and one is the user's, with every track's value and the
 * default (the staying track's). Asked for again when the track that stays
 * changes.
 */
export function useMergeQuestions(
  anchor: number,
  recordingIds: number[],
  keep: number | null,
  enabled: boolean,
) {
  return useQuery({
    queryKey: [...REVIEW_MERGE_QUESTIONS_QUERY_KEY, anchor, recordingIds, keep],
    enabled: enabled && keep !== null,
    queryFn: (): Promise<MergeField[]> =>
      unwrap(commands.reviewMergeQuestions(anchor, recordingIds, keep ?? 0)),
  });
}

/**
 * "Same track" (Enter) for a possible version mix-up or "Unclear duplicates"
 * (1bE-2): the files are one track. `recordingIds` are the tracks the screen
 * showed. Undoable.
 */
export function useSameTrack(onDone?: () => void) {
  return useAction<{ id: ReviewItemId; recordingIds: number[] }, Acted>(
    ({ id, recordingIds }) => unwrap(commands.reviewSameTrack(id, recordingIds)),
    onDone,
  );
}

/**
 * S (1bE-2): "Not the same" for an item with two files (`fileId` null), or the
 * file `fileId` split off one with three or more. `onDone` hears whether the
 * item is still in the queue (a split can leave its other question open).
 * Undoable.
 */
export function useNotSame(onDone?: (done: NotSame) => void) {
  return useAction<{ id: ReviewItemId; recordingIds: number[]; fileId: number | null }, NotSame>(
    ({ id, recordingIds, fileId }) => unwrap(commands.reviewNotSame(id, recordingIds, fileId)),
    onDone,
  );
}
