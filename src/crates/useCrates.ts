import { type QueryClient, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import {
  commands,
  type AddedToCrate,
  type AddedToNewCrate,
  type Changed,
  type Crate,
  type FolderContents,
} from "../bindings";
import { reloadLists } from "../app/lists";
import { ALL_MUSIC_TABLE_QUERY_KEY } from "../allMusic/useAllMusic";
import { LIBRARY_TABLE_QUERY_KEY } from "../library/useLibrarySummary";

/** Every crate query starts with this, so one invalidation reaches them all. */
export const CRATES_QUERY_KEY = ["crates"] as const;
/** The tree of folders and crates with their track counts. */
export const CRATES_LIST_QUERY_KEY = [...CRATES_QUERY_KEY, "list"] as const;
const LIST_KEY = CRATES_LIST_QUERY_KEY;

/**
 * Call after anything that can change what a crate holds: a crate change, or
 * a Library track going or coming back (its entries go and come with it).
 */
export function invalidateCrates(queryClient: QueryClient): Promise<void> {
  // The Library's and All music's tables can be filtered to the tracks that
  // are, or aren't, in a crate.
  void queryClient.invalidateQueries({ queryKey: LIBRARY_TABLE_QUERY_KEY });
  void queryClient.invalidateQueries({ queryKey: ALL_MUSIC_TABLE_QUERY_KEY });
  return queryClient.invalidateQueries({ queryKey: CRATES_QUERY_KEY });
}

/** Every folder and crate with its track count. */
export function useCrates() {
  return useQuery({ queryKey: LIST_KEY, queryFn: (): Promise<Crate[]> => unwrap(commands.listCrates()) });
}

/** What deleting a folder would delete; asked only while `id` is a folder. */
export function useFolderContents(id: number | null) {
  return useQuery({
    queryKey: [...CRATES_QUERY_KEY, "contents", id ?? -1] as const,
    queryFn: (): Promise<FolderContents> => unwrap(commands.crateFolderContents(id ?? -1)),
    enabled: id !== null,
    // Never shown from an earlier look: a count the confirmation shows is
    // the count right now.
    gcTime: 0,
  });
}

/** Where a new crate or folder goes: a folder's id, or `null` for the top. */
export interface NewIn {
  name: string;
  parent: number | null;
}

/** Resolves to the new crate's id. */
export function useCreateCrate() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ name, parent }: NewIn) => unwrap(commands.createCrate(name, parent)),
    onSettled: () => invalidateCrates(queryClient),
  });
}

/** Resolves to the new folder's id. */
export function useCreateFolder() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ name, parent }: NewIn) => unwrap(commands.createCrateFolder(name, parent)),
    onSettled: () => invalidateCrates(queryClient),
  });
}

/** Resolves to the operation recorded, or `null` when it was there already. */
export function useMoveCrate() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ id, folder }: { id: number; folder: number | null }) =>
      unwrap(commands.moveCrate(id, folder)),
    onSettled: () => invalidateCrates(queryClient),
  });
}

export function useRenameCrate() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ id, name }: { id: number; name: string }) =>
      unwrap(commands.renameCrate(id, name)),
    onSettled: () => invalidateCrates(queryClient),
  });
}

export function useDeleteCrate() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: number) => unwrap(commands.deleteCrate(id)),
    onSettled: () => invalidateCrates(queryClient),
  });
}

/** Resolves to how many tracks were added and how many were already there. */
export function useAddToCrate() {
  const queryClient = useQueryClient();
  return useMutation<Changed, Error, { id: number; tracks: number[] }>({
    mutationFn: ({ id, tracks }) => unwrap(commands.addTracksToCrate(id, tracks)),
    onSettled: () => invalidateCrates(queryClient),
  });
}

/**
 * Call after an add by track id: it can put tracks in the Library as well
 * (1cB-23), which every list of Library tracks, All music, the rekordbox
 * offer and the overview's counts show.
 */
function invalidateCratesAndLibrary(queryClient: QueryClient): Promise<unknown> {
  return Promise.all([
    invalidateCrates(queryClient),
    reloadLists(queryClient, "library", "offer", "overview"),
  ]);
}

/**
 * Adds tracks to a crate by their track id (a track table's selection,
 * 1cB-13). One operation for all of them. A track outside the Library joins
 * it too (1cB-23). Resolves to how many joined, how many came into the
 * Library with them, how many were in the crate already and how many were
 * left out, by reason.
 */
export function useAddRecordingsToCrate() {
  const queryClient = useQueryClient();
  return useMutation<AddedToCrate, Error, { id: number; recordings: readonly number[] }>({
    mutationFn: ({ id, recordings }) => unwrap(commands.addRecordingsToCrate(id, [...recordings])),
    onSettled: () => invalidateCratesAndLibrary(queryClient),
  });
}

/**
 * Makes a crate at the top of the tree and adds tracks to it, as one
 * operation (one Undo): a refused name or a failed add leaves no crate.
 */
export function useAddRecordingsToNewCrate() {
  const queryClient = useQueryClient();
  return useMutation<AddedToNewCrate, Error, { name: string; recordings: readonly number[] }>({
    mutationFn: ({ name, recordings }) => unwrap(commands.addRecordingsToNewCrate(name, [...recordings])),
    onSettled: () => invalidateCratesAndLibrary(queryClient),
  });
}

/** The crates (not folders) a track is in, by id; asked only for a Library track. */
export function useCratesOfTrack(recordingId: number, enabled: boolean) {
  return useQuery({
    queryKey: [...CRATES_QUERY_KEY, "of", recordingId] as const,
    queryFn: (): Promise<number[]> => unwrap(commands.cratesOfTrack(recordingId)),
    enabled,
  });
}

export function useRemoveFromCrate() {
  const queryClient = useQueryClient();
  return useMutation<Changed, Error, { id: number; tracks: number[] }>({
    mutationFn: ({ id, tracks }) => unwrap(commands.removeTracksFromCrate(id, tracks)),
    onSettled: () => invalidateCrates(queryClient),
  });
}
