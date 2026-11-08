import { useMutation, useQueryClient } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { reloadLists } from "../app/lists";
import { commands, type LibraryTrackId } from "../bindings";

/**
 * Sets a Library track's genre to a genre's name, as the user's own edit
 * (ROADMAP 3.8: "Use" on a genre guess; 5B B1): one operation, so one Undo,
 * in the top bar, puts back what was there. The next send carries it. The
 * track's row shows it in every table and the Details panel, the Overview
 * and the genre doors count it, and a name the genre tree doesn't know is in
 * Review's Unmapped genres, so those are asked for again. Takes any genre
 * name, so "Your genres" guesses can use it too.
 */
export function useSetGenre() {
  const queryClient = useQueryClient();
  return useMutation<number | null, Error, { libraryTrackId: LibraryTrackId; name: string }>({
    mutationFn: ({ libraryTrackId, name }) => unwrap(commands.setGenre(libraryTrackId, name)),
    onSuccess: () =>
      reloadLists(queryClient, "library", "crates", "playlists", "overview", "tagSources", "genre"),
  });
}
