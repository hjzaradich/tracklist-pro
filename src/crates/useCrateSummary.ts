import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { commands, type CrateSummary } from "../bindings";
import { CRATES_QUERY_KEY, invalidateCrates } from "./useCrates";

/**
 * Every crate's summary query starts with this (under the crate queries, so
 * the lists' invalidations reach it: a crate change, an Undo, a job ending).
 * Anything that changes a track's values without touching a crate asks for
 * them again with this.
 */
export const CRATE_SUMMARY_QUERY_KEY = [...CRATES_QUERY_KEY, "summary"] as const;

/** A crate's note and what its tracks add up to (1cB-15). */
export function useCrateSummary(id: number) {
  return useQuery({
    queryKey: [...CRATE_SUMMARY_QUERY_KEY, id] as const,
    queryFn: (): Promise<CrateSummary> => unwrap(commands.crateSummary(id)),
  });
}

/**
 * Gives a crate a note, or none (an empty note). One operation, so one Undo
 * (in the top bar) puts the old note back. Resolves to the operation
 * recorded, or `null` when the note is the one it has.
 */
export function useSetCrateNotes() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ id, notes }: { id: number; notes: string }) => unwrap(commands.setCrateNotes(id, notes)),
    onSettled: () => invalidateCrates(queryClient),
  });
}
