import { useMutation, useQueryClient } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { commands } from "../bindings";
import { MUSIC_FOLDERS_QUERY_KEY } from "../scan/useMusicFolders";

/**
 * Adds a music folder. The command scans it too (as a job, shown in
 * Activity). The folder is only read.
 */
export function useAddMusicFolder() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (path: string) => unwrap(commands.addMusicFolder(path, null)),
    onSettled: () => queryClient.invalidateQueries({ queryKey: MUSIC_FOLDERS_QUERY_KEY }),
  });
}

/** "Scan again": scans one music folder now (as a job, shown in Activity). */
export function useScanMusicFolder() {
  return useMutation({
    mutationFn: (id: number) => unwrap(commands.scanMusicFolders([id])),
  });
}
