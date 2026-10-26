import { useMutation, useQueryClient } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { commands } from "../bindings";
import { REKORDBOX_OFFER_QUERY_KEY } from "../firstRun/queryKeys";
import { LIBRARY_TRACKS_QUERY_KEY } from "../library/useLibraryTracks";

/** Invalidate this after anything adds or removes a Library track. */
export const ALL_MUSIC_QUERY_KEY = ["allMusic"] as const;

/**
 * All music's track table (1cA-3) keeps its windows of rows under this key,
 * which starts with {@link ALL_MUSIC_QUERY_KEY}, so whatever invalidates All
 * music fetches the windows on screen again.
 */
export const ALL_MUSIC_TABLE_QUERY_KEY = [...ALL_MUSIC_QUERY_KEY, "table"] as const;

/** What an "Add to Library" asks for: the track, and whether the user already confirmed the low-bitrate warning. */
export type AddRequest = { recordingId: number; confirmLowBitrate: boolean };

/**
 * "Add to Library": makes the track a linked Library track. Its file is never
 * written. The answer is `done`, or `needsConfirmation` with a warning (1bE-8):
 * then nothing was added, and asking again with `confirmLowBitrate` adds it.
 */
export function useAddToLibrary() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ recordingId, confirmLowBitrate }: AddRequest) =>
      unwrap(commands.promoteTrack(recordingId, confirmLowBitrate)),
    onSuccess: (outcome) =>
      // A warned add changed nothing, so there is nothing to reload.
      outcome.outcome !== "done"
        ? undefined
        : Promise.all([
            queryClient.invalidateQueries({ queryKey: ALL_MUSIC_QUERY_KEY }),
            queryClient.invalidateQueries({ queryKey: LIBRARY_TRACKS_QUERY_KEY }),
            // A track added by hand leaves the rekordbox offer.
            queryClient.invalidateQueries({ queryKey: REKORDBOX_OFFER_QUERY_KEY }),
          ]),
  });
}
