import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { unwrap } from "../api/errors";
import { commands, events, type MusicFolder } from "../bindings";

export const MUSIC_FOLDERS_QUERY_KEY = ["musicFolders"] as const;

function fetchMusicFolders(): Promise<MusicFolder[]> {
  return unwrap(commands.musicFolders());
}

/**
 * Every music folder, with whether its drive is connected and what the
 * last scan found. Reloaded when a scan finishes, so the counts are
 * current, and when drives come or go, so a folder whose drive was
 * unplugged shows greyed out at once.
 */
export function useMusicFolders() {
  const queryClient = useQueryClient();
  useEffect(() => {
    let stopped = false;
    const unlisteners: (() => void)[] = [];
    const reload = () => void queryClient.invalidateQueries({ queryKey: MUSIC_FOLDERS_QUERY_KEY });
    const keep = (listening: Promise<() => void>) =>
      listening
        .then((stop) => {
          if (stopped) stop();
          else unlisteners.push(stop);
        })
        // Outside the app (e.g. a plain browser) no events ever arrive.
        .catch(() => {});

    void keep(
      events.jobUpdates.listen((event) => {
        const scanFinished = event.payload.some(
          (job) => job.kind === "scan" && job.status !== "queued" && job.status !== "running",
        );
        if (scanFinished) reload();
      }),
    );
    void keep(events.volumesChanged.listen(reload));

    return () => {
      stopped = true;
      for (const stop of unlisteners) stop();
    };
  }, [queryClient]);

  return useQuery({ queryKey: MUSIC_FOLDERS_QUERY_KEY, queryFn: fetchMusicFolders });
}
