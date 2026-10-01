import { useMutation, useQueryClient } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { commands, type UndoOutcome } from "../bindings";
import { LIBRARY_TRACKS_QUERY_KEY } from "./useLibraryTracks";

/** Removes a Library track (1aE-6). Its file is never touched. */
export function useRemoveLibraryTrack() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: number) => unwrap(commands.removeLibraryTrack(id)),
    onSettled: () => queryClient.invalidateQueries({ queryKey: LIBRARY_TRACKS_QUERY_KEY }),
  });
}

/** Undoes the last operation, e.g. a removal. Resolves to what undo did. */
export function useUndoLast() {
  const queryClient = useQueryClient();
  return useMutation<UndoOutcome>({
    mutationFn: () => unwrap(commands.undoLastOperation()),
    onSettled: () => queryClient.invalidateQueries({ queryKey: LIBRARY_TRACKS_QUERY_KEY }),
  });
}
