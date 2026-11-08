import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { reloadLists } from "../app/lists";
import {
  commands,
  type IdentifyChoice,
  type IdentifyLookupStatus,
  type IdentifySuggestions,
  type LibraryTrackId,
} from "../bindings";
import { useOnlineServicesStatus } from "../settings/useOnlineServices";

// Identification (ROADMAP 2.2): what the app suggests for a Library track's
// details, from its own files and the online look-ups kept for them, and where
// the look-up stands. Nothing here changes a track except "Use".

/**
 * Everything identification asks for starts with this. Not in `app/lists.ts`
 * (only the Look up panel shows it), so a change invalidates it directly.
 */
export const IDENTIFY_QUERY_KEY = ["identify"] as const;

/** The suggestions for a Library track. Offline ones are there with every service off. */
export function useIdentifySuggestions(libraryTrackId: LibraryTrackId) {
  return useQuery({
    queryKey: [...IDENTIFY_QUERY_KEY, "suggestions", libraryTrackId],
    queryFn: (): Promise<IdentifySuggestions> => unwrap(commands.identifySuggestions(libraryTrackId)),
  });
}

/** Where the online look-up of a Library track stands. */
export function useLookupStatus(libraryTrackId: LibraryTrackId) {
  return useQuery({
    queryKey: [...IDENTIFY_QUERY_KEY, "status", libraryTrackId],
    // None when the backend has no status for the track: a query can't hold `undefined`.
    queryFn: async (): Promise<IdentifyLookupStatus | null> =>
      (await unwrap(commands.identifyLookupStatus([libraryTrackId])))[0] ?? null,
  });
}

/**
 * "Use": sets the chosen fields as the track's own values, as one operation
 * (one Undo). Answers the operation, none if nothing changed. The lists that
 * show a track's values are asked for again, as are the suggestions (a value
 * now in use isn't suggested again).
 */
export function useUseSuggestions() {
  const queryClient = useQueryClient();
  return useMutation<number | null, Error, { libraryTrackId: LibraryTrackId; choices: IdentifyChoice[] }>({
    mutationFn: ({ libraryTrackId, choices }) => unwrap(commands.identifyUse(libraryTrackId, choices)),
    onSuccess: () =>
      Promise.all([
        queryClient.invalidateQueries({ queryKey: IDENTIFY_QUERY_KEY }),
        reloadLists(queryClient, "library", "crates", "playlists", "tagSources", "overview"),
      ]),
  });
}

/**
 * Asks for the online look-up of Library tracks, in the background. Answers
 * the job, or none when no track has a linked file. The status is asked for
 * again at once (it now says queued); the end of the job asks again.
 */
export function useLookUpTracks() {
  const queryClient = useQueryClient();
  return useMutation<number | null, Error, LibraryTrackId[]>({
    mutationFn: (libraryTrackIds) => unwrap(commands.identifyLookUpTracks(libraryTrackIds)),
    onSettled: () => queryClient.invalidateQueries({ queryKey: IDENTIFY_QUERY_KEY }),
  });
}

/**
 * Whether an online look-up can run now: at least one of MusicBrainz,
 * AcoustID (on, with this build's key) and Discogs is switched on. `undefined`
 * until Settings' answer is in.
 */
export function useOnlineLookupAvailable(): boolean | undefined {
  const status = useOnlineServicesStatus().data;
  if (status === undefined) return undefined;
  return status.musicBrainz || (status.acoustId.on && !status.acoustId.keyMissing) || status.discogs.on;
}
