import { type QueryClient, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { commands, type Changed, type Crate, type LibraryTrack } from "../bindings";

/** Every crate query starts with this, so one invalidation reaches them all. */
export const CRATES_QUERY_KEY = ["crates"] as const;
const LIST_KEY = [...CRATES_QUERY_KEY, "list"] as const;
const tracksKey = (id: number) => [...CRATES_QUERY_KEY, "tracks", id] as const;

/**
 * Call after anything that can change what a crate holds: a crate change, or
 * a Library track going or coming back (its entries go and come with it).
 */
export function invalidateCrates(queryClient: QueryClient): Promise<void> {
  return queryClient.invalidateQueries({ queryKey: CRATES_QUERY_KEY });
}

/** Every crate with its track count. */
export function useCrates() {
  return useQuery({ queryKey: LIST_KEY, queryFn: (): Promise<Crate[]> => unwrap(commands.listCrates()) });
}

/** A crate's tracks, in the order they were added. */
export function useCrateTracks(id: number | null) {
  return useQuery({
    queryKey: tracksKey(id ?? -1),
    queryFn: (): Promise<LibraryTrack[]> => unwrap(commands.crateTracks(id ?? -1)),
    enabled: id !== null,
  });
}

/** Resolves to the new crate's id. */
export function useCreateCrate() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (name: string) => unwrap(commands.createCrate(name)),
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

export function useRemoveFromCrate() {
  const queryClient = useQueryClient();
  return useMutation<Changed, Error, { id: number; tracks: number[] }>({
    mutationFn: ({ id, tracks }) => unwrap(commands.removeTracksFromCrate(id, tracks)),
    onSettled: () => invalidateCrates(queryClient),
  });
}
