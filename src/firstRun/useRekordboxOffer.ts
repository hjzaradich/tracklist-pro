import { useMutation, useQuery, useQueryClient, type QueryClient } from "@tanstack/react-query";
import { useWhenJobsEnd } from "../activity/useWhenJobsEnd";
import { ALL_MUSIC_QUERY_KEY } from "../allMusic/useAllMusic";
import { unwrap } from "../api/errors";
import { commands, type AddSummary, type Offer, type PlaylistChoice } from "../bindings";
import { LIBRARY_TRACKS_QUERY_KEY } from "../library/useLibraryTracks";
import { REKORDBOX_OFFER_QUERY_KEY, REKORDBOX_PLAYLISTS_QUERY_KEY } from "./queryKeys";

// The offer to add rekordbox tracks to the Library (1aE-2, 1aE-3; ROADMAP
// 1.3). The app only ever asks how many could be added; they're added when
// the user says so, as one operation that one undo removes.

/** A rekordbox playlist: its folders from below ROOT, then its name. */
export type PlaylistPath = string[];

/** Everything the offer and what it changes are read from. */
function reloadAfterChange(queryClient: QueryClient) {
  return Promise.all([
    queryClient.invalidateQueries({ queryKey: REKORDBOX_OFFER_QUERY_KEY }),
    queryClient.invalidateQueries({ queryKey: LIBRARY_TRACKS_QUERY_KEY }),
    queryClient.invalidateQueries({ queryKey: ALL_MUSIC_QUERY_KEY }),
  ]);
}

/**
 * Asks for the offer and the playlists again whenever a background task
 * ends, since a rekordbox read, a relink or a scan can each change them.
 * Call it once, where the offer is shown.
 */
export function useReloadOfferWhenJobsEnd() {
  const queryClient = useQueryClient();
  useWhenJobsEnd(() => {
    void queryClient.invalidateQueries({ queryKey: REKORDBOX_OFFER_QUERY_KEY });
    void queryClient.invalidateQueries({ queryKey: REKORDBOX_PLAYLISTS_QUERY_KEY });
  });
}

/**
 * What the offer holds: for the whole collection (`null`), or for the
 * tracks in the chosen playlists.
 */
export function useRekordboxOffer(playlists: PlaylistPath[] | null) {
  return useQuery({
    queryKey: [...REKORDBOX_OFFER_QUERY_KEY, playlists],
    queryFn: (): Promise<Offer> => unwrap(commands.rekordboxOffer(playlists)),
  });
}

/** The rekordbox playlists that hold tracks, asked for only once `enabled`. */
export function useRekordboxPlaylists(enabled: boolean) {
  return useQuery({
    queryKey: REKORDBOX_PLAYLISTS_QUERY_KEY,
    queryFn: (): Promise<PlaylistChoice[]> => unwrap(commands.rekordboxPlaylists()),
    enabled,
  });
}

/** Adds the offered tracks (all, or those in the chosen playlists). */
export function useAddRekordboxTracks() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (playlists: PlaylistPath[] | null): Promise<AddSummary> =>
      unwrap(commands.addRekordboxTracks(playlists)),
    onSettled: () => reloadAfterChange(queryClient),
  });
}

/** Undoes an add, unless something else was done since. */
export function useUndoAddRekordboxTracks() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (operationId: number) => unwrap(commands.undoAddRekordboxTracks(operationId)),
    onSettled: () => reloadAfterChange(queryClient),
  });
}
