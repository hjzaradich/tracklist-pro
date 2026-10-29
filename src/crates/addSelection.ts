import { create } from "zustand";
import { errorMessage } from "../api/errors";
import type { AddedToCrate } from "../bindings";
import { useAddRecordingsToCrate, useAddRecordingsToNewCrate } from "./useCrates";

/**
 * What the last "add the selected tracks to a crate" did (1cB-13), for the
 * line over the track table. Whatever added them (the sidebar's drop, the C
 * key's dialog) reports here, and the table that is on screen says it.
 */
export type AddReport =
  | ({ kind: "added"; crate: string } & Pick<
      AddedToCrate,
      "added" | "alsoAddedToLibrary" | "alreadyThere" | "leftOut"
    >)
  /** Dropped on a folder, which holds crates, not tracks. */
  | { kind: "folder" }
  | { kind: "failed"; message: string };

export const useAddReport = create<{
  report: AddReport | null;
  show: (report: AddReport) => void;
  clear: () => void;
}>((set) => ({
  report: null,
  show: (report) => set({ report }),
  clear: () => set((state) => (state.report === null ? state : { report: null })),
}));

/**
 * Adds tracks (`TableRow.recordingId`) to a crate as one operation, and puts
 * what happened in {@link useAddReport}. Resolves to whether it worked.
 */
export function useAddSelectionToCrate() {
  const add = useAddRecordingsToCrate();
  return async (crate: { id: number; name: string }, recordings: readonly number[]): Promise<boolean> => {
    try {
      const done = await add.mutateAsync({ id: crate.id, recordings });
      showAdded(crate.name, done);
      return true;
    } catch (error) {
      useAddReport.getState().show({ kind: "failed", message: errorMessage(error) });
      return false;
    }
  };
}

function showAdded(crate: string, done: AddedToCrate) {
  useAddReport.getState().show({
    kind: "added",
    crate,
    added: done.added,
    alsoAddedToLibrary: done.alsoAddedToLibrary,
    alreadyThere: done.alreadyThere,
    leftOut: done.leftOut,
  });
}

/**
 * Makes a crate at the top from `name` and adds the tracks to it as ONE
 * operation (one Undo), and puts what happened in {@link useAddReport}.
 * Throws when it is refused (the name, a track that is gone): nothing was
 * made, and the caller shows the reason where the name was typed.
 */
export function useAddSelectionToNewCrate() {
  const add = useAddRecordingsToNewCrate();
  return async (name: string, recordings: readonly number[]): Promise<void> => {
    const done = await add.mutateAsync({ name, recordings });
    showAdded(done.name, done.added);
  };
}
