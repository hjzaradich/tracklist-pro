import { useMutation, useQueryClient } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { commands, type MusicFolder, type MusicFolderId } from "../bindings";
import { MUSIC_FOLDERS_QUERY_KEY } from "./useMusicFolders";

// A music folder's watcher (1aC-6): on, the folder is rescanned in the
// background whenever files change under it, and when its drive comes
// back. The flag lives on the backend's `music_folder` row.

/** Turns a music folder's watcher on or off. */
export function useSetMusicFolderWatch() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async ({ id, watch }: { id: MusicFolderId; watch: boolean }) => {
      await unwrap(commands.setMusicFolderWatch(id, watch));
      return { id, watch };
    },
    onSuccess: ({ id, watch }) => {
      queryClient.setQueryData<MusicFolder[]>(MUSIC_FOLDERS_QUERY_KEY, (folders) =>
        folders?.map((folder) => (folder.id === id ? { ...folder, watch } : folder)),
      );
    },
  });
}
