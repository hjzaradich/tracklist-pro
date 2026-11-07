import { useQuery } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { commands, type MusicFolder } from "../bindings";

/**
 * The ids of the music folders that sit in OneDrive. Asked again when the
 * set of folders changes (the key holds their ids). A failed check shows
 * nothing: the note is a courtesy, not a gate.
 */
export function useOnedriveFolders(folders: MusicFolder[] | undefined): ReadonlySet<number> {
  const ids = (folders ?? []).map((folder) => folder.id);
  const { data } = useQuery({
    queryKey: ["onedriveFolders", ids],
    queryFn: () => unwrap(commands.onedriveMusicFolders()),
    enabled: ids.length > 0,
  });
  return new Set(data ?? []);
}
