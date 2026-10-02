import { useMutation, useQueryClient } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { commands } from "../bindings";
import { invalidateCrates } from "../crates/useCrates";
import { LIBRARY_TRACKS_QUERY_KEY } from "./useLibraryTracks";

/** Removes a Library track (1aE-6). Its file is never touched. */
export function useRemoveLibraryTrack() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: number) => unwrap(commands.removeLibraryTrack(id)),
    // A removed track leaves its crates (and comes back to them on undo).
    onSettled: () =>
      Promise.all([
        queryClient.invalidateQueries({ queryKey: LIBRARY_TRACKS_QUERY_KEY }),
        invalidateCrates(queryClient),
      ]),
  });
}
