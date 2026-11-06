import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import {
  commands,
  type AliasKind,
  type GenreTree,
  type Spelling,
  type UnmappedGenres,
} from "../bindings";

/** Every genre query starts with this, so one invalidation reaches them all. */
export const GENRE_QUERY_KEY = ["genre"] as const;

/** The genre tree, its aliases and the Library tracks per genre (3.2). */
export function useGenreTree() {
  return useQuery({
    queryKey: [...GENRE_QUERY_KEY, "tree"] as const,
    queryFn: (): Promise<GenreTree> => unwrap(commands.genreTree()),
  });
}

/** Every genre spelling the Library uses, with where it goes (or that it is unmapped). */
export function useSpellings() {
  return useQuery({
    queryKey: [...GENRE_QUERY_KEY, "spellings"] as const,
    queryFn: (): Promise<Spelling[]> => unwrap(commands.genreSpellings()),
  });
}

/** Review's group: tracks with no genre, then the spellings no rule maps. */
export function useUnmappedGenres() {
  return useQuery({
    queryKey: [...GENRE_QUERY_KEY, "unmapped"] as const,
    queryFn: (): Promise<UnmappedGenres> => unwrap(commands.genreUnmapped()),
  });
}

/**
 * One mutation per edit. Each is an undoable operation; afterwards every
 * genre list is asked for again (the counts follow once the index has run).
 */
export function useGenreEdits() {
  const queryClient = useQueryClient();
  const settled = () =>
    queryClient.invalidateQueries({ queryKey: GENRE_QUERY_KEY });
  return {
    add: useMutation({
      mutationFn: ({ name, parent }: { name: string; parent: number | null }) =>
        unwrap(commands.genreAdd(name, parent)),
      onSettled: settled,
    }),
    rename: useMutation({
      mutationFn: ({ id, name }: { id: number; name: string }) =>
        unwrap(commands.genreRename(id, name)),
      onSettled: settled,
    }),
    move: useMutation({
      mutationFn: ({ id, parent }: { id: number; parent: number | null }) =>
        unwrap(commands.genreMove(id, parent)),
      onSettled: settled,
    }),
    addAlias: useMutation({
      mutationFn: ({
        genreId,
        alias,
        kind,
      }: {
        genreId: number;
        alias: string;
        kind: AliasKind;
      }) => unwrap(commands.genreAddAlias(genreId, alias, kind)),
      onSettled: settled,
    }),
    removeAlias: useMutation({
      mutationFn: (id: number) => unwrap(commands.genreRemoveAlias(id)),
      onSettled: settled,
    }),
    loadSeed: useMutation({
      mutationFn: () => unwrap(commands.genreLoadSeed()),
      onSettled: settled,
    }),
  };
}
