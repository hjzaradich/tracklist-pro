import { errorMessage } from "../api/errors";
import { useAddReport } from "../crates/addSelection";
import { useAddRecordingsToPlaylist } from "./usePlaylists";

/**
 * Adds tracks (`TableRow.recordingId`) to the end of a playlist as one
 * operation, and puts what happened in the crates' {@link useAddReport}, so
 * the line over the track table says it whatever added them (the sidebar's
 * drop, the C key's dialog). Resolves to whether it worked.
 */
export function useAddSelectionToPlaylist() {
  const add = useAddRecordingsToPlaylist();
  return async (playlist: { id: number; name: string }, recordings: readonly number[]): Promise<boolean> => {
    try {
      const done = await add.mutateAsync({ id: playlist.id, recordings });
      useAddReport.getState().show({
        kind: "addedPlaylist",
        playlist: playlist.name,
        added: done.changed,
        notInLibrary: done.skipped,
      });
      return true;
    } catch (error) {
      useAddReport.getState().show({ kind: "failed", message: errorMessage(error) });
      return false;
    }
  };
}
