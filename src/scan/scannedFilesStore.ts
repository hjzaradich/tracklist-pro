import { create } from "zustand";
import type { ScannedFile } from "../bindings";

type ScannedFilesState = {
  /**
   * Every file the walk has added since the app started, by row id.
   * Changed in place, so a 100k-file first scan costs each batch only its
   * own files. So a component that selects only `files` never re-renders:
   * read it through {@link useScannedFiles}, which selects `version`.
   */
  files: Map<number, ScannedFile>;
  /** How many files are in `files`. */
  count: number;
  /** Goes up by one with every batch that changed `files`. */
  version: number;
  add: (batch: ScannedFile[]) => void;
  reset: () => void;
};

/**
 * Files the walk (stage 1 of the scan) has just added to the index, as they
 * stream in. Holds them until All music can show tracks (1aC-2). A file
 * sent twice is kept once.
 */
export const useScannedFilesStore = create<ScannedFilesState>()((set) => ({
  files: new Map(),
  count: 0,
  version: 0,
  add: (batch) =>
    set((state) => {
      if (batch.length === 0) return state;
      for (const file of batch) state.files.set(file.id, file);
      return { count: state.files.size, version: state.version + 1 };
    }),
  reset: () => set({ files: new Map(), count: 0, version: 0 }),
}));

/**
 * The scanned files, for a component: re-renders after every batch that
 * changed them. Selecting `files` alone wouldn't, because the Map is
 * changed in place.
 */
export function useScannedFiles(): ReadonlyMap<number, ScannedFile> {
  useScannedFilesStore((state) => state.version);
  return useScannedFilesStore.getState().files;
}
