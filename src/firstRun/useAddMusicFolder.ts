import { useMutation, useQueryClient } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { commands } from "../bindings";
import { MUSIC_FOLDERS_QUERY_KEY } from "../scan/useMusicFolders";

/**
 * Adds a music folder with the existing command, then scans it (as a job,
 * shown in Activity). The folder is only read.
 */
export function useAddMusicFolder() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (path: string) => {
      const folder = await unwrap(commands.addMusicFolder(path, null));
      await unwrap(commands.scanMusicFolders([folder.id]));
      return folder;
    },
    onSettled: () => queryClient.invalidateQueries({ queryKey: MUSIC_FOLDERS_QUERY_KEY }),
  });
}
