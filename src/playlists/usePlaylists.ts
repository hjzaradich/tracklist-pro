import { type QueryClient, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import {
  commands,
  type AddedToPlaylist,
  type CopiedFromRekordbox,
  type Playlist,
  type PlaylistChanged,
  type RekordboxPlaylist,
} from "../bindings";
import { reloadLists } from "../app/lists";
import { invalidateCrates } from "../crates/useCrates";
import type { RowsQuery } from "../trackTable/useTrackRows";

/** Every playlist query starts with this, so one invalidation reaches them all. */
export const PLAYLISTS_QUERY_KEY = ["playlists"] as const;
const LIST_KEY = [...PLAYLISTS_QUERY_KEY, "list"] as const;

/** Call after anything that can change what a playlist holds or is called. */
export function invalidatePlaylists(queryClient: QueryClient): Promise<void> {
  return queryClient.invalidateQueries({ queryKey: PLAYLISTS_QUERY_KEY });
}

/** Every playlist, oldest first, with its track count. */
export function usePlaylists() {
  return useQuery({ queryKey: LIST_KEY, queryFn: (): Promise<Playlist[]> => unwrap(commands.listPlaylists()) });
}

/** Resolves to the new playlist's id. */
export function useCreatePlaylist() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (name: string) => unwrap(commands.createPlaylist(name)),
    onSettled: () => invalidatePlaylists(queryClient),
  });
}

/** Resolves to the operation recorded, or `null` when the name is the one it has. */
export function useRenamePlaylist() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ id, name }: { id: number; name: string }) => unwrap(commands.renamePlaylist(id, name)),
    onSettled: () => invalidatePlaylists(queryClient),
  });
}

export function useDeletePlaylist() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: number) => unwrap(commands.deletePlaylist(id)),
    onSettled: () => invalidatePlaylists(queryClient),
  });
}

/**
 * Adds tracks (`TableRow.recordingId`) to the end of a playlist, one
 * operation, as an add to a crate does: a track outside the Library joins it
 * too, and the ones "Add to Library" refuses come back as left out.
 */
export function useAddRecordingsToPlaylist() {
  const queryClient = useQueryClient();
  return useMutation<AddedToPlaylist, Error, { id: number; recordings: readonly number[] }>({
    mutationFn: ({ id, recordings }) => unwrap(commands.addRecordingsToPlaylist(id, [...recordings], null)),
    // It can put tracks in the Library as well: every list of them, the offer and the overview show it.
    onSettled: () =>
      Promise.all([invalidatePlaylists(queryClient), reloadLists(queryClient, "library", "offer", "overview")]),
  });
}

/** Takes entries (`TableRow.entryId`) out of a playlist, one operation. */
export function useRemoveFromPlaylist() {
  const queryClient = useQueryClient();
  return useMutation<PlaylistChanged, Error, { id: number; entries: number[] }>({
    mutationFn: ({ id, entries }) => unwrap(commands.removeFromPlaylist(id, entries)),
    onSettled: () => invalidatePlaylists(queryClient),
  });
}

/** Moves entries before another entry (`null`: to the end), one operation. */
export function useMoveInPlaylist() {
  const queryClient = useQueryClient();
  return useMutation<PlaylistChanged, Error, { id: number; entries: number[]; before: number | null }>({
    mutationFn: ({ id, entries, before }) => unwrap(commands.moveInPlaylist(id, entries, before)),
    onSettled: () => invalidatePlaylists(queryClient),
  });
}

/** The most rows one table request can ask for (the backend's `MAX_WINDOW`). */
const PAGE = 500;

/** The playlist entries of a table's rows, in the order the table shows them (a track in twice gives both). */
export async function shownEntries(query: RowsQuery): Promise<number[]> {
  const order: number[] = [];
  for (let offset = 0; ; offset += PAGE) {
    const page = await unwrap(commands.trackTablePage({ ...query, offset, limit: PAGE, withTotal: false }));
    for (const row of page.rows) if (row.entryId !== null) order.push(row.entryId);
    if (page.rows.length < PAGE) return order;
  }
}

/**
 * "Keep this order": the playlist in the order its table shows now (sorted by
 * fit, say), one operation, so one Undo. `view` is what the table asks for.
 */
export function useKeepPlaylistOrder() {
  const queryClient = useQueryClient();
  return useMutation<PlaylistChanged, Error, { id: number; view: RowsQuery }>({
    mutationFn: async ({ id, view }) => unwrap(commands.setPlaylistOrder(id, await shownEntries(view))),
    onSettled: () => invalidatePlaylists(queryClient),
  });
}

/** "Save as crate": resolves to the new crate's id. */
export function useSavePlaylistAsCrate() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ id, name }: { id: number; name: string }) => unwrap(commands.savePlaylistAsCrate(id, name)),
    onSettled: () => invalidateCrates(queryClient),
  });
}

/** The Library tracks of a table's rows, in the order the table shows them. */
export async function libraryOrder(query: RowsQuery): Promise<number[]> {
  const order: number[] = [];
  for (let offset = 0; ; offset += PAGE) {
    const page = await unwrap(commands.trackTablePage({ ...query, offset, limit: PAGE, withTotal: false }));
    for (const row of page.rows) if (row.libraryTrackId !== null) order.push(row.libraryTrackId);
    if (page.rows.length < PAGE) return order;
  }
}

/**
 * "Start a playlist from this crate", in the order of the crate's table as it
 * is now (`view`: its sort and filters; without one, the crate's own order).
 * Resolves to the new playlist's id.
 */
export function usePlaylistFromCrate() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async ({ crateId, name, view }: { crateId: number; name: string; view: RowsQuery | null }) =>
      unwrap(commands.playlistFromCrate(crateId, name, view === null ? [] : await libraryOrder(view))),
    onSettled: () => invalidatePlaylists(queryClient),
  });
}

/** The playlists of the newest rekordbox read, for the picker; asked only while it is open. */
export function useRekordboxPlaylistsToCopy(enabled: boolean) {
  return useQuery({
    queryKey: [...PLAYLISTS_QUERY_KEY, "rekordbox"] as const,
    queryFn: (): Promise<RekordboxPlaylist[]> => unwrap(commands.rekordboxPlaylistsToCopy()),
    enabled,
    gcTime: 0,
  });
}

/** "Make an app playlist from this rekordbox playlist". */
export function usePlaylistFromRekordbox() {
  const queryClient = useQueryClient();
  return useMutation<CopiedFromRekordbox, Error, { path: string[]; name: string }>({
    mutationFn: ({ path, name }) => unwrap(commands.playlistFromRekordbox(path, name)),
    onSettled: () => invalidatePlaylists(queryClient),
  });
}
