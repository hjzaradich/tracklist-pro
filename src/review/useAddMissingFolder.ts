import { useMutation, useQueryClient } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { commands } from "../bindings";
import { MUSIC_FOLDERS_QUERY_KEY } from "../scan/useMusicFolders";
import { MISSING_TRACKS_QUERY_KEY } from "./useMissingTracks";

/**
 * Adds a Missing group's folder as a music folder, with the existing
 * add-folder command. The scan and relink that follow run as usual, and the
 * list is reloaded once the folder is in. `isPending` is true while an add
 * runs, and `error` holds why the last one failed.
 */
export function useAddMissingFolder() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (folder: string) => unwrap(commands.addMusicFolder(folder, null)),
    onSuccess: () =>
      Promise.all([
        queryClient.invalidateQueries({ queryKey: MUSIC_FOLDERS_QUERY_KEY }),
        queryClient.invalidateQueries({ queryKey: MISSING_TRACKS_QUERY_KEY }),
      ]),
  });
}
