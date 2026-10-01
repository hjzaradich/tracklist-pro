import { useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import { unwrap } from "../api/errors";
import { commands } from "../bindings";
import { MUSIC_FOLDERS_QUERY_KEY } from "../scan/useMusicFolders";
import { MISSING_TRACKS_QUERY_KEY } from "./useMissingTracks";

/**
 * Adds a Missing group's folder as a music folder, with the existing
 * add-folder command. The scan and relink that follow run as usual, and the
 * list is reloaded once the folder is in.
 */
export function useAddMissingFolder() {
  const queryClient = useQueryClient();
  return useCallback(
    (folder: string) => {
      void unwrap(commands.addMusicFolder(folder, null))
        .then(() =>
          Promise.all([
            queryClient.invalidateQueries({ queryKey: MUSIC_FOLDERS_QUERY_KEY }),
            queryClient.invalidateQueries({ queryKey: MISSING_TRACKS_QUERY_KEY }),
          ]),
        )
        .catch(() => {
          // The folder stays on offer; a failed add changes nothing.
        });
    },
    [queryClient],
  );
}
