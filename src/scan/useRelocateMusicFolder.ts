import { useMutation, useQueryClient } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { commands, type MusicFolderId } from "../bindings";
import { MUSIC_FOLDERS_QUERY_KEY } from "./useMusicFolders";

/**
 * "Find it…": points a missing music folder at where it is now. The command
 * checks that the music is there, moves the folder (one Undo takes it back)
 * and scans it again. The result says how many of the files it checked were
 * found. Nothing in the music itself is touched.
 */
export function useRelocateMusicFolder() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (args: { id: MusicFolderId; path: string }) =>
      unwrap(commands.relocateMusicFolder(args.id, args.path)),
    onSettled: () => queryClient.invalidateQueries({ queryKey: MUSIC_FOLDERS_QUERY_KEY }),
  });
}
