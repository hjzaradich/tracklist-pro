import { useCallback, useEffect } from "react";
import { create } from "zustand";
import type { LibraryTrackId, TableRow } from "../bindings";

/**
 * The track the Details panel shows (ROADMAP 1.1, Layout): the selected row
 * of the track table on screen. A table reports its selection here
 * ({@link useReportSelection}); the panel reads it. `ids` are the tracks
 * selected (`TableRow.recordingId`): the focused row alone, or all of a bigger
 * selection, which the Energy buttons act on (1cD, {@link useReportSelectedIds}).
 * `tables` is how many tables are on screen, so the shell draws the panel only
 * beside one. `libraryIdsOf` is the table's way from selected tracks to their
 * Library tracks, for what is asked of Library tracks (the look-up); it
 * answers only for rows the table has loaded, and a track outside the Library
 * has none.
 */
export const useSelectedTrack = create<{
  row: TableRow | null;
  ids: readonly number[];
  tables: number;
  libraryIdsOf: ((recordingIds: readonly number[]) => LibraryTrackId[]) | null;
  setLibraryIdsOf: (resolve: ((recordingIds: readonly number[]) => LibraryTrackId[]) | null) => void;
  select: (row: TableRow | null) => void;
  selectIds: (ids: readonly number[]) => void;
  tableMounted: () => void;
  tableUnmounted: () => void;
}>((set) => ({
  row: null,
  ids: [],
  tables: 0,
  libraryIdsOf: null,
  setLibraryIdsOf: (libraryIdsOf) => set({ libraryIdsOf }),
  select: (row) => set({ row }),
  selectIds: (ids) => set({ ids }),
  tableMounted: () => set((state) => ({ tables: state.tables + 1 })),
  tableUnmounted: () => set((state) => ({ tables: Math.max(0, state.tables - 1), row: null, ids: [], libraryIdsOf: null })),
}));

/** Whether a track table is on screen, so the Details panel has a track to show. */
export function useTableOnScreen(): boolean {
  return useSelectedTrack((state) => state.tables > 0);
}

/**
 * For a track table: reports the selected row to the Details panel, and
 * calls `onSelectionChange` as well. The table's selection goes when it does.
 */
export function useReportSelection(onSelectionChange?: (row: TableRow | null) => void) {
  const select = useSelectedTrack((state) => state.select);
  useEffect(() => {
    const { tableMounted, tableUnmounted } = useSelectedTrack.getState();
    tableMounted();
    return tableUnmounted;
  }, []);
  return useCallback(
    (row: TableRow | null) => {
      select(row);
      onSelectionChange?.(row);
    },
    [select, onSelectionChange],
  );
}

/** For a track table: reports the ids of the selected tracks to the Details panel. */
export function useReportSelectedIds(): (ids: readonly number[]) => void {
  return useSelectedTrack((state) => state.selectIds);
}
